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
}
