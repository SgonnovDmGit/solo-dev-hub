// F-000041: local git CLI shellout layer. First module in the project to
// depend on a `git` binary on disk (everything before went through octokit
// or pure filesystem ops). Functions here are sync — every git op finishes
// in well under 500ms on realistic repos, and the Tauri command boundary
// doesn't benefit from async at that latency.

use crate::models::UntrackReport;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolved location of the git binary. Wrapped in a newtype so callers can't
/// accidentally pass an unrelated `PathBuf` (e.g. a repo path) where a binary
/// is expected.
#[derive(Debug, Clone)]
pub struct GitBinary(pub PathBuf);

// B-000014: on Windows release builds a bare `Command::new("git")` flashes a
// console window every time the subprocess starts. CREATE_NO_WINDOW suppresses
// it. Every production callsite below goes through `spawn_cmd` so the flag is
// applied uniformly; tests can keep using `Command::new` directly.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn spawn_cmd(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// PATH-first probe; on Windows, fall back to the two standard Git-for-Windows
/// install locations before giving up. Returns `None` if no usable binary is
/// found — callers must handle this (UI hides the Untrack button).
pub fn check_git_available() -> Option<GitBinary> {
    if let Ok(out) = spawn_cmd("git").arg("--version").output() {
        if out.status.success() {
            return Some(GitBinary(PathBuf::from("git")));
        }
    }

    #[cfg(windows)]
    {
        for candidate in [
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files (x86)\Git\cmd\git.exe",
        ] {
            let path = PathBuf::from(candidate);
            if path.exists() {
                if let Ok(out) = spawn_cmd(&path).arg("--version").output() {
                    if out.status.success() {
                        return Some(GitBinary(path));
                    }
                }
            }
        }
    }

    None
}

/// True if `local_path` looks like a git work tree (has a `.git` entry).
/// `.git` may be a directory (normal repo) or a file (worktree, submodule);
/// `Path::exists` accepts both.
pub fn is_git_repo(local_path: &Path) -> bool {
    local_path.join(".git").exists()
}

/// Three-state classification of the repo's working state. Untrack is only
/// safe in `Clean` — `git rm --cached` during a merge or rebase would corrupt
/// the in-flight operation's metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoState {
    Clean,
    MidMerge,
    MidRebase,
}

/// Detect mid-merge / mid-rebase via well-known marker files inside `.git/`.
/// Order matters only for reporting; the markers are mutually exclusive in
/// practice. Falls through to `Clean` if neither is present.
pub fn detect_repo_state(local_path: &Path) -> RepoState {
    let git_dir = local_path.join(".git");
    if git_dir.join("MERGE_HEAD").exists() {
        return RepoState::MidMerge;
    }
    if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        return RepoState::MidRebase;
    }
    RepoState::Clean
}

/// Current checked-out branch name, or None if detached HEAD (or unborn).
// T-000141 foundation: consumed by the v1.7.0 auto-commit wiring (T-000137+).
#[allow(dead_code)]
pub fn current_branch(git: &GitBinary, local_path: &Path) -> Result<Option<String>, String> {
    let out = spawn_cmd(&git.0)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git rev-parse failed to start: {}", e))?;
    if !out.status.success() {
        // Unborn branch (no commits yet) — treat as no resolvable branch.
        return Ok(None);
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() || name == "HEAD" {
        Ok(None) // detached HEAD
    } else {
        Ok(Some(name))
    }
}

/// Outcome of a scoped (pathspec) commit attempt.
// T-000141 foundation: consumed by the v1.7.0 auto-commit wiring (T-000137+).
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed {
        files: usize,
    },
    NothingToCommit,
    WrongBranch {
        current: Option<String>,
        expected: String,
    },
}

