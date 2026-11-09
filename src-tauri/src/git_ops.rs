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

pub(crate) fn spawn_cmd(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// The git binaries to probe, in order, for an OS (`std::env::consts::OS`).
/// `"git"` is the one on PATH.
///
/// - Windows: PATH first, then the two standard Git-for-Windows locations.
/// - macOS (T-000125): Homebrew first. An app opened from Finder gets a PATH
///   without Homebrew, and its `/usr/bin/git` is the Command Line Tools shim,
///   which on a Mac without them opens the "install developer tools" dialog on
///   every probe. Apple Silicon Homebrew, then Intel / older Homebrew, then PATH.
/// - Elsewhere: PATH only.
fn git_candidates(os: &str) -> &'static [&'static str] {
    match os {
        "windows" => &[
            "git",
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files (x86)\Git\cmd\git.exe",
        ],
        "macos" => &["/opt/homebrew/bin/git", "/usr/local/bin/git", "git"],
        _ => &["git"],
    }
}

/// Probes `git_candidates` for this OS; an absolute candidate is tried only if
/// the file exists. Returns `None` if no usable binary is found — callers must
/// handle this (UI hides the Untrack button).
pub fn check_git_available() -> Option<GitBinary> {
    git_candidates(std::env::consts::OS)
        .iter()
        .map(PathBuf::from)
        .filter(|path| !path.is_absolute() || path.exists())
        .find(|path| {
            spawn_cmd(path)
                .arg("--version")
                .output()
                .is_ok_and(|out| out.status.success())
        })
        .map(GitBinary)
}

/// `check_git_available`, probed once per process: the first successful probe
/// is kept, so per-call paths (RepoDetail asks on every repo open) spawn no
/// `git --version`. A failed probe is not kept — installing git later works
/// without restarting the app.
pub fn cached_git() -> Option<GitBinary> {
    static GIT: std::sync::OnceLock<GitBinary> = std::sync::OnceLock::new();
    if let Some(git) = GIT.get() {
        return Some(git.clone());
    }
    let git = check_git_available()?;
    Some(GIT.get_or_init(|| git).clone())
}

/// True if `local_path` looks like a git work tree (has a `.git` entry).
/// `.git` may be a directory (normal repo) or a file (worktree, submodule);
/// `Path::exists` accepts both.
pub fn is_git_repo(local_path: &Path) -> bool {
    local_path.join(".git").exists()
}

// ── F-000045: exact GitHub remote matcher ─────────────────────────────────────

/// F-000045: split a git remote URL into `(host, path)` — HTTPS/SSH with a
/// scheme (`https://github.com/o/r.git`) and the scp-like SSH form without one
/// (`git@github.com:o/r.git`). Userinfo (`user:token@`) is allowed and ignored;
/// query/fragment never reach the path. `None` when `url` is not a URL-shaped
/// remote at all (a comment line, prose).
fn github_remote_parts(url: &str) -> Option<(&str, &str)> {
    let url = url.trim();
    let host_is_github = |authority: &str| -> bool {
        // Strip `user[:pass]@` userinfo; the host itself must be exactly
        // github.com — a look-alike host (`github.com.evil`) proves nothing.
        authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host)
            .eq_ignore_ascii_case("github.com")
    };
    if let Some(idx) = url.find("://") {
        // A scheme is `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`; anything
        // else before `://` (a comment, prose) is not a URL.
        let scheme = &url[..idx];
        let scheme_ok = !scheme.is_empty()
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.');
        if !scheme_ok {
            return None;
        }
        let rest = &url[idx + 3..];
        let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        if !host_is_github(&rest[..auth_end]) {
            return None;
        }
        let path = rest[auth_end..].split(['?', '#']).next().unwrap_or("");
        Some(("github.com", path.strip_prefix('/')?))
    } else {
        // scp-like: `[user@]host:path`. Anything after the host is the path.
        let idx = url.find(':')?;
        if !host_is_github(&url[..idx]) {
            return None;
        }
        let path = &url[idx + 1..];
        let path = path.split(['?', '#']).next().unwrap_or("");
        Some(("github.com", path.strip_prefix('/').unwrap_or(path)))
    }
}

/// F-000045: true iff `url` is a GitHub remote of exactly `github_name`
/// (`owner/repo`). Host and full owner/repo path compare case-insensitively
/// (GitHub ignores case in both); a `.git` suffix and a trailing `/` are
/// optional. HTTPS and both SSH spellings match:
/// `https://github.com/owner/repo[.git]`, `ssh://git@github.com/owner/repo.git`
/// and `git@github.com:owner/repo.git`. A different host, a prefix/suffix repo
/// (`owner/repo-other`), a nested path, userinfo alone or a `pushurl` value do
/// NOT prove a match. Replaces the workspace scanner's substring matching,
/// where `owner/repo` also matched `owner/repo-other` and any commented URL.
pub fn remote_matches_github(url: &str, github_name: &str) -> bool {
    let name = github_name.trim();
    // Exactly `owner/repo`: one separator, both halves non-empty. A name with
    // more segments or with URL noise can never name a GitHub repository.
    let (owner, repo) = match name.split_once('/') {
        Some((o, r)) if !o.is_empty() && !r.is_empty() && !r.contains('/') => (o, r),
        _ => return false,
    };
    let Some((_, path)) = github_remote_parts(url) else {
        return false;
    };
    let mut path = path.trim_end_matches('/').to_ascii_lowercase();
    if let Some(stripped) = path.strip_suffix(".git") {
        path = stripped.to_string();
    }
    path == format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    )
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