/// Commit ONLY `paths` (adds/mods/deletes) on top of HEAD using a pathspec
/// commit, so any other staged or unstaged work in the tree is left untouched.
///
/// Guard: commits only if the current branch == `expected_branch`; otherwise
/// returns `WrongBranch` without modifying the repo (never checks out a branch).
///
/// Identity (author + committer) is set via `-c user.name=/-c user.email=`,
/// which does NOT mutate the repo's git config.
///
/// `paths` are repo-relative. New/modified/deleted are all handled: we
/// `git add -A -- <paths>` first (so brand-new untracked synced files are staged),
/// then `git commit -- <paths>` (pathspec scopes the commit to just these files).
// T-000141 foundation: consumed by the v1.7.0 auto-commit wiring (T-000137+).
#[allow(dead_code, clippy::too_many_arguments)]
pub fn commit_paths(
    git: &GitBinary,
    local_path: &Path,
    expected_branch: &str,
    paths: &[PathBuf],
    subject: &str,
    body: Option<&str>,
    author_name: &str,
    author_email: &str,
) -> Result<CommitOutcome, String> {
    if paths.is_empty() {
        return Ok(CommitOutcome::NothingToCommit);
    }
    // Branch guard.
    let current = current_branch(git, local_path)?;
    if current.as_deref() != Some(expected_branch) {
        return Ok(CommitOutcome::WrongBranch {
            current,
            expected: expected_branch.to_string(),
        });
    }
    // Stage our paths (adds/mods/deletes), scoped by pathspec.
    let add = spawn_cmd(&git.0)
        .arg("add")
        .arg("-A")
        .arg("--")
        .args(paths)
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git add failed to start: {}", e))?;
    if !add.status.success() {
        return Err(format!(
            "git add exit {}: {}",
            add.status,
            String::from_utf8_lossy(&add.stderr)
        ));
    }
    // Anything actually staged for these paths vs HEAD?
    let staged = spawn_cmd(&git.0)
        .args(["diff", "--cached", "--name-only", "-z", "--"])
        .args(paths)
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git diff --cached failed to start: {}", e))?;
    if !staged.status.success() {
        return Err(format!(
            "git diff --cached exit {}: {}",
            staged.status,
            String::from_utf8_lossy(&staged.stderr)
        ));
    }
    let staged_names: Vec<&[u8]> = staged
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    if staged_names.is_empty() {
        return Ok(CommitOutcome::NothingToCommit);
    }
    // Pathspec commit — scopes the commit to just these paths, leaving other
    // staged/unstaged work in the index untouched.
    let mut cmd = spawn_cmd(&git.0);
    cmd.arg("-c")
        .arg(format!("user.name={}", author_name))
        .arg("-c")
        .arg(format!("user.email={}", author_email))
        .arg("commit")
        .arg("-m")
        .arg(subject);
    if let Some(b) = body {
        cmd.arg("-m").arg(b);
    }
    cmd.arg("--").args(paths).current_dir(local_path);
    let out = cmd
        .output()
        .map_err(|e| format!("git commit failed to start: {}", e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Defensive: race where nothing remained to commit.
        if stderr.contains("nothing to commit") || stderr.contains("no changes added") {
            return Ok(CommitOutcome::NothingToCommit);
        }
        return Err(format!("git commit exit {}: {}", out.status, stderr));
    }
    Ok(CommitOutcome::Committed {
        files: staged_names.len(),
    })
}

/// List tracked files that match `.gitignore` rules. Uses `-z` so the parser
/// is whitespace-safe (filenames with spaces, newlines, quotes all survive).
/// Git writes UTF-8 to stdout regardless of platform.
pub fn list_gitignored_tracked(git: &GitBinary, local_path: &Path) -> Result<Vec<PathBuf>, String> {
    let out = spawn_cmd(&git.0)
        .args(["ls-files", "-ci", "--exclude-standard", "-z"])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git ls-files failed to start: {}", e))?;

    if !out.status.success() {
        return Err(format!(
            "git ls-files exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    parse_null_terminated_paths(&out.stdout)
}

/// Untrack `files` in batches of 100 (`git rm --cached <a> <b> ...`).
/// Per-chunk errors are captured in the report; we keep going so a single bad
/// chunk doesn't abort a 250-file run.
pub fn untrack_files(
    git: &GitBinary,
    local_path: &Path,
    files: &[PathBuf],
) -> Result<UntrackReport, String> {
    let mut report = UntrackReport {
        untracked: 0,
        errors: Vec::new(),
    };

    for chunk in chunk_files(files, 100) {
        let out = spawn_cmd(&git.0)
            .arg("rm")
            .arg("--cached")
            .args(&chunk)
            .current_dir(local_path)
            .output()
            .map_err(|e| format!("git rm --cached failed to start: {}", e))?;

        if out.status.success() {
            report.untracked += chunk.len();
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            report.errors.push(stderr.trim().to_string());
        }
    }

    Ok(report)
}

/// Count entries currently in the index that aren't in `exclude`. Used by the
/// UI to warn the user "N other staged changes detected" before they confirm.
pub fn count_other_staged_changes(
    git: &GitBinary,
    local_path: &Path,
    exclude: &[PathBuf],
) -> Result<usize, String> {
    let out = spawn_cmd(&git.0)
        .args(["diff", "--cached", "--name-only", "-z"])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git diff failed to start: {}", e))?;

    if !out.status.success() {
        return Err(format!(
            "git diff exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let staged = parse_null_terminated_paths(&out.stdout)?;
    let exclude_set: HashSet<&Path> = exclude.iter().map(|p| p.as_path()).collect();
    Ok(staged
        .iter()
        .filter(|p| !exclude_set.contains(p.as_path()))
        .count())
}

/// Split `files` into batches of `chunk_size`. Defensive against `chunk_size == 0`
/// (returns empty Vec rather than looping forever). Pure, no I/O — covered by a
/// unit test independent of any git subprocess.
pub fn chunk_files(files: &[PathBuf], chunk_size: usize) -> Vec<Vec<PathBuf>> {
    if chunk_size == 0 {
        return Vec::new();
    }
    files.chunks(chunk_size).map(|c| c.to_vec()).collect()
}

/// Decode `git ... -z` output: UTF-8 bytes split by NUL with a trailing NUL
/// after the last entry. We accept the trailing empty segment by stripping it.
fn parse_null_terminated_paths(bytes: &[u8]) -> Result<Vec<PathBuf>, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| format!("git stdout is not valid UTF-8: {}", e))?;
    Ok(text
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// Init a git repo in `dir` with a sensible identity so commits work
    /// without depending on the host's global git config.
    fn init_test_repo(dir: &Path) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
        // Avoid "Refusing to commit on detached HEAD" / branch-name flakiness
        // across git versions by pinning to master.
        run_git(dir, &["symbolic-ref", "HEAD", "refs/heads/master"]);
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git invocation failed");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn test_check_git_available_returns_some_when_git_in_path() {
        // Test machines (CI included) have git on PATH.
        let g = check_git_available();
        assert!(g.is_some(), "expected git on PATH for tests");
    }

    #[test]
    fn test_is_git_repo_true_for_initialized_repo() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        assert!(is_git_repo(tmp.path()));
    }

    #[test]
    fn test_is_git_repo_false_for_plain_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_git_repo(tmp.path()));
    }

    #[test]
    fn test_detect_repo_state_clean_after_init() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        assert_eq!(detect_repo_state(tmp.path()), RepoState::Clean);
    }

    #[test]
    fn test_detect_repo_state_mid_merge_marker() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(tmp.path().join(".git/MERGE_HEAD"), "deadbeef\n").unwrap();
        assert_eq!(detect_repo_state(tmp.path()), RepoState::MidMerge);
    }

    #[test]
    fn test_detect_repo_state_mid_rebase_marker() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::create_dir(tmp.path().join(".git/rebase-merge")).unwrap();
        assert_eq!(detect_repo_state(tmp.path()), RepoState::MidRebase);
    }

    #[test]
    fn test_list_gitignored_tracked_returns_paths() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        // Commit .env before adding the ignore rule.
        fs::write(tmp.path().join(".env"), "SECRET=x\n").unwrap();
        run_git(tmp.path(), &["add", ".env"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add env"]);
        fs::write(tmp.path().join(".gitignore"), ".env\n").unwrap();
        run_git(tmp.path(), &["add", ".gitignore"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add gitignore"]);

        let git = check_git_available().expect("git available");
        let result = list_gitignored_tracked(&git, tmp.path()).unwrap();
        assert_eq!(result, vec![PathBuf::from(".env")]);
    }

    #[test]
    fn test_list_gitignored_tracked_empty_when_no_match() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(tmp.path().join("README.md"), "# r\n").unwrap();
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);

        let git = check_git_available().expect("git available");
        let result = list_gitignored_tracked(&git, tmp.path()).unwrap();
        assert!(result.is_empty(), "expected empty, got {:?}", result);
    }

    #[test]
    fn test_untrack_files_removes_from_index_keeps_worktree() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(tmp.path().join(".env"), "SECRET=x\n").unwrap();
        run_git(tmp.path(), &["add", ".env"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add env"]);
        fs::write(tmp.path().join(".gitignore"), ".env\n").unwrap();
        run_git(tmp.path(), &["add", ".gitignore"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add gi"]);

        let git = check_git_available().expect("git available");
        let report = untrack_files(&git, tmp.path(), &[PathBuf::from(".env")]).unwrap();
        assert_eq!(report.untracked, 1);
        assert!(report.errors.is_empty(), "errors: {:?}", report.errors);

        // Index should no longer list .env, but file still on disk.
        let ls = Command::new("git")
            .args(["ls-files"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&ls.stdout).to_string();
        assert!(!listing.contains(".env\n") || listing.contains(".gitignore"));
        let env_listed = listing.lines().any(|l| l == ".env");
        assert!(!env_listed, "git ls-files still contains .env: {}", listing);
        assert!(
            tmp.path().join(".env").exists(),
            "working tree file must remain"
        );
    }

    #[test]
    fn test_chunk_files_splits_into_batches() {
        let files: Vec<PathBuf> = (0..250).map(|i| PathBuf::from(format!("f{}", i))).collect();
        let chunks = chunk_files(&files, 100);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 100);
        assert_eq!(chunks[1].len(), 100);
        assert_eq!(chunks[2].len(), 50);
    }

    #[test]
    fn test_chunk_files_zero_chunk_size_returns_empty() {
        let files: Vec<PathBuf> = vec![PathBuf::from("a"), PathBuf::from("b")];
        let chunks = chunk_files(&files, 0);
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_count_other_staged_changes() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        // Initial commit so the index has a base.
        fs::write(tmp.path().join("README.md"), "# r\n").unwrap();
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);

        // Stage 3 new files; we'll mark one as "the one we're about to untrack".
        fs::write(tmp.path().join("a.txt"), "a").unwrap();
        fs::write(tmp.path().join("b.txt"), "b").unwrap();
        fs::write(tmp.path().join("c.txt"), "c").unwrap();
        run_git(tmp.path(), &["add", "a.txt", "b.txt", "c.txt"]);

        let git = check_git_available().expect("git available");
        let n = count_other_staged_changes(&git, tmp.path(), &[PathBuf::from("a.txt")]).unwrap();
        assert_eq!(n, 2, "expected b.txt + c.txt = 2 other staged");
    }

    // ── T-000141: current_branch / commit_paths ───────────────────────────────

    /// Count commits reachable from HEAD (0 if unborn).
    fn commit_count(dir: &Path) -> usize {
        let out = Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        if !out.status.success() {
            return 0; // unborn branch
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }

    /// Files in HEAD's tree commit (`git log -1 --name-only`, filtered to paths).
    fn head_files(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["log", "-1", "--name-only", "--format="])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect()
    }

    fn staged_files(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect()
    }

    #[test]
    fn test_current_branch_returns_master() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        // `git rev-parse --abbrev-ref HEAD` only resolves once the branch has a
        // commit (an unborn branch reports None) — seed one first.
        fs::write(tmp.path().join("README.md"), "# r\n").unwrap();
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);
        let git = check_git_available().expect("git available");
        assert_eq!(
            current_branch(&git, tmp.path()).unwrap(),
            Some("master".to_string())
        );
    }

    #[test]
    fn test_commit_paths_new_file_committed() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        // Baseline commit so the branch is born (commit_paths' branch guard needs
        // a resolvable branch); synced.md is a brand-new file added on top.
        fs::write(tmp.path().join("README.md"), "# r\n").unwrap();
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);
        fs::write(tmp.path().join("synced.md"), "# hi\n").unwrap();

        let git = check_git_available().expect("git available");
        let outcome = commit_paths(
            &git,
            tmp.path(),
            "master",
            &[PathBuf::from("synced.md")],
            "sync: add synced.md",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });

        // File is in HEAD.
        assert!(
            head_files(tmp.path()).contains(&"synced.md".to_string()),
            "synced.md must be in HEAD tree"
        );

        // Author identity applied without touching repo config.
        let an = Command::new("git")
            .args(["log", "-1", "--format=%an"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&an.stdout).trim(), "Solo Dev Hub");
    }

    #[test]
    fn test_commit_paths_wrong_branch_no_commit() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        // Seed a commit so we have a non-zero baseline count.
        fs::write(tmp.path().join("README.md"), "# r\n").unwrap();
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);
        let before = commit_count(tmp.path());

        fs::write(tmp.path().join("synced.md"), "# hi\n").unwrap();
        let git = check_git_available().expect("git available");
        let outcome = commit_paths(
            &git,
            tmp.path(),
            "dev", // repo is on master
            &[PathBuf::from("synced.md")],
            "sync: add synced.md",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(
            outcome,
            CommitOutcome::WrongBranch {
                current: Some("master".to_string()),
                expected: "dev".to_string(),
            }
        );
        assert_eq!(commit_count(tmp.path()), before, "no commit must be made");
    }

    #[test]
    fn test_commit_paths_preserves_other_staged_work() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(tmp.path().join("README.md"), "# r\n").unwrap();
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);

        // Unrelated staged work.
        fs::write(tmp.path().join("wip.txt"), "work in progress\n").unwrap();
        run_git(tmp.path(), &["add", "wip.txt"]);

        // Our synced file.
        fs::write(tmp.path().join("synced.md"), "# hi\n").unwrap();

        let git = check_git_available().expect("git available");
        let outcome = commit_paths(
            &git,
            tmp.path(),
            "master",
            &[PathBuf::from("synced.md")],
            "sync: add synced.md",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });

        // synced.md is in HEAD, wip.txt is NOT committed.
        let hf = head_files(tmp.path());
        assert!(hf.contains(&"synced.md".to_string()));
        assert!(
            !hf.contains(&"wip.txt".to_string()),
            "wip.txt must not be in the commit"
        );

        // wip.txt still staged.
        assert!(
            staged_files(tmp.path()).contains(&"wip.txt".to_string()),
            "wip.txt must remain staged after scoped commit"
        );
    }

    #[test]
    fn test_commit_paths_handles_deletion() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(tmp.path().join("gone.md"), "bye\n").unwrap();
        run_git(tmp.path(), &["add", "gone.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add gone"]);

        // Delete from disk, then commit that path.
        fs::remove_file(tmp.path().join("gone.md")).unwrap();
        let git = check_git_available().expect("git available");
        let outcome = commit_paths(
            &git,
            tmp.path(),
            "master",
            &[PathBuf::from("gone.md")],
            "sync: remove gone.md",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });

        // Deletion recorded: file no longer tracked in HEAD tree.
        let ls = Command::new("git")
            .args(["ls-tree", "-r", "--name-only", "HEAD"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let tree = String::from_utf8_lossy(&ls.stdout).to_string();
        assert!(
            !tree.lines().any(|l| l == "gone.md"),
            "gone.md must be removed from the tree: {}",
            tree
        );
    }

    #[test]
    fn test_commit_paths_nothing_to_commit() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(tmp.path().join("stable.md"), "unchanged\n").unwrap();
        run_git(tmp.path(), &["add", "stable.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add stable"]);

        // No changes to stable.md since HEAD.
        let git = check_git_available().expect("git available");
        let outcome = commit_paths(
            &git,
            tmp.path(),
            "master",
            &[PathBuf::from("stable.md")],
            "sync: noop",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::NothingToCommit);
    }
}