/// The full ref `HEAD` points at (`refs/heads/<name>`), or `None` when HEAD
/// is detached.
///
/// Review fix (2nd pass of T-000244): only exit 1 of `symbolic-ref -q HEAD`
/// MEANS detached. Git also exits 128 for a HEAD that cannot be resolved at
/// all — a cyclic symbolic ref dies with `fatal: No such ref: HEAD` while
/// `repo_state` still succeeds — and folding every nonzero exit into `None`
/// made `commit_state_safe` read such a checkout as a detached one and defer
/// it silently instead of letting the caller report the failure.
fn head_symbolic_ref(git: &GitBinary, local_path: &Path) -> Result<Option<String>, String> {
    let out = spawn_cmd(&git.0)
        .args(["symbolic-ref", "-q", "HEAD"])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git symbolic-ref failed to start: {}", e))?;
    match out.status.code() {
        Some(0) => {}
        // Exit 1 with -q: HEAD is detached.
        Some(1) => return Ok(None),
        _ => {
            return Err(format!(
                "git symbolic-ref exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ))
        }
    }
    Ok(Some(
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

/// Current checked-out branch name, or None if detached HEAD. An unborn branch
/// (no commits yet) still has a name. Read from the symbolic ref itself:
/// `rev-parse --abbrev-ref HEAD` prints `heads/<name>` when a tag of the same
/// name exists.
pub fn current_branch(git: &GitBinary, local_path: &Path) -> Result<Option<String>, String> {
    let Some(full) = head_symbolic_ref(git, local_path)? else {
        return Ok(None);
    };
    Ok(full.strip_prefix("refs/heads/").map(|s| s.to_string()))
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

/// True if `git add -- <path>` would refuse `rel_path` (repo-relative) as
/// gitignored. Auto-commit uses this to skip cross-repo folders the dev
/// deliberately gitignored (their local "inbox"): `git add` refuses an
/// explicitly-named ignored path with exit 1, which would otherwise abort the
/// whole scoped commit for every folder in the batch.
///
/// For a directory that holds tracked files plain `check-ignore` says "not
/// ignored" (the index wins), yet `git add -- <dir>` still refuses it. So a
/// directory is checked with `--no-index`; a single file keeps index
/// semantics (`git add` accepts an explicitly named tracked file even when a
/// rule matches it).
pub fn is_ignored_for_add(
    git: &GitBinary,
    local_path: &Path,
    rel_path: &Path,
) -> Result<bool, String> {
    let no_index = local_path.join(rel_path).is_dir();
    check_ignore(git, local_path, rel_path, no_index)
}

/// `check-ignore` for one path; `no_index` skips the index, so a tracked file
/// is judged by its ignore rules alone. T-000244: the managed-block regression
/// tests of `sync::gitignore` use it against the real bundled template.
pub(crate) fn check_ignore(
    git: &GitBinary,
    local_path: &Path,
    rel_path: &Path,
    no_index: bool,
) -> Result<bool, String> {
    let mut cmd = spawn_cmd(&git.0);
    cmd.arg("check-ignore").arg("-q");
    if no_index {
        cmd.arg("--no-index");
    }
    let out = cmd
        .arg("--")
        .arg(rel_path)
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git check-ignore failed to start: {}", e))?;
    // check-ignore exit codes: 0 = path is ignored, 1 = not ignored,
    // 128 (or any other) = fatal error.
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(format!(
            "git check-ignore exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )),
    }
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

/// T-000244: pathspecs matching every rule hand-back marker under
/// `docs/sdh_skills/` — `<name>.md.approved` (the agent's approval marker) and
/// `<name>.md.conflict` (the app's reason file). `*` also covers the `.md`
/// before the suffix. Local service files: they never belong in a commit.
pub const MARKER_PATHSPECS: &[&str] = &["docs/sdh_skills/*.approved", "docs/sdh_skills/*.conflict"];

/// Tracked files matching `pathspecs` (`git ls-files -z -- …`), read from the
/// INDEX rather than from the work tree: a pass may have already deleted the
/// worktree file while the index entry that keeps it in the branch is still
/// there. NUL-separated output, paths passed through literally — no shell
/// expansion ever sees them.
pub fn list_tracked_files(
    git: &GitBinary,
    local_path: &Path,
    pathspecs: &[&str],
) -> Result<Vec<PathBuf>, String> {
    if pathspecs.is_empty() {
        return Ok(Vec::new());
    }
    let out = spawn_cmd(&git.0)
        .args(["ls-files", "-z", "--"])
        .args(pathspecs)
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

/// T-000244: does `rel_path` name a rule hand-back marker? Pathspec globbing is
/// not used to decide this — a plain suffix check on a `docs/sdh_skills/` path.
pub fn is_rule_marker(rel_path: &Path) -> bool {
    rel_path.starts_with("docs/sdh_skills/")
        && rel_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".approved") || n.ends_with(".conflict"))
}

/// The rule hand-back markers `rev` (e.g. `HEAD`) still carries in its tree
/// (`git ls-tree -r --name-only`). A deletion this app has to deliver is read
/// from the branch's history, never from the worktree or the current index:
/// after a restart or a failed commit the marker is still there even though the
/// folder on disk may be gone.
pub fn list_markers_in_rev(
    git: &GitBinary,
    local_path: &Path,
    rev: &str,
) -> Result<Vec<PathBuf>, String> {
    let out = spawn_cmd(&git.0)
        .args([
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            rev,
            "--",
            "docs/sdh_skills/",
        ])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git ls-tree failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git ls-tree exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(parse_null_terminated_paths(&out.stdout)?
        .into_iter()
        .filter(|p| is_rule_marker(p))
        .collect())
}

/// Is `path` (a file or a dir prefix) still present in the git INDEX? An entry
/// surviving in the index while its worktree file is gone is a pending
/// deletion a scoped `git add -A` can stage — and the pathspec is then
/// guaranteed to match, unlike a HEAD check which would also hit
/// already-staged deletions and make `git add` fail on an unmatched spec
/// (T-000190: legacy channel dirs deleted by a migration's fs moves).
pub fn path_in_index(git: &GitBinary, local_path: &Path, path: &Path) -> Result<bool, String> {
    let out = spawn_cmd(&git.0)
        .args(["ls-files", "--"])
        .arg(path)
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
    Ok(!out.stdout.is_empty())
}

/// A throw-away index file, removed together with its lock on every exit path.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        // `git read-tree` / `git add` may leave a stale lock behind if killed.
        let mut lock = self.0.as_os_str().to_owned();
        lock.push(".lock");
        let _ = std::fs::remove_file(PathBuf::from(lock));
    }
}

static INDEX_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Path of a throw-away index inside the repo's git dir. Resolved with
/// `--git-path`, so a linked worktree gets its own private directory. The name
/// lives in its own namespace (`git_sync`'s plumbing index is `sdh-index-*`), so
/// the two can never borrow each other's file.
fn temp_index_path(git: &GitBinary, local_path: &Path) -> Result<TempIndex, String> {
    let name = format!(
        "sdh-untrack-index-{}-{}",
        std::process::id(),
        INDEX_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let out = spawn_cmd(&git.0)
        .args(["rev-parse", "--git-path", name.as_str()])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git rev-parse failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git rev-parse exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let p = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    Ok(TempIndex(if p.is_absolute() {
        p
    } else {
        local_path.join(p)
    }))
}

/// True while `GIT_INDEX_FILE` of `cmd` points at the throw-away `index`.
fn with_temp_index(cmd: &mut Command, index: &TempIndex) {
    cmd.env("GIT_INDEX_FILE", &index.0);
}

/// Review fix for f0a40b2, shared by `commit_paths_index_only` and the marker
/// sweep (`sync::skills::untrack_rule_markers`): may this checkout's refs and
/// index be touched by one of our scoped commits?
///
/// `Ok(false)` is a RECOGNIZED unsafe state that must wait for a safer pass —
/// mid-merge or mid-rebase (a linked worktree included: the state is read
/// through `git_sync::repo_state`'s `--git-path`, not a plain `.git/` lookup),
/// a detached HEAD, or unmerged index entries. `Err` means the state could not
/// be read at all; proceeding then would be indistinguishable from safe, so
/// callers report it instead of skipping silently.
pub(crate) fn commit_state_safe(git: &GitBinary, local_path: &Path) -> Result<bool, String> {
    match crate::git_sync::repo_state(git, local_path)? {
        RepoState::Clean => {}
        RepoState::MidMerge | RepoState::MidRebase => return Ok(false),
    }
    if current_branch(git, local_path)?.is_none() {
        return Ok(false);
    }
    let out = spawn_cmd(&git.0)
        .args(["ls-files", "-u", "-z"])
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
    // `-z` prints nothing at all when no entry is unmerged; any byte other
    // than the NUL separators is an unmerged stage line.
    Ok(out.stdout.iter().all(|&b| b == 0))
}

/// T-000244: commit the deletion of every rule hand-back marker git still
/// tracks, plus the allowed ordinary changes of `content_paths`, on top of HEAD
/// through a throw-away index.
///
/// `commit_paths` cannot carry a marker path: it runs `git add -A` and then a
/// pathspec `git commit`, so a live worktree marker would be swept in, and the
/// marker exclusion would also exclude the deletion. Here instead:
///
/// * the throw-away index starts as HEAD's tree (`read-tree HEAD`), takes the
///   `git add -A -- content_paths` of those paths alone, and then loses every
///   marker it holds (`update-index --force-remove`) — so a marker can be
///   deleted but never added or modified, whatever `content_paths` names;
/// * the commit is made from that index without a pathspec, so it is exactly
///   the snapshot just built — never the caller's real index;
/// * hooks and signing are NOT bypassed (no `--no-verify`, no
///   `-c commit.gpgsign=false`); identity still goes through `-c`, which does
///   not touch the repo config;
/// * the real index is reconciled afterwards for the committed paths only
///   (`git reset -q HEAD -- …`): without it a file this commit added would sit
///   in `git status` as a staged deletion forever. Everyone else's staged and
///   unstaged work is left exactly as it was;
/// * no file is ever deleted from the worktree and nothing is force-added. A
///   failed commit leaves the real index — including a staged marker deletion —
///   untouched, so the next pass simply tries again.
///
/// Same branch guard as `commit_paths`: `WrongBranch` touches nothing. The
/// state guard of `commit_state_safe` applies too: mid-merge, mid-rebase
/// (linked worktrees included), a detached HEAD and unresolved index entries
/// all return `NothingToCommit` without touching HEAD, the real index or the
/// in-flight operation — the deletion waits for a safer pass.
#[allow(clippy::too_many_arguments)]
pub fn commit_paths_index_only(
    git: &GitBinary,
    local_path: &Path,
    expected_branch: &str,
    content_paths: &[PathBuf],
    subject: &str,
    body: Option<&str>,
    author_name: &str,
    author_email: &str,
) -> Result<CommitOutcome, String> {
    // Branch guard — identical to `commit_paths`, never checks anything out.
    let current = current_branch(git, local_path)?;
    if current.as_deref() != Some(expected_branch) {
        return Ok(CommitOutcome::WrongBranch {
            current,
            expected: expected_branch.to_string(),
        });
    }
    // An unborn branch has no tree to delete anything from.
    if !rev_exists(git, local_path, "HEAD")? {
        return Ok(CommitOutcome::NothingToCommit);
    }
    // Review fix (BLOCKER of f0a40b2): never build the temp index, let alone
    // commit, while a merge/rebase is in flight or index entries are
    // unresolved. `git commit` here would fold MERGE_HEAD into a two-parent
    // merge commit built from HEAD's tree alone — dropping the other side's
    // changes and clearing the operation's state — and the temp index would
    // hide the unresolved entries from every check the commit path makes.
    if !commit_state_safe(git, local_path)? {
        return Ok(CommitOutcome::NothingToCommit);
    }
    let index = temp_index_path(git, local_path)?;
    // A stale file at this path (crashed run, recycled pid) must never act as a
    // starting index or block the lock; start clean.
    let _ = std::fs::remove_file(&index.0);
    let run = |args: &[&str]| -> Result<std::process::Output, String> {
        let mut cmd = spawn_cmd(&git.0);
        with_temp_index(&mut cmd, &index);
        cmd.args(args)
            .current_dir(local_path)
            .output()
            .map_err(|e| format!("git {} failed to start: {}", args[0], e))
    };
    // Review fix (MAJOR of f0a40b2): a failed init must fail closed. `run`
    // only reports spawn errors; without this check a nonzero exit left later
    // ops running against an absent/incomplete index, where every unrelated
    // tracked file reads as a deletion.
    let init = run(&["read-tree", "HEAD"])?;
    if !init.status.success() {
        return Err(format!(
            "git read-tree exit {}: {}",
            init.status,
            String::from_utf8_lossy(&init.stderr)
        ));
    }
    if !content_paths.is_empty() {
        let mut args: Vec<&str> = vec!["add", "-A", "--"];
        args.extend(content_paths.iter().map(|p| p.to_str().unwrap_or("")));
        let out = run(&args)?;
        if !out.status.success() {
            return Err(format!(
                "git add exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    // Every marker the throw-away index holds goes, whether HEAD tracked it or
    // `add -A` just staged it: the deletion is the only thing markers may do
    // in a commit.
    let markers = {
        let mut ls: Vec<String> = vec!["ls-files".into(), "-z".into(), "--".into()];
        ls.extend(MARKER_PATHSPECS.iter().map(|s| s.to_string()));
        let args: Vec<&str> = ls.iter().map(|s| s.as_str()).collect();
        let out = run(&args)?;
        if !out.status.success() {
            return Err(format!(
                "git ls-files exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        parse_null_terminated_paths(&out.stdout)?
    };
    for marker in &markers {
        let path = marker.to_string_lossy().into_owned();
        let out = run(&["update-index", "--force-remove", "--", path.as_str()])?;
        if !out.status.success() {
            return Err(format!(
                "git update-index exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    // What does this index change against HEAD? Nothing = nothing to commit.
    let changed = {
        // Review fix (2nd pass of T-000244): the trailing `--` keeps `HEAD` a
        // revision. Without it, a worktree file literally named `HEAD` made
        // git refuse the call as an ambiguous argument (`fatal: ambiguous
        // argument 'HEAD': both revision and filename`), so the marker
        // cleanup failed on that repo shape.
        let out = run(&["diff-index", "--cached", "--name-only", "-z", "HEAD", "--"])?;
        if !out.status.success() {
            return Err(format!(
                "git diff-index exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        parse_null_terminated_paths(&out.stdout)?
    };
    if changed.is_empty() {
        return Ok(CommitOutcome::NothingToCommit);
    }
    let mut cmd = spawn_cmd(&git.0);
    with_temp_index(&mut cmd, &index);
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
    let out = cmd
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git commit failed to start: {}", e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("nothing to commit") || stderr.contains("no changes added") {
            return Ok(CommitOutcome::NothingToCommit);
        }
        return Err(format!("git commit exit {}: {}", out.status, stderr));
    }
    // The real index was never the input; bring it up to the new HEAD for the
    // committed paths alone, so no phantom staged deletion survives and no
    // foreign staged change is dropped.
    // Review fix (MAJOR of f0a40b2): `HEAD` must come BEFORE the `--`. After
    // it, `HEAD` is a pathspec, so `git reset -q -- HEAD <paths>` reset the
    // staged blob of any file or directory literally named `HEAD`.
    let mut reset: Vec<&str> = vec!["HEAD", "--"];
    reset.extend(content_paths.iter().map(|p| p.to_str().unwrap_or("")));
    reset.extend(markers.iter().map(|p| p.to_str().unwrap_or("")));
    let out = spawn_cmd(&git.0)
        .args(["reset", "-q"])
        .args(&reset)
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git reset failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git reset exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(CommitOutcome::Committed {
        files: changed.len(),
    })
}

/// Does `rev` resolve? (`rev-parse --verify --quiet`, no error text parsing.)
/// `pub(crate)`: `sync::autocommit` asks whether HEAD is born before reading
/// historical markers off it.
///
/// Review fix (2nd pass of T-000244): a nonzero exit is NOT uniformly "no".
/// `rev-parse --verify --quiet 'HEAD^{commit}'` exits 1 both for a genuinely
/// unborn HEAD (the symbolic ref's target branch has no commit yet) AND for a
/// ref that exists but points at a missing or corrupt commit object. Only the
/// former is `Ok(false)`; the latter is a damaged repo and propagates as
/// `Err`, because `autocommit_repo` would otherwise answer `NothingToCommit`
/// and silently drop the marker deletion the branch still owes. The target
/// ref is verified WITHOUT peeling — `rev-parse --verify refs/heads/x` still
/// exits 0 when x resolves to an oid whose object is gone, which is exactly
/// what distinguishes "broken" from "unborn".
pub(crate) fn rev_exists(git: &GitBinary, local_path: &Path, rev: &str) -> Result<bool, String> {
    let spec = format!("{}^{{commit}}", rev);
    let out = spawn_cmd(&git.0)
        .args(["rev-parse", "--verify", "--quiet", spec.as_str()])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git rev-parse failed to start: {}", e))?;
    if out.status.success() {
        return Ok(true);
    }
    // The unborn exception only exists for the symbolic HEAD; any other rev
    // that fails to peel keeps the plain "does not resolve" answer.
    if rev != "HEAD" {
        return Ok(false);
    }
    let Some(target) = head_symbolic_ref(git, local_path)? else {
        return Err(format!(
            "git rev-parse exit {}: detached HEAD does not resolve to a commit: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    };
    let target_out = spawn_cmd(&git.0)
        .args(["rev-parse", "--verify", "--quiet", target.as_str()])
        .current_dir(local_path)
        .output()
        .map_err(|e| format!("git rev-parse failed to start: {}", e))?;
    match target_out.status.code() {
        // The branch ref exists but HEAD still fails to peel to a commit:
        // its object is missing or corrupt.
        Some(0) => Err(format!(
            "git rev-parse exit {}: HEAD points at '{}' but it does not resolve to a commit: {}",
            out.status,
            target,
            String::from_utf8_lossy(&out.stderr)
        )),
        // The branch ref itself does not exist yet: HEAD is unborn.
        Some(1) => Ok(false),
        _ => Err(format!(
            "git rev-parse exit {}: {}",
            target_out.status,
            String::from_utf8_lossy(&target_out.stderr)
        )),
    }
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

    #[test]
    fn git_candidates_try_path_first_on_windows_and_homebrew_first_on_macos() {
        let win = git_candidates("windows");
        assert_eq!(win[0], "git");
        assert!(win.contains(&r"C:\Program Files\Git\cmd\git.exe"));

        // From Finder the PATH has no Homebrew and /usr/bin/git may be the
        // Command Line Tools shim: Homebrew is probed before it.
        assert_eq!(
            git_candidates("macos"),
            &["/opt/homebrew/bin/git", "/usr/local/bin/git", "git"]
        );

        assert_eq!(git_candidates("linux"), &["git"]);
    }

    // ── F-000045: remote_matches_github ─────────────────────────────────────

    #[test]
    fn test_remote_matches_github_https_and_ssh_forms() {
        for url in [
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo",
            "https://github.com/owner/repo/",
            "git@github.com:owner/repo.git",
            "git@github.com:owner/repo",
            "ssh://git@github.com/owner/repo.git",
            "ssh://git@github.com/owner/repo",
            // userinfo does not change what the URL points at
            "https://user:tok3n@github.com/owner/repo.git",
        ] {
            assert!(remote_matches_github(url, "owner/repo"), "{url}");
        }
    }

    #[test]
    fn test_remote_matches_github_ignores_case_on_host_and_path() {
        // T-000242 keeps working: the clone's folder case differs from the
        // GitHub name on a case-sensitive filesystem.
        assert!(remote_matches_github(
            "git@github.com:Owner/Repo_With_Case.git",
            "owner/repo_with_case"
        ));
        assert!(remote_matches_github(
            "https://GITHUB.COM/owner/repo.git",
            "Owner/Repo"
        ));
    }

    #[test]
    fn test_remote_matches_github_rejects_prefix_suffix_and_nested_paths() {
        // The old scanner's substring match accepted all of these.
        for url in [
            "https://github.com/owner/repo-other.git",
            "https://github.com/owner/repo.git.bak",
            "https://github.com/other-owner/repo.git",
            "https://github.com/x/owner/repo.git",
            "https://github.com/owner/repo/extra.git",
            "git@github.com:owner/repo-other.git",
        ] {
            assert!(!remote_matches_github(url, "owner/repo"), "{url}");
        }
    }

    #[test]
    fn test_remote_matches_github_rejects_foreign_hosts_and_noise() {
        for url in [
            "https://github.com.evil.example/owner/repo.git",
            "https://gitlab.com/owner/repo.git",
            "ssh://git@bitbucket.org/owner/repo.git",
            "git@gitlab.com:owner/repo.git",
            "/srv/git/owner/repo.git",
            "https://github.com/owner",
            "https://github.com",
        ] {
            assert!(!remote_matches_github(url, "owner/repo"), "{url}");
        }
        // A commented URL or prose is not a remote value.
        assert!(!remote_matches_github(
            "# clone from https://github.com/owner/repo.git",
            "owner/repo"
        ));
        assert!(!remote_matches_github("", "owner/repo"));
        // A github_name that is not a plain owner/repo never matches.
        assert!(!remote_matches_github(
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo.git"
        ));
        assert!(!remote_matches_github(
            "https://github.com/owner/repo.git",
            "repo"
        ));
        assert!(!remote_matches_github(
            "https://github.com/a/b/c.git",
            "a/b"
        ));
        // A pushurl value of another repo is not what the fetch URL says.
        assert!(!remote_matches_github(
            "https://github.com/owner/elsewhere.git",
            "owner/repo"
        ));
    }

    #[test]
    fn test_remote_matches_github_accepts_dotted_repo_names() {
        // GitHub allows dots in repo names (but never a trailing `.git`).
        assert!(remote_matches_github(
            "https://github.com/owner/repo.js.git",
            "owner/repo.js"
        ));
        assert!(remote_matches_github(
            "https://github.com/owner/repo.js",
            "owner/repo.js"
        ));
    }

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
    fn test_is_ignored_for_add_true_for_ignored_false_for_tracked() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(
            tmp.path().join(".gitignore"),
            "docs/backend-requirements/\n",
        )
        .unwrap();
        run_git(tmp.path(), &["add", ".gitignore"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add gitignore"]);
        // The trailing-slash pattern only matches an actual directory — create
        // both folders on disk, exactly as `autocommit_repo` sees them (it only
        // probes folders that exist).
        fs::create_dir_all(tmp.path().join("docs/backend-requirements")).unwrap();
        fs::create_dir_all(tmp.path().join("docs/server-api")).unwrap();

        let git = check_git_available().expect("git available");
        assert!(
            is_ignored_for_add(
                &git,
                tmp.path(),
                &PathBuf::from("docs/backend-requirements")
            )
            .unwrap(),
            "gitignored folder must report true"
        );
        assert!(
            !is_ignored_for_add(&git, tmp.path(), &PathBuf::from("docs/server-api")).unwrap(),
            "non-ignored folder must report false"
        );
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
    fn test_current_branch_ignores_same_named_tag_and_detects_detached() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        fs::write(tmp.path().join("README.md"), "# r\n").unwrap();
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);
        // With a tag named like the branch, `rev-parse --abbrev-ref HEAD`
        // prints `heads/master`.
        run_git(tmp.path(), &["tag", "master"]);
        let git = check_git_available().expect("git available");
        assert_eq!(
            current_branch(&git, tmp.path()).unwrap(),
            Some("master".to_string())
        );
        run_git(tmp.path(), &["checkout", "-q", "--detach"]);
        assert_eq!(current_branch(&git, tmp.path()).unwrap(), None);
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

    // ── T-000244: marker listing + the scoped index-only commit ──────────────

    /// Raw stdout of a successful git command.
    fn git_stdout(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// HEAD's commit id — to assert it did not move.
    fn head_sha(dir: &Path) -> String {
        git_stdout(dir, &["rev-parse", "HEAD"]).trim().to_string()
    }

    /// The unmerged stage lines of the index, to compare them across a call.
    fn unmerged_lines(dir: &Path) -> String {
        git_stdout(dir, &["ls-files", "-u"])
    }

    /// `<status> <path>` of everything staged against HEAD.
    fn staged_status(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["diff", "--cached", "--name-status", "-z"])
            .current_dir(dir)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let fields: Vec<&str> = text.split('\0').filter(|s| !s.is_empty()).collect();
        fields
            .chunks(2)
            .map(|c| format!("{} {}", c[0], c[1]))
            .collect()
    }

    /// Files of HEAD's tree.
    fn head_tree(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["ls-tree", "-r", "--name-only", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.to_string())
            .collect()
    }

    /// A repo whose HEAD already tracks a rule copy, an approval marker and a
    /// reason file under `docs/sdh_skills/`.
    fn repo_with_tracked_marker(dir: &Path, marker_note: &str) {
        init_test_repo(dir);
        write_file(dir, "docs/sdh_skills/sdh-alpha.md", "ALPHA\n");
        write_file(dir, "docs/sdh_skills/sdh-alpha.md.approved", marker_note);
        write_file(dir, "docs/sdh_skills/sdh-alpha.md.conflict", "reason\n");
        run_git(dir, &["add", "-A", "."]);
        run_git(dir, &["commit", "-q", "-m", "marker committed by an agent"]);
    }

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let full = dir.join(rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_list_tracked_files_reads_the_index_not_the_disk() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        // The worktree marker is gone (a collection already took it up); the
        // index entry — what keeps it in the branch — is still there.
        fs::remove_file(tmp.path().join("docs/sdh_skills/sdh-alpha.md.approved")).unwrap();

        let git = check_git_available().expect("git available");
        let listed = list_tracked_files(&git, tmp.path(), MARKER_PATHSPECS).unwrap();
        assert_eq!(
            listed,
            vec![
                PathBuf::from("docs/sdh_skills/sdh-alpha.md.approved"),
                PathBuf::from("docs/sdh_skills/sdh-alpha.md.conflict"),
            ]
        );
        // The rule copy itself is not a marker.
        assert!(!is_rule_marker(Path::new("docs/sdh_skills/sdh-alpha.md")));
        assert!(is_rule_marker(Path::new("docs/sdh_skills/a.md.conflict")));
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_list_markers_in_rev_reads_the_history_not_the_worktree() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        // Folder gone from disk entirely — the deletion is still to be delivered.
        fs::remove_dir_all(tmp.path().join("docs/sdh_skills")).unwrap();

        let git = check_git_available().expect("git available");
        assert_eq!(
            list_markers_in_rev(&git, tmp.path(), "HEAD").unwrap(),
            vec![
                PathBuf::from("docs/sdh_skills/sdh-alpha.md.approved"),
                PathBuf::from("docs/sdh_skills/sdh-alpha.md.conflict"),
            ]
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_deletes_the_marker_and_keeps_the_file() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved in chat\n");
        let marker = tmp.path().join("docs/sdh_skills/sdh-alpha.md.approved");

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });

        // Both markers left the branch; the rule copy stayed; nothing on disk
        // was deleted.
        assert!(!head_tree(tmp.path())
            .iter()
            .any(|f| f.contains(".approved")));
        assert!(!head_tree(tmp.path())
            .iter()
            .any(|f| f.contains(".conflict")));
        assert!(head_tree(tmp.path()).contains(&"docs/sdh_skills/sdh-alpha.md".to_string()));
        assert_eq!(
            fs::read_to_string(&marker).unwrap(),
            "approved in chat\n",
            "the live marker is byte-for-byte intact"
        );
        // And the delivery left nothing staged behind.
        assert!(
            staged_status(tmp.path()).is_empty(),
            "{:?}",
            staged_status(tmp.path())
        );

        // Second run: nothing left to deliver.
        let again = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(again, CommitOutcome::NothingToCommit);
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_never_adds_a_live_or_modified_marker() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        // A live marker of another rule beside the copies, and the tracked one
        // rewritten on disk: neither may reach the branch.
        write_file(tmp.path(), "docs/sdh_skills/sdh-beta.md.approved", "NEW\n");
        fs::write(
            tmp.path().join("docs/sdh_skills/sdh-alpha.md.approved"),
            "MODIFIED\n",
        )
        .unwrap();

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[PathBuf::from("docs/sdh_skills")],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });

        let tree = head_tree(tmp.path());
        assert!(
            !tree
                .iter()
                .any(|f| f.ends_with(".approved") || f.ends_with(".conflict")),
            "no marker in the tree: {:?}",
            tree
        );
        assert!(tree.contains(&"docs/sdh_skills/sdh-alpha.md".to_string()));
        assert!(
            !tree.contains(&"docs/sdh_skills/sdh-beta.md".to_string()),
            "a live marker's copy was never a content path"
        );
        // The modified text stayed in the worktree, not in git.
        assert_eq!(
            fs::read_to_string(tmp.path().join("docs/sdh_skills/sdh-alpha.md.approved")).unwrap(),
            "MODIFIED\n"
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_preserves_foreign_staged_and_unstaged_work() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        // A tracked file the dev has since edited (unstaged), foreign staged
        // work and a foreign untracked file.
        write_file(tmp.path(), "README.md", "clean\n");
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "readme"]);
        write_file(tmp.path(), "src/lib.rs", "fn main() {}\n");
        run_git(tmp.path(), &["add", "src/lib.rs"]);
        fs::write(tmp.path().join("README.md"), "dirty\n").unwrap();
        write_file(tmp.path(), "scratch.txt", "mine\n");

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });

        // None of the foreign work is in the commit.
        let tree = head_tree(tmp.path());
        assert!(!tree.contains(&"src/lib.rs".to_string()), "{:?}", tree);
        assert!(!tree.contains(&"scratch.txt".to_string()));
        assert!(tree.contains(&"README.md".to_string()), "{:?}", tree);
        // And all of it is still where it was.
        assert!(staged_status(tmp.path()).contains(&"A src/lib.rs".to_string()));
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&status.stdout).to_string();
        assert!(text.contains(" M README.md"), "{}", text);
        assert!(text.contains("?? scratch.txt"), "{}", text);
        // The markers the commit deleted are now plain untracked files.
        assert!(
            text.contains("?? docs/sdh_skills/sdh-alpha.md.approved"),
            "{}",
            text
        );
        assert!(
            text.contains("?? docs/sdh_skills/sdh-alpha.md.conflict"),
            "{}",
            text
        );
        assert!(
            !text.lines().any(|l| l.ends_with("sdh-alpha.md")),
            "the untouched rule copy shows no change: {}",
            text
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_carries_the_ordinary_content_paths() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[PathBuf::from("docs/server-api")],
            "chore(sdh-sync): sync cross-repo files",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 3 });

        let tree = head_tree(tmp.path());
        assert!(
            tree.contains(&"docs/server-api/api.md".to_string()),
            "{:?}",
            tree
        );
        assert!(!tree.iter().any(|f| f.ends_with(".approved")));
        // The committed file left no phantom staged deletion behind.
        assert!(
            staged_status(tmp.path()).is_empty(),
            "{:?}",
            staged_status(tmp.path())
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_wrong_branch_touches_nothing() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "dev",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
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
        assert!(
            head_tree(tmp.path())
                .iter()
                .any(|f| f.ends_with(".approved")),
            "the marker is still tracked: the deletion waits for the allowed branch"
        );
        assert!(staged_status(tmp.path()).is_empty());
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_no_temp_index_left_behind() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        let git = check_git_available().expect("git available");
        commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();
        let out = Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let git_dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        let left: Vec<_> = fs::read_dir(if git_dir.is_absolute() {
            git_dir
        } else {
            tmp.path().join(git_dir)
        })
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("sdh-index-") || n.starts_with("sdh-untrack-index-"))
        .collect();
        assert!(
            left.is_empty(),
            "throw-away index files left behind: {:?}",
            left
        );
    }

    // ── Review of f0a40b2: unsafe states, the reset pathspec, the temp index ─

    /// A checkout mid-conflict: `git merge other` stopped with unresolved
    /// entries (and MERGE_HEAD set) on an unrelated file.
    fn repo_with_conflicted_merge(dir: &Path) {
        repo_with_tracked_marker(dir, "approved\n");
        write_file(dir, "README.md", "base\n");
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-q", "-m", "base"]);
        run_git(dir, &["checkout", "-q", "-b", "other"]);
        fs::write(dir.join("README.md"), "theirs\n").unwrap();
        run_git(dir, &["commit", "-q", "-am", "theirs"]);
        run_git(dir, &["checkout", "-q", "master"]);
        fs::write(dir.join("README.md"), "ours\n").unwrap();
        run_git(dir, &["commit", "-q", "-am", "ours"]);
        let out = Command::new("git")
            .args(["merge", "other"])
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "the merge must conflict: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_leaves_a_mid_merge_checkout_alone() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        fs::write(tmp.path().join(".git/MERGE_HEAD"), "deadbeef\n").unwrap();
        let before = head_sha(tmp.path());

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();

        assert_eq!(outcome, CommitOutcome::NothingToCommit);
        assert_eq!(head_sha(tmp.path()), before, "HEAD must not move mid-merge");
        assert!(
            tmp.path().join(".git/MERGE_HEAD").exists(),
            "the in-flight merge is not concluded behind the dev's back"
        );
        assert!(
            head_tree(tmp.path())
                .iter()
                .any(|f| f.ends_with(".approved")),
            "the marker stays tracked until a safer pass"
        );
        assert!(
            staged_status(tmp.path()).is_empty(),
            "the index is not touched: {:?}",
            staged_status(tmp.path())
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_leaves_a_mid_rebase_checkout_alone() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        fs::create_dir(tmp.path().join(".git/rebase-apply")).unwrap();
        let before = head_sha(tmp.path());

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();

        assert_eq!(outcome, CommitOutcome::NothingToCommit);
        assert_eq!(
            head_sha(tmp.path()),
            before,
            "HEAD must not move mid-rebase"
        );
        assert!(
            tmp.path().join(".git/rebase-apply").exists(),
            "the in-flight rebase is not disturbed"
        );
        assert!(head_tree(tmp.path())
            .iter()
            .any(|f| f.ends_with(".approved")));
    }

    /// A conflicted merge has MERGE_HEAD set AND unresolved entries — both
    /// nets of the guard must hold, and the temp index may never hide the
    /// unresolved ones from the real index.
    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_leaves_a_conflicted_merge_alone() {
        let tmp = TempDir::new().unwrap();
        repo_with_conflicted_merge(tmp.path());
        let before = head_sha(tmp.path());
        let unmerged = unmerged_lines(tmp.path());
        assert!(!unmerged.trim().is_empty(), "the fixture must conflict");

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();

        assert_eq!(outcome, CommitOutcome::NothingToCommit);
        assert_eq!(head_sha(tmp.path()), before, "HEAD must not move");
        assert_eq!(
            unmerged_lines(tmp.path()),
            unmerged,
            "the unresolved entries are still in the real index"
        );
        assert!(tmp.path().join(".git/MERGE_HEAD").exists());
        assert!(head_tree(tmp.path())
            .iter()
            .any(|f| f.ends_with(".approved")));
    }

    /// A conflicted merge has MERGE_HEAD set AND unresolved entries. With
    /// MERGE_HEAD removed — what a killed merge leaves behind — only the
    /// unmerged-index net of the guard refuses the commit.
    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_leaves_an_unmerged_index_alone() {
        let tmp = TempDir::new().unwrap();
        repo_with_conflicted_merge(tmp.path());
        fs::remove_file(tmp.path().join(".git/MERGE_HEAD")).unwrap();
        assert!(!tmp.path().join(".git/MERGE_HEAD").exists());
        let before = head_sha(tmp.path());
        let unmerged = unmerged_lines(tmp.path());
        assert!(!unmerged.trim().is_empty(), "the fixture must conflict");

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();

        assert_eq!(outcome, CommitOutcome::NothingToCommit);
        assert_eq!(head_sha(tmp.path()), before, "HEAD must not move");
        assert_eq!(
            unmerged_lines(tmp.path()),
            unmerged,
            "the unresolved entries are still in the real index"
        );
        assert!(head_tree(tmp.path())
            .iter()
            .any(|f| f.ends_with(".approved")));
    }

    /// The guard reads the state through `--git-path`, so a mid-merge LINKED
    /// worktree (whose `.git` is a file) is refused too.
    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_leaves_a_mid_merge_linked_worktree_alone() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        write_file(tmp.path(), "README.md", "# r\n");
        run_git(tmp.path(), &["add", "README.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "init"]);
        let wt = tmp.path().join("wt");
        run_git(
            tmp.path(),
            &["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()],
        );
        write_file(&wt, "docs/sdh_skills/sdh-alpha.md.approved", "approved\n");
        run_git(&wt, &["add", "-A", "."]);
        run_git(&wt, &["commit", "-q", "-m", "marker"]);
        let merge_head = {
            let p =
                PathBuf::from(git_stdout(&wt, &["rev-parse", "--git-path", "MERGE_HEAD"]).trim());
            if p.is_absolute() {
                p
            } else {
                wt.join(p)
            }
        };
        fs::write(&merge_head, "deadbeef\n").unwrap();
        let before = head_sha(&wt);

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            &wt,
            "wt",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();

        assert_eq!(outcome, CommitOutcome::NothingToCommit);
        assert_eq!(head_sha(&wt), before, "HEAD must not move mid-merge");
        assert!(merge_head.exists(), "the in-flight merge survives");
        assert!(head_tree(&wt).iter().any(|f| f.ends_with(".approved")));
    }

    /// `git reset -q -- HEAD <paths>` read HEAD as a pathspec and reset the
    /// staged blob of an unrelated file literally named `HEAD`.
    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_reset_spares_a_file_literally_named_head() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        write_file(tmp.path(), "HEAD", "tracked\n");
        run_git(tmp.path(), &["add", "--", "HEAD"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add a file named HEAD"]);
        // The dev stages a new blob for it and removes the worktree copy —
        // the staged blob is exactly what the broken reset destroyed. (The
        // worktree copy is removed so this test isolates the reset; the
        // sibling test below covers the file-present shape that the
        // `diff-index … HEAD --` fix made reachable.)
        fs::write(tmp.path().join("HEAD"), "service file\n").unwrap();
        run_git(tmp.path(), &["add", "--", "HEAD"]);
        fs::remove_file(tmp.path().join("HEAD")).unwrap();
        assert!(
            staged_status(tmp.path()).contains(&"M HEAD".to_string()),
            "the fixture must have the new blob staged: {:?}",
            staged_status(tmp.path())
        );

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();

        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });
        let staged = staged_status(tmp.path());
        assert!(
            staged.contains(&"M HEAD".to_string()),
            "the dev's staged blob for the file named HEAD survives: {:?}",
            staged
        );
        assert!(
            !staged
                .iter()
                .any(|s| s.ends_with(".approved") || s.ends_with(".conflict")),
            "the marker deletions are reconciled, nothing else: {:?}",
            staged
        );
    }

    /// Review fix (2nd pass of T-000244): `diff-index --cached --name-only -z
    /// HEAD` without a trailing `--` read `HEAD` as an ambiguous
    /// revision/path argument whenever a worktree file literally named `HEAD`
    /// existed, so the marker cleanup failed on that repo outright. With the
    /// `--` the commit goes through WITH the file present, and both halves of
    /// the dev's state — the staged blob and the worktree contents — come out
    /// unchanged.
    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_copes_with_a_worktree_file_literally_named_head() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        write_file(tmp.path(), "HEAD", "service file\n");
        run_git(tmp.path(), &["add", "--", "HEAD"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add a file named HEAD"]);
        // The dev stages a new blob for the file and keeps it on disk — the
        // exact shape the ambiguous-argument failure used to make unreachable.
        fs::write(tmp.path().join("HEAD"), "staged and on disk\n").unwrap();
        run_git(tmp.path(), &["add", "--", "HEAD"]);
        assert!(
            staged_status(tmp.path()).contains(&"M HEAD".to_string()),
            "the fixture must have the new blob staged: {:?}",
            staged_status(tmp.path())
        );

        let git = check_git_available().expect("git available");
        let outcome = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        )
        .unwrap();

        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });
        assert_eq!(
            fs::read_to_string(tmp.path().join("HEAD")).unwrap(),
            "staged and on disk\n",
            "the worktree contents are untouched"
        );
        let staged = staged_status(tmp.path());
        assert!(
            staged.contains(&"M HEAD".to_string()),
            "the dev's staged blob for the file named HEAD survives: {:?}",
            staged
        );
        assert!(
            !staged
                .iter()
                .any(|s| s.ends_with(".approved") || s.ends_with(".conflict")),
            "only the marker deletions are reconciled: {:?}",
            staged
        );
    }

    /// A temp index whose `read-tree HEAD` failed must never reach the commit:
    /// later ops would run against an absent index and read every unrelated
    /// tracked file as a deletion. Injected by deleting HEAD's root tree
    /// object — every name still resolves, only the read fails.
    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_commit_paths_index_only_fails_closed_when_the_temp_index_init_fails() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path(), "approved\n");
        let tree = git_stdout(tmp.path(), &["rev-parse", "HEAD^{tree}"])
            .trim()
            .to_string();
        fs::remove_file(
            tmp.path()
                .join(format!(".git/objects/{}/{}", &tree[..2], &tree[2..])),
        )
        .unwrap();
        let head_before = head_sha(tmp.path());
        let index_before = git_stdout(tmp.path(), &["ls-files", "-s"]);

        let git = check_git_available().expect("git available");
        let failed = commit_paths_index_only(
            &git,
            tmp.path(),
            "master",
            &[],
            "chore(sdh-sync): untrack rule hand-back markers",
            None,
            "Solo Dev Hub",
            "hub@example.com",
        );

        let err = failed.expect_err("a failed read-tree must fail the commit");
        assert!(err.contains("read-tree"), "{}", err);
        assert_eq!(head_sha(tmp.path()), head_before, "no commit was made");
        assert_eq!(
            git_stdout(tmp.path(), &["ls-files", "-s"]),
            index_before,
            "the real index is unchanged — no phantom deletions"
        );
        // The throw-away index (and its lock) did not survive the failure.
        let git_dir = PathBuf::from(git_stdout(tmp.path(), &["rev-parse", "--git-dir"]).trim());
        let left: Vec<_> = fs::read_dir(if git_dir.is_absolute() {
            git_dir
        } else {
            tmp.path().join(git_dir)
        })
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("sdh-untrack-index-"))
        .collect();
        assert!(
            left.is_empty(),
            "throw-away index files left behind: {:?}",
            left
        );
    }
}
