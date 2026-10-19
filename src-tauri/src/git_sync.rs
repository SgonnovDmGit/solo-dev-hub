// F-000044 / T-000217: git layer for contributor mode.
//
// Two installs of the app (owner = "main", helper = "contributor") share repos
// through git. This module is the git plumbing only: fetch / fast-forward /
// commit into a branch that is NOT checked out / push. No sync logic, no DB,
// no UI — the orchestration (T-000218, T-000220) calls these functions.
//
// Safety rules enforced here, before any git process is spawned:
//   * `master` / `main` are never written to, never pushed (`is_protected_branch`).
//   * no `--force`, `-f` or `+refspec` is ever passed to git.
//   * network commands run non-interactively (`GIT_TERMINAL_PROMPT=0`) and
//     under a timeout, so a credential prompt can never freeze the app.
//   * the working tree, the real index, HEAD and the current branch are never
//     touched by `commit_files_to_branch` (temp index + plumbing).

// Consumed by the contributor-mode tasks that follow (T-000218, T-000220).
#![allow(dead_code)]

use crate::git_ops::{self, GitBinary, RepoState};
use std::ffi::OsStr;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Default timeout for network commands (fetch / push).
pub const NET_TIMEOUT: Duration = Duration::from_secs(60);
/// Timeout for local commands. Local git ops finish in milliseconds; this only
/// guards against a wedged process (e.g. a hook waiting on input).
const LOCAL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long to wait for the output pipes to drain after the process exited.
const DRAIN_GRACE: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetErrorKind {
    Auth,
    Network,
    Timeout,
    Other,
}

/// Failure of a network git command. `message` never carries credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetError {
    pub kind: NetErrorKind,
    pub message: String,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfOutcome {
    UpToDate,
    FastForwarded,
    LocalAhead,
    Diverged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    UpToDate,
    Merged,
    Conflict { files: Vec<String> },
}

/// One file to write into a branch. `content: None` deletes the file.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub rel_path: PathBuf,
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchCommit {
    Committed {
        sha: String,
    },
    NothingToCommit,
    /// The branch ref moved between reading it and updating it. Nothing was
    /// written; the caller retries on the next sync.
    RefMoved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    Pushed,
    UpToDate,
    RejectedNonFastForward,
    /// The remote refused because the branch is protected (GH006 / GH013).
    RejectedProtected(String),
    RejectedOther(String),
}

// ---------------------------------------------------------------------------
// Safety helpers
// ---------------------------------------------------------------------------

/// `refs/heads/` prefix stripped, ASCII case-insensitively (NTFS and macOS
/// resolve `refs/heads/Master` to the `master` ref file).
fn short_branch(name: &str) -> &str {
    const PREFIX: &str = "refs/heads/";
    match name.get(..PREFIX.len()) {
        Some(p) if p.eq_ignore_ascii_case(PREFIX) => &name[PREFIX.len()..],
        _ => name,
    }
}

/// Branch names are compared ASCII case-insensitively everywhere in this
/// module: on case-insensitive filesystems `Dev` and `dev` are the same ref.
fn same_branch(a: &str, b: &str) -> bool {
    short_branch(a).eq_ignore_ascii_case(short_branch(b))
}

/// True for `master` and `main`, bare or as `refs/heads/<name>`, in any ASCII
/// letter case (`Master`, `MAIN`, ...).
pub fn is_protected_branch(name: &str) -> bool {
    let short = short_branch(name);
    short.eq_ignore_ascii_case("master") || short.eq_ignore_ascii_case("main")
}

fn refuse_protected(name: &str) -> Result<(), String> {
    if is_protected_branch(name) {
        Err(format!(
            "refusing to touch protected branch '{}'",
            short_branch(name)
        ))
    } else {
        Ok(())
    }
}

/// Conservative branch-name check (a subset of `git check-ref-format`): we
/// only accept short names, never `refs/...`, never anything that could be read
/// as an option or a refspec.
fn validate_branch(name: &str) -> Result<(), String> {
    let bad = name.is_empty()
        || name.starts_with('-')
        || name.starts_with('/')
        || name.starts_with("refs/")
        || name.ends_with('/')
        || name.ends_with('.')
        || name.ends_with(".lock")
        || name.contains("..")
        || name.contains("//")
        || name.contains("@{")
        || name == "@"
        || name.eq_ignore_ascii_case("HEAD")
        || name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c));
    if bad {
        Err(format!("invalid branch name '{}'", name))
    } else {
        Ok(())
    }
}

/// A rev / ref / sha argument must not be readable as an option.
fn validate_rev(rev: &str) -> Result<(), String> {
    if rev.is_empty() || rev.starts_with('-') || rev.contains('\0') || rev.contains('\n') {
        Err(format!("invalid revision '{}'", rev.replace('\n', " ")))
    } else {
        Ok(())
    }
}

/// Repo-relative path in git form (forward slashes). Rejects absolute paths,
/// `..` and anything inside `.git`.
fn git_path(path: &Path) -> Result<String, String> {
    let mut parts: Vec<&str> = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(s) => {
                let s = s
                    .to_str()
                    .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))?;
                if s.eq_ignore_ascii_case(".git") {
                    return Err(format!(
                        "path inside .git is not allowed: {}",
                        path.display()
                    ));
                }
                parts.push(s);
            }
            Component::CurDir => {}
            _ => return Err(format!("path must be repo-relative: {}", path.display())),
        }
    }
    if parts.is_empty() {
        return Err("empty path".to_string());
    }
    Ok(parts.join("/"))
}

/// Strip `scheme://user:token@` userinfo from every URL in `s`.
fn redact_credentials(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => out.push_str(&authority[at + 1..]),
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Process runner
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum RunError {
    Spawn(String),
    Timeout { cmd: String, after: Duration },
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::Spawn(m) => write!(f, "{}", m),
            RunError::Timeout { cmd, after } => {
                write!(f, "git {} timed out after {}s", cmd, after.as_secs_f32())
            }
        }
    }
}

struct RunOpts<'a> {
    env: &'a [(&'a str, &'a OsStr)],
    stdin: Option<&'a [u8]>,
    timeout: Duration,
}

impl RunOpts<'_> {
    const LOCAL: RunOpts<'static> = RunOpts {
        env: &[],
        stdin: None,
        timeout: LOCAL_TIMEOUT,
    };
}

fn drain<R: Read + Send + 'static>(reader: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    match reader {
        Some(mut r) => {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = r.read_to_end(&mut buf);
                let _ = tx.send(buf);
            });
        }
        None => {
            let _ = tx.send(Vec::new());
        }
    }
    rx
}

/// The git subcommand of `args`, skipping leading `-c <k=v>` pairs and options
/// (used only for messages).
fn subcommand<S: AsRef<OsStr>>(args: &[S]) -> String {
    let mut it = args
        .iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned());
    while let Some(a) = it.next() {
        if a == "-c" {
            it.next();
        } else if !a.starts_with('-') {
            return a;
        }
    }
    String::new()
}

/// Kill `child` and everything it spawned. `git.exe` on Windows is a launcher:
/// killing only that process leaves the real git and `git-remote-https` alive
/// and holding our pipes. `taskkill /T` walks the tree (no extra crate needed);
/// elsewhere the child leads its own process group, which `kill` takes down.
fn kill_process_tree(child: &mut Child) {
    let pid = child.id().to_string();
    #[cfg(windows)]
    {
        let _ = git_ops::spawn_cmd("taskkill")
            .args(["/T", "/F", "/PID", pid.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let group = format!("-{}", pid);
        let _ = git_ops::spawn_cmd("kill")
            .args(["-KILL", "--", group.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Run git with a timeout. Output is drained on helper threads so a large
/// stdout/stderr cannot deadlock the pipe; on timeout the whole process tree
/// is killed (see `kill_process_tree`), which also releases the pipes.
fn run_git<S: AsRef<OsStr>>(
    git: &GitBinary,
    repo: &Path,
    args: &[S],
    opts: &RunOpts,
) -> Result<Output, RunError> {
    let label = subcommand(args);
    // Relative paths would be ambiguous for `GIT_INDEX_FILE` and `current_dir`.
    if !repo.is_absolute() {
        return Err(RunError::Spawn(format!(
            "git {}: repository path must be absolute: {}",
            label,
            repo.display()
        )));
    }
    let mut cmd = git_ops::spawn_cmd(&git.0);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.args(args)
        .current_dir(repo)
        // English messages: stderr is parsed to classify failures. LC_ALL would
        // override LC_MESSAGES, so it is removed for the child.
        .env_remove("LC_ALL")
        .env("LC_MESSAGES", "C")
        .env("LANGUAGE", "C")
        .stdin(if opts.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in opts.env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| RunError::Spawn(format!("git {} failed to start: {}", label, e)))?;

    if let (Some(mut si), Some(data)) = (child.stdin.take(), opts.stdin) {
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = si.write_all(&data);
            // `si` dropped here -> EOF for the child.
        });
    }
    let out_rx = drain(child.stdout.take());
    let err_rx = drain(child.stderr.take());

    let deadline = Instant::now() + opts.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(e) => {
                kill_process_tree(&mut child);
                return Err(RunError::Spawn(format!("git {} wait failed: {}", label, e)));
            }
        }
        if Instant::now() >= deadline {
            kill_process_tree(&mut child);
            return Err(RunError::Timeout {
                cmd: label,
                after: opts.timeout,
            });
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stdout = out_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default();
    let stderr = err_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn stderr_text(out: &Output) -> String {
    redact_credentials(String::from_utf8_lossy(&out.stderr).trim())
}

fn stdout_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn failure(sub: &str, out: &Output) -> String {
    format!(
        "git {} failed ({}): {}",
        sub,
        out.status
            .code()
            .map_or("signal".to_string(), |c| c.to_string()),
        stderr_text(out)
    )
}

/// Local command that must succeed.
fn run_ok<S: AsRef<OsStr>>(git: &GitBinary, repo: &Path, args: &[S]) -> Result<Output, String> {
    run_ok_with(git, repo, args, &RunOpts::LOCAL)
}

fn run_ok_with<S: AsRef<OsStr>>(
    git: &GitBinary,
    repo: &Path,
    args: &[S],
    opts: &RunOpts,
) -> Result<Output, String> {
    let sub = subcommand(args);
    let out = run_git(git, repo, args, opts).map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out)
    } else {
        Err(failure(&sub, &out))
    }
}

// ---------------------------------------------------------------------------
// Network: fetch
// ---------------------------------------------------------------------------

fn classify_net_stderr(stderr: &str) -> NetErrorKind {
    let s = stderr.to_ascii_lowercase();
    const AUTH: &[&str] = &[
        "authentication failed",
        "could not read username",
        "could not read password",
        "permission denied",
        "terminal prompts disabled",
        "invalid username or password",
        "returned error: 401",
        "returned error: 403",
        "access denied",
    ];
    const NETWORK: &[&str] = &[
        "could not resolve host",
        "unable to access",
        "connection",
        "failed to connect",
        "network is unreachable",
        "does not appear to be a git repository",
        "could not read from remote repository",
        "operation timed out",
        "timed out",
    ];
    if AUTH.iter().any(|p| s.contains(p)) {
        NetErrorKind::Auth
    } else if NETWORK.iter().any(|p| s.contains(p)) {
        NetErrorKind::Network
    } else {
        NetErrorKind::Other
    }
}

fn net_error_from_output(sub: &str, out: &Output) -> NetError {
    let text = stderr_text(out);
    let mut message = format!("git {} failed: {}", sub, text);
    if message.len() > 1000 {
        let mut cut = 1000;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        message.truncate(cut);
    }
    NetError {
        kind: classify_net_stderr(&text),
        message,
    }
}

fn net_error_from_run(e: RunError) -> NetError {
    match e {
        RunError::Timeout { .. } => NetError {
            kind: NetErrorKind::Timeout,
            message: e.to_string(),
        },
        RunError::Spawn(m) => NetError {
            kind: NetErrorKind::Other,
            message: redact_credentials(&m),
        },
    }
}

fn net_opts<'a>(env: &'a [(&'a str, &'a OsStr)], timeout: Duration) -> RunOpts<'a> {
    RunOpts {
        env,
        stdin: None,
        timeout,
    }
}

/// Non-interactive environment for every network command.
fn net_env() -> [(&'static str, &'static OsStr); 2] {
    [
        ("GIT_TERMINAL_PROMPT", OsStr::new("0")),
        // Git Credential Manager: fail instead of opening a prompt.
        ("GCM_INTERACTIVE", OsStr::new("never")),
    ]
}

/// A transfer slower than 1000 B/s for 30 s ends by itself, so a stalled HTTP
/// connection does not have to wait for the overall timeout.
const LOW_SPEED_ARGS: [&str; 4] = [
    "-c",
    "http.lowSpeedLimit=1000",
    "-c",
    "http.lowSpeedTime=30",
];

/// `git fetch <remote>` — non-interactive, 60 s timeout.
pub fn fetch(git: &GitBinary, repo: &Path, remote: &str) -> Result<(), NetError> {
    fetch_with_timeout(git, repo, remote, NET_TIMEOUT)
}

pub fn fetch_with_timeout(
    git: &GitBinary,
    repo: &Path,
    remote: &str,
    timeout: Duration,
) -> Result<(), NetError> {
    validate_rev(remote).map_err(|m| NetError {
        kind: NetErrorKind::Other,
        message: m,
    })?;
    let env = net_env();
    let mut args: Vec<&str> = LOW_SPEED_ARGS.to_vec();
    args.extend(["fetch", remote]);
    let out = run_git(git, repo, &args, &net_opts(&env, timeout)).map_err(net_error_from_run)?;
    if out.status.success() {
        Ok(())
    } else {
        Err(net_error_from_output("fetch", &out))
    }
}

// ---------------------------------------------------------------------------
// Read-only queries
// ---------------------------------------------------------------------------

/// Resolve `rev` to a commit sha, `None` when it does not exist.
pub fn rev_parse(git: &GitBinary, repo: &Path, rev: &str) -> Result<Option<String>, String> {
    validate_rev(rev)?;
    let spec = format!("{}^{{commit}}", rev);
    let out = run_git(
        git,
        repo,
        &["rev-parse", "--verify", "--quiet", spec.as_str()],
        &RunOpts::LOCAL,
    )
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(Some(stdout_text(&out)));
    }
    if out.status.code() == Some(1) && out.stderr.is_empty() {
        return Ok(None);
    }
    Err(failure("rev-parse", &out))
}

pub fn merge_base(
    git: &GitBinary,
    repo: &Path,
    a: &str,
    b: &str,
) -> Result<Option<String>, String> {
    validate_rev(a)?;
    validate_rev(b)?;
    let out =
        run_git(git, repo, &["merge-base", a, b], &RunOpts::LOCAL).map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(Some(stdout_text(&out)));
    }
    if out.status.code() == Some(1) && out.stderr.is_empty() {
        return Ok(None);
    }
    Err(failure("merge-base", &out))
}

/// True when `a` is an ancestor of `b` (or equal).
pub fn is_ancestor(git: &GitBinary, repo: &Path, a: &str, b: &str) -> Result<bool, String> {
    validate_rev(a)?;
    validate_rev(b)?;
    let out = run_git(
        git,
        repo,
        &["merge-base", "--is-ancestor", a, b],
        &RunOpts::LOCAL,
    )
    .map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(failure("merge-base --is-ancestor", &out)),
    }
}

/// Blob sha of `path` at `rev`, `None` when the path (or the rev) is absent.
/// Callers that must tell the two apart resolve the rev with `rev_parse` first.
pub fn blob_sha_at(
    git: &GitBinary,
    repo: &Path,
    rev: &str,
    path: impl AsRef<Path>,
) -> Result<Option<String>, String> {
    validate_rev(rev)?;
    let spec = format!("{}:{}", rev, git_path(path.as_ref())?);
    let out = run_git(
        git,
        repo,
        &["rev-parse", "--verify", "--quiet", spec.as_str()],
        &RunOpts::LOCAL,
    )
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(Some(stdout_text(&out)));
    }
    if out.status.code() == Some(1) && out.stderr.is_empty() {
        return Ok(None);
    }
    Err(failure("rev-parse", &out))
}

/// Raw bytes of `path` at `rev` (what `git show <rev>:<path>` prints for a
/// blob), `None` when the path does not exist in that rev. Resolved in two
/// steps (sha, then `cat-file blob`) so "absent" never depends on parsing
/// git's error text.
pub fn show_file(
    git: &GitBinary,
    repo: &Path,
    rev: &str,
    path: impl AsRef<Path>,
) -> Result<Option<Vec<u8>>, String> {
    let Some(sha) = blob_sha_at(git, repo, rev, path)? else {
        return Ok(None);
    };
    let out = run_ok(git, repo, &["cat-file", "blob", sha.as_str()])?;
    Ok(Some(out.stdout))
}

/// Like `show_file`, but refuses a blob larger than `max_len` bytes (checked
/// with `git cat-file -s` before the content is read), so untrusted refs
/// cannot make the app load an arbitrary amount of data.
pub fn show_file_capped(
    git: &GitBinary,
    repo: &Path,
    rev: &str,
    path: impl AsRef<Path>,
    max_len: u64,
) -> Result<Option<Vec<u8>>, String> {
    let Some(sha) = blob_sha_at(git, repo, rev, path)? else {
        return Ok(None);
    };
    let size = run_ok(git, repo, &["cat-file", "-s", sha.as_str()])?;
    let size: u64 = stdout_text(&size)
        .parse()
        .map_err(|e| format!("git cat-file -s: bad size: {}", e))?;
    if size > max_len {
        return Err(format!(
            "blob is too large ({} bytes, limit {})",
            size, max_len
        ));
    }
    let out = run_ok(git, repo, &["cat-file", "blob", sha.as_str()])?;
    Ok(Some(out.stdout))
}

// ---------------------------------------------------------------------------
// Ref updates
// ---------------------------------------------------------------------------

/// Compare-and-swap ref update. `old: None` means "the ref must not exist".
/// `Ok(false)` when the current value did not match; `Err` for other failures.
pub fn update_ref_cas(
    git: &GitBinary,
    repo: &Path,
    refname: &str,
    new: &str,
    old: Option<&str>,
) -> Result<bool, String> {
    refuse_protected(refname)?;
    if !refname.starts_with("refs/") {
        return Err(format!("invalid ref name '{}'", refname));
    }
    validate_rev(new)?;
    let zero = "0".repeat(40);
    let old = old.unwrap_or(zero.as_str());
    validate_rev(old)?;
    let out = run_git(
        git,
        repo,
        &["update-ref", "--no-deref", refname, new, old],
        &RunOpts::LOCAL,
    )
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(true);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("cannot lock ref")
        || err.contains("but expected")
        || err.contains("reference already exists")
    {
        return Ok(false);
    }
    Err(failure("update-ref", &out))
}

/// Branches checked out in ANY worktree of this repository (the main one, the
/// current one and every `git worktree add` one), as short names.
fn checked_out_branches(git: &GitBinary, repo: &Path) -> Result<Vec<String>, String> {
    let out = run_ok(git, repo, &["worktree", "list", "--porcelain"])?;
    let mut branches: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("branch "))
        .map(|b| short_branch(b.trim()).to_string())
        .collect();
    // `worktree list` already includes the current worktree; this is a cheap
    // second source in case the list is unavailable for it (e.g. bare layouts).
    if let Some(cur) = git_ops::current_branch(git, repo)? {
        branches.push(cur);
    }
    Ok(branches)
}

/// Err when `branch` is checked out anywhere (case-insensitively): moving a
/// checked-out branch behind the work tree's back would corrupt that checkout.
fn refuse_checked_out(
    git: &GitBinary,
    repo: &Path,
    branch: &str,
    hint: &str,
) -> Result<(), String> {
    if checked_out_branches(git, repo)?
        .iter()
        .any(|b| same_branch(b, branch))
    {
        return Err(format!("'{}' is a checked-out branch; {}", branch, hint));
    }
    Ok(())
}

/// Repo state from the real git dir (`rev-parse --git-path`), so it also works
/// when `.git` is a file (linked worktree, submodule).
pub(crate) fn repo_state(git: &GitBinary, repo: &Path) -> Result<RepoState, String> {
    let out = run_ok(
        git,
        repo,
        &[
            "rev-parse",
            "--git-path",
            "MERGE_HEAD",
            "--git-path",
            "rebase-merge",
            "--git-path",
            "rebase-apply",
        ],
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let exists = |line: Option<&str>| {
        line.map(|l| {
            let p = PathBuf::from(l.trim());
            (if p.is_absolute() { p } else { repo.join(p) }).exists()
        })
        .unwrap_or(false)
    };
    let mut lines = text.lines();
    if exists(lines.next()) {
        return Ok(RepoState::MidMerge);
    }
    if exists(lines.next()) || exists(lines.next()) {
        return Ok(RepoState::MidRebase);
    }
    Ok(RepoState::Clean)
}

/// Move local `branch` (NOT the checked-out one) forward to `target_rev`
/// without touching the working tree. A missing local branch is created.
pub fn fast_forward_ref(
    git: &GitBinary,
    repo: &Path,
    branch: &str,
    target_rev: &str,
) -> Result<FfOutcome, String> {
    refuse_protected(branch)?;
    validate_branch(branch)?;
    validate_rev(target_rev)?;
    refuse_checked_out(git, repo, branch, "use merge_ff_only")?;
    let target = rev_parse(git, repo, target_rev)?
        .ok_or_else(|| format!("target '{}' does not exist", target_rev))?;
    let refname = format!("refs/heads/{}", branch);
    let Some(local) = rev_parse(git, repo, &refname)? else {
        return if update_ref_cas(git, repo, &refname, &target, None)? {
            Ok(FfOutcome::FastForwarded)
        } else {
            Err(format!("branch '{}' appeared concurrently", branch))
        };
    };
    if local == target {
        return Ok(FfOutcome::UpToDate);
    }
    if is_ancestor(git, repo, &target, &local)? {
        return Ok(FfOutcome::LocalAhead);
    }
    if is_ancestor(git, repo, &local, &target)? {
        return if update_ref_cas(git, repo, &refname, &target, Some(&local))? {
            Ok(FfOutcome::FastForwarded)
        } else {
            Err(format!("branch '{}' moved concurrently", branch))
        };
    }
    Ok(FfOutcome::Diverged)
}

/// The checked-out, non-protected branch of `repo`, with the repo in a clean
/// (not mid-merge / mid-rebase) state — the precondition of both merge fns.
fn checked_out_branch_for_merge(git: &GitBinary, repo: &Path) -> Result<String, String> {
    let branch = git_ops::current_branch(git, repo)?
        .ok_or_else(|| "HEAD is detached or unborn; refusing to merge".to_string())?;
    refuse_protected(&branch)?;
    match repo_state(git, repo)? {
        RepoState::Clean => Ok(branch),
        RepoState::MidMerge => Err("repository is in the middle of a merge".to_string()),
        RepoState::MidRebase => Err("repository is in the middle of a rebase".to_string()),
    }
}

/// Fast-forward the CURRENT branch to `upstream` (`git merge --ff-only`).
pub fn merge_ff_only(git: &GitBinary, repo: &Path, upstream: &str) -> Result<FfOutcome, String> {
    validate_rev(upstream)?;
    checked_out_branch_for_merge(git, repo)?;
    let head = rev_parse(git, repo, "HEAD")?.ok_or_else(|| "HEAD has no commit".to_string())?;
    let up = rev_parse(git, repo, upstream)?
        .ok_or_else(|| format!("upstream '{}' does not exist", upstream))?;
    if head == up {
        return Ok(FfOutcome::UpToDate);
    }
    if is_ancestor(git, repo, &up, &head)? {
        return Ok(FfOutcome::LocalAhead);
    }
    let out = run_git(
        git,
        repo,
        &["merge", "--ff-only", up.as_str()],
        &RunOpts::LOCAL,
    )
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(FfOutcome::FastForwarded);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("Not possible to fast-forward") {
        return Ok(FfOutcome::Diverged);
    }
    Err(failure("merge --ff-only", &out))
}

/// Merge `upstream` into the CURRENT branch. On conflict the merge is aborted
/// (the files that conflicted are reported) and HEAD is verified restored.
pub fn merge_or_abort(
    git: &GitBinary,
    repo: &Path,
    upstream: &str,
) -> Result<MergeOutcome, String> {
    validate_rev(upstream)?;
    checked_out_branch_for_merge(git, repo)?;
    let head_before =
        rev_parse(git, repo, "HEAD")?.ok_or_else(|| "HEAD has no commit".to_string())?;
    let up = rev_parse(git, repo, upstream)?
        .ok_or_else(|| format!("upstream '{}' does not exist", upstream))?;
    if is_ancestor(git, repo, &up, &head_before)? {
        return Ok(MergeOutcome::UpToDate);
    }
    let env: [(&str, &OsStr); 1] = [("GIT_MERGE_AUTOEDIT", OsStr::new("no"))];
    let opts = RunOpts {
        env: &env,
        stdin: None,
        timeout: LOCAL_TIMEOUT,
    };
    let out = run_git(git, repo, &["merge", "--no-edit", up.as_str()], &opts)
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(MergeOutcome::Merged);
    }
    if repo_state(git, repo)? != RepoState::MidMerge {
        // Refused before touching anything (dirty tree overlap, no identity, ...).
        return Err(failure("merge", &out));
    }
    // The abort is attempted whatever happens while listing the conflicts, so a
    // failed listing can never leave the repo stuck mid-merge.
    let listing = run_ok(git, repo, &["diff", "--name-only", "--diff-filter=U", "-z"]);
    let abort = run_ok(git, repo, &["merge", "--abort"]);
    let restored = match (rev_parse(git, repo, "HEAD"), repo_state(git, repo)) {
        (Ok(head), Ok(state)) => {
            if head.as_deref() == Some(head_before.as_str()) && state == RepoState::Clean {
                Ok(())
            } else {
                Err("merge --abort did not restore the pre-merge state".to_string())
            }
        }
        (Err(e), _) | (_, Err(e)) => Err(format!("could not verify the repo after abort: {}", e)),
    };
    let mut errors: Vec<String> = Vec::new();
    let files = match listing {
        Ok(diff) => diff
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .collect(),
        Err(e) => {
            errors.push(format!("listing conflicted files: {}", e));
            Vec::new()
        }
    };
    if let Err(e) = abort {
        errors.push(format!("aborting the merge: {}", e));
    }
    if let Err(e) = restored {
        errors.push(e);
    }
    if errors.is_empty() {
        Ok(MergeOutcome::Conflict { files })
    } else {
        Err(errors.join("; "))
    }
}

// ---------------------------------------------------------------------------
// Writing objects / committing into a branch that is not checked out
// ---------------------------------------------------------------------------

/// `git hash-object -w --stdin --path=<rel_path>`: stores `bytes` as a blob,
/// applying the path's clean / eol filters exactly like a normal `git add`.
pub fn hash_object_write(
    git: &GitBinary,
    repo: &Path,
    rel_path: impl AsRef<Path>,
    bytes: &[u8],
) -> Result<String, String> {
    let path_arg = format!("--path={}", git_path(rel_path.as_ref())?);
    let opts = RunOpts {
        env: &[],
        stdin: Some(bytes),
        timeout: LOCAL_TIMEOUT,
    };
    let out = run_ok_with(
        git,
        repo,
        &["hash-object", "-w", "--stdin", path_arg.as_str()],
        &opts,
    )?;
    Ok(stdout_text(&out))
}

/// Deletes the temp index file when dropped, on every exit path.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        // `git read-tree` may leave a stale lock if it was killed.
        let mut lock = self.0.as_os_str().to_owned();
        lock.push(".lock");
        let _ = std::fs::remove_file(PathBuf::from(lock));
    }
}

static INDEX_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_index_path(git: &GitBinary, repo: &Path) -> Result<PathBuf, String> {
    let name = format!(
        "sdh-index-{}-{}",
        std::process::id(),
        INDEX_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let out = run_ok(git, repo, &["rev-parse", "--git-path", name.as_str()])?;
    let p = PathBuf::from(stdout_text(&out));
    Ok(if p.is_absolute() { p } else { repo.join(p) })
}

/// Commit `changes` onto `branch` (a branch that is NOT checked out) using
/// plumbing and a throw-away index: the working tree, the real index, HEAD and
/// the current branch are never touched. The ref is moved with a
/// compare-and-swap, so a concurrent move yields `RefMoved`, never a lost update.
pub fn commit_files_to_branch(
    git: &GitBinary,
    repo: &Path,
    branch: &str,
    changes: &[FileChange],
    message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<BranchCommit, String> {
    commit_files_to_branch_inner(
        git,
        repo,
        branch,
        changes,
        message,
        author_name,
        author_email,
        &mut || {},
    )
}

/// Same as `commit_files_to_branch`; `before_cas` runs right before the ref
/// update. Production passes a no-op — the seam lets tests move the ref inside
/// the read-to-CAS window.
#[allow(clippy::too_many_arguments)]
fn commit_files_to_branch_inner(
    git: &GitBinary,
    repo: &Path,
    branch: &str,
    changes: &[FileChange],
    message: &str,
    author_name: &str,
    author_email: &str,
    before_cas: &mut dyn FnMut(),
) -> Result<BranchCommit, String> {
    refuse_protected(branch)?;
    validate_branch(branch)?;
    refuse_checked_out(git, repo, branch, "commit it with commit_paths")?;
    let refname = format!("refs/heads/{}", branch);
    let old = rev_parse(git, repo, &refname)?
        .ok_or_else(|| format!("branch '{}' does not exist", branch))?;
    if changes.is_empty() {
        return Ok(BranchCommit::NothingToCommit);
    }
    // Validate every path before creating anything.
    let paths: Vec<String> = changes
        .iter()
        .map(|c| git_path(&c.rel_path))
        .collect::<Result<_, _>>()?;

    let index = TempIndex(temp_index_path(git, repo)?);
    let env: [(&str, &OsStr); 1] = [("GIT_INDEX_FILE", index.0.as_os_str())];
    let idx_opts = RunOpts {
        env: &env,
        stdin: None,
        timeout: LOCAL_TIMEOUT,
    };

    // A stale file at this path (crashed run, recycled pid) must never act as a
    // starting index or block the lock; start clean.
    let _ = std::fs::remove_file(&index.0);
    run_ok_with(git, repo, &["read-tree", old.as_str()], &idx_opts)?;
    for (change, path) in changes.iter().zip(&paths) {
        match &change.content {
            Some(bytes) => {
                let sha = hash_object_write(git, repo, &change.rel_path, bytes)?;
                let cacheinfo = format!("100644,{},{}", sha, path);
                run_ok_with(
                    git,
                    repo,
                    &["update-index", "--add", "--cacheinfo", cacheinfo.as_str()],
                    &idx_opts,
                )?;
            }
            None => {
                run_ok_with(
                    git,
                    repo,
                    &["update-index", "--force-remove", "--", path.as_str()],
                    &idx_opts,
                )?;
            }
        }
    }
    let tree = stdout_text(&run_ok_with(git, repo, &["write-tree"], &idx_opts)?);
    let old_tree_spec = format!("{}^{{tree}}", old);
    let old_tree = stdout_text(&run_ok(
        git,
        repo,
        &["rev-parse", "--verify", old_tree_spec.as_str()],
    )?);
    if tree == old_tree {
        return Ok(BranchCommit::NothingToCommit);
    }

    let mut msg = message.to_string();
    if !msg.ends_with('\n') {
        msg.push('\n');
    }
    let name_cfg = format!("user.name={}", author_name);
    let email_cfg = format!("user.email={}", author_email);
    let commit_opts = RunOpts {
        env: &[],
        stdin: Some(msg.as_bytes()),
        timeout: LOCAL_TIMEOUT,
    };
    let new = stdout_text(&run_ok_with(
        git,
        repo,
        &[
            "-c",
            name_cfg.as_str(),
            "-c",
            email_cfg.as_str(),
            "-c",
            "commit.gpgsign=false",
            "commit-tree",
            tree.as_str(),
            "-p",
            old.as_str(),
        ],
        &commit_opts,
    )?);

    before_cas();
    if update_ref_cas(git, repo, &refname, &new, Some(&old))? {
        Ok(BranchCommit::Committed { sha: new })
    } else {
        Ok(BranchCommit::RefMoved)
    }
}

// ---------------------------------------------------------------------------
// Network: push
// ---------------------------------------------------------------------------

/// Push `branches` to `remote` in one `git push --porcelain` (never forced,
/// never a protected branch). A rejected branch is a per-branch outcome; only a
/// process-level failure (auth / network / timeout) is an `Err`.
pub fn push_branches(
    git: &GitBinary,
    repo: &Path,
    remote: &str,
    branches: &[&str],
) -> Result<Vec<(String, PushOutcome)>, NetError> {
    push_branches_with_timeout(git, repo, remote, branches, NET_TIMEOUT)
}

pub fn push_branches_with_timeout(
    git: &GitBinary,
    repo: &Path,
    remote: &str,
    branches: &[&str],
    timeout: Duration,
) -> Result<Vec<(String, PushOutcome)>, NetError> {
    let refuse = |m: String| NetError {
        kind: NetErrorKind::Other,
        message: m,
    };
    for b in branches {
        refuse_protected(b).map_err(refuse)?;
        validate_branch(b).map_err(refuse)?;
    }
    validate_rev(remote).map_err(refuse)?;
    if branches.is_empty() {
        return Ok(Vec::new());
    }
    let mut args: Vec<String> = LOW_SPEED_ARGS.iter().map(|a| a.to_string()).collect();
    args.extend(["push".into(), "--porcelain".into(), remote.into()]);
    for b in branches {
        args.push(format!("refs/heads/{0}:refs/heads/{0}", b));
    }
    let env = net_env();
    let out = run_git(git, repo, &args, &net_opts(&env, timeout)).map_err(net_error_from_run)?;
    parse_push_output(
        branches,
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
        &out,
    )
}

/// One status line of `git push --porcelain`:
/// `<flag>\t<from>:<to>\t<summary> [(<reason>)]`.
struct PushLine {
    flag: char,
    branch: String,
    summary: String,
}

fn parse_push_lines(stdout: &str) -> Vec<PushLine> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let flag = parts.next()?.chars().next().unwrap_or(' ');
            let refspec = parts.next()?;
            let summary = parts.next().unwrap_or("").trim().to_string();
            let to = refspec.rsplit(':').next()?;
            let branch = to.strip_prefix("refs/heads/")?.to_string();
            Some(PushLine {
                flag,
                branch,
                summary,
            })
        })
        .collect()
}

const PROTECTED_MARKERS: &[&str] = &[
    "gh006",
    "gh013",
    "protected branch",
    "repository rule violations",
    "rule violations",
];

fn has_protected_marker(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    PROTECTED_MARKERS.iter().any(|m| l.contains(m))
}

/// Reason text when the remote refused `branch` because it is protected.
fn protected_reason(
    branch: &str,
    summary: &str,
    stderr: &str,
    only_rejected: bool,
) -> Option<String> {
    if has_protected_marker(summary) {
        return Some(summary.to_string());
    }
    let strip = |l: &str| l.trim().trim_start_matches("remote:").trim().to_string();
    let ref_token = format!("refs/heads/{}", branch);
    let arrow = format!("{0} -> {0}", branch);
    // A stderr line naming this branch and carrying a marker.
    for line in stderr.lines() {
        if has_protected_marker(line) && (line.contains(&ref_token) || line.contains(&arrow)) {
            return Some(strip(line));
        }
    }
    // A single rejected branch + a marker anywhere in stderr (GH013 lists the
    // rule violations on following lines without naming the branch again).
    if only_rejected {
        if let Some(line) = stderr.lines().find(|l| has_protected_marker(l)) {
            return Some(strip(line));
        }
    }
    None
}

fn parse_push_output(
    branches: &[&str],
    stdout: &str,
    stderr: &str,
    out: &Output,
) -> Result<Vec<(String, PushOutcome)>, NetError> {
    let lines = parse_push_lines(stdout);
    if lines.is_empty() && !out.status.success() {
        return Err(net_error_from_output("push", out));
    }
    let stderr = redact_credentials(stderr);
    let rejected = lines.iter().filter(|l| l.flag == '!').count();
    let results = branches
        .iter()
        .map(|b| {
            let outcome = match lines.iter().find(|l| l.branch == *b) {
                None => PushOutcome::RejectedOther("git push reported no status".to_string()),
                Some(l) => match l.flag {
                    '=' => PushOutcome::UpToDate,
                    '!' => {
                        let low = l.summary.to_ascii_lowercase();
                        if let Some(reason) =
                            protected_reason(b, &l.summary, &stderr, rejected == 1)
                        {
                            PushOutcome::RejectedProtected(redact_credentials(&reason))
                        } else if low.contains("non-fast-forward")
                            || low.contains("fetch first")
                            || low.contains("stale info")
                        {
                            PushOutcome::RejectedNonFastForward
                        } else {
                            PushOutcome::RejectedOther(redact_credentials(&l.summary))
                        }
                    }
                    // ' ' fast-forward, '*' new ref, '+' forced (never requested), '-' delete.
                    _ => PushOutcome::Pushed,
                },
            };
            (b.to_string(), outcome)
        })
        .collect();
    Ok(results)
}

// ---------------------------------------------------------------------------
// Local repo config
// ---------------------------------------------------------------------------

/// `git config --local --get <key>`.
pub fn config_get(git: &GitBinary, repo: &Path, key: &str) -> Result<Option<String>, String> {
    validate_rev(key)?;
    let out = run_git(
        git,
        repo,
        &["config", "--local", "--get", key],
        &RunOpts::LOCAL,
    )
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        let v = String::from_utf8_lossy(&out.stdout);
        return Ok(Some(v.trim_end_matches(['\r', '\n']).to_string()));
    }
    if out.status.code() == Some(1) {
        return Ok(None);
    }
    Err(failure("config", &out))
}

/// `git config --local <key> <value>`.
pub fn config_set(git: &GitBinary, repo: &Path, key: &str, value: &str) -> Result<(), String> {
    validate_rev(key)?;
    run_ok(git, repo, &["config", "--local", "--", key, value]).map(|_| ())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn gb() -> GitBinary {
        GitBinary(PathBuf::from("git"))
    }

    /// Run git, assert success, return trimmed stdout.
    fn g(dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn configure(dir: &Path) {
        g(dir, &["config", "user.email", "repo@example.com"]);
        g(dir, &["config", "user.name", "Repo User"]);
        g(dir, &["config", "core.autocrlf", "false"]);
        g(dir, &["config", "commit.gpgsign", "false"]);
    }

    /// bare remote + clone A (branch `dev` pushed, plus `shared`).
    struct Fixture {
        _tmp: TempDir,
        remote: PathBuf,
        a: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let remote = tmp.path().join("remote.git");
        fs::create_dir(&remote).unwrap();
        g(&remote, &["init", "--bare", "-q"]);
        g(&remote, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        let a = tmp.path().join("a");
        fs::create_dir(&a).unwrap();
        g(&a, &["init", "-q"]);
        g(&a, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        configure(&a);
        g(&a, &["remote", "add", "origin", remote.to_str().unwrap()]);
        fs::write(a.join("README.md"), "hello\n").unwrap();
        g(&a, &["add", "."]);
        g(&a, &["commit", "-q", "-m", "init"]);
        g(&a, &["branch", "shared"]);
        g(&a, &["push", "-q", "origin", "dev", "shared"]);
        Fixture {
            _tmp: tmp,
            remote,
            a,
        }
    }

    fn clone_b(fx: &Fixture) -> PathBuf {
        let b = fx._tmp.path().join("b");
        g(
            fx._tmp.path(),
            &[
                "clone",
                "-c",
                "core.autocrlf=false",
                "-q",
                "-b",
                "dev",
                fx.remote.to_str().unwrap(),
                b.to_str().unwrap(),
            ],
        );
        configure(&b);
        b
    }

    fn commit_file(dir: &Path, name: &str, content: &str, msg: &str) -> String {
        fs::write(dir.join(name), content).unwrap();
        g(dir, &["add", name]);
        g(dir, &["commit", "-q", "-m", msg]);
        g(dir, &["rev-parse", "HEAD"])
    }

    fn change(path: &str, content: Option<&str>) -> FileChange {
        FileChange {
            rel_path: PathBuf::from(path),
            content: content.map(|c| c.as_bytes().to_vec()),
        }
    }

    // ----- pure helpers -------------------------------------------------

    #[test]
    fn protected_branch_detection() {
        assert!(is_protected_branch("master"));
        assert!(is_protected_branch("main"));
        assert!(is_protected_branch("refs/heads/master"));
        assert!(is_protected_branch("refs/heads/main"));
        assert!(is_protected_branch("Master"));
        assert!(is_protected_branch("MAIN"));
        assert!(is_protected_branch("refs/heads/Master"));
        assert!(is_protected_branch("REFS/HEADS/main"));
        assert!(!is_protected_branch("Mainline"));
        assert!(!is_protected_branch("dev"));
        assert!(!is_protected_branch("main2"));
        assert!(!is_protected_branch("feature/main"));
        assert!(!is_protected_branch("refs/remotes/origin/main"));
    }

    #[test]
    fn redacts_userinfo_from_urls() {
        assert_eq!(
            redact_credentials(
                "fatal: unable to access 'https://user:tok3n@github.com/o/r.git/': x"
            ),
            "fatal: unable to access 'https://github.com/o/r.git/': x"
        );
        assert_eq!(
            redact_credentials("https://ghp_abc@github.com/o/r and https://github.com/x"),
            "https://github.com/o/r and https://github.com/x"
        );
        assert_eq!(redact_credentials("no url here"), "no url here");
        assert_eq!(redact_credentials("ssh://git@host/x"), "ssh://host/x");
    }

    #[test]
    fn classifies_net_errors() {
        use NetErrorKind::*;
        let c = classify_net_stderr;
        assert_eq!(
            c("remote: Invalid username or password.\nfatal: Authentication failed for 'x'"),
            Auth
        );
        assert_eq!(c("fatal: could not read Username for 'https://github.com': terminal prompts disabled"), Auth);
        assert_eq!(c("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository."), Auth);
        assert_eq!(
            c("fatal: unable to access 'x': The requested URL returned error: 403"),
            Auth
        );
        assert_eq!(
            c("fatal: unable to access 'x': Could not resolve host: github.com"),
            Network
        );
        assert_eq!(
            c("fatal: unable to access 'x': Failed to connect to github.com port 443"),
            Network
        );
        assert_eq!(
            c("fatal: '/nope' does not appear to be a git repository"),
            Network
        );
        assert_eq!(c("error: something odd"), Other);
    }

    #[test]
    fn git_path_validation() {
        assert_eq!(git_path(Path::new("docs/todo.md")).unwrap(), "docs/todo.md");
        assert_eq!(
            git_path(Path::new("./docs\\done.md"))
                .unwrap()
                .replace('\\', "/"),
            "docs/done.md"
        );
        assert!(git_path(Path::new("../x")).is_err());
        assert!(git_path(Path::new(".git/config")).is_err());
        assert!(git_path(Path::new("")).is_err());
        #[cfg(windows)]
        assert!(git_path(Path::new("C:\\x")).is_err());
        #[cfg(not(windows))]
        assert!(git_path(Path::new("/x")).is_err());
    }

    #[test]
    fn branch_name_validation() {
        assert!(validate_branch("dev").is_ok());
        assert!(validate_branch("sdh/shared-1").is_ok());
        for bad in [
            "",
            "-x",
            "a..b",
            "a b",
            "a:b",
            "refs/heads/x",
            "x.lock",
            "x/",
            "a~1",
            "a@{1}",
            "HEAD",
            "head",
        ] {
            assert!(validate_branch(bad).is_err(), "{bad:?} should be invalid");
        }
    }

    // ----- push output parser -------------------------------------------

    fn ok_output() -> Output {
        Command::new("git").arg("--version").output().unwrap()
    }

    fn failed_output(stderr: &str) -> Output {
        let mut o = Command::new("git")
            .arg("no-such-subcommand-xyz")
            .output()
            .unwrap();
        assert!(!o.status.success());
        o.stderr = stderr.as_bytes().to_vec();
        o
    }

    #[test]
    fn parses_push_porcelain_basic() {
        let stdout = "To /tmp/remote.git\n\
                      =\trefs/heads/a:refs/heads/a\t[up to date]\n \trefs/heads/b:refs/heads/b\tabc1234..def5678\n\
                      *\trefs/heads/c:refs/heads/c\t[new branch]\n\
                      !\trefs/heads/d:refs/heads/d\t[rejected] (non-fast-forward)\n\
                      !\trefs/heads/e:refs/heads/e\t[rejected] (fetch first)\n\
                      !\trefs/heads/f:refs/heads/f\t[remote rejected] (hook declined)\n\
                      Done\n";
        let r = parse_push_output(
            &["a", "b", "c", "d", "e", "f", "g"],
            stdout,
            "",
            &failed_output(""),
        )
        .unwrap();
        let m: Vec<_> = r.iter().map(|(_, o)| o.clone()).collect();
        assert_eq!(m[0], PushOutcome::UpToDate);
        assert_eq!(m[1], PushOutcome::Pushed);
        assert_eq!(m[2], PushOutcome::Pushed);
        assert_eq!(m[3], PushOutcome::RejectedNonFastForward);
        assert_eq!(m[4], PushOutcome::RejectedNonFastForward);
        assert!(matches!(&m[5], PushOutcome::RejectedOther(s) if s.contains("hook declined")));
        assert!(matches!(&m[6], PushOutcome::RejectedOther(_)));
        assert_eq!(r[0].0, "a");
    }

    #[test]
    fn parses_push_gh006_protected() {
        let stdout = "To https://github.com/o/r.git\n\
                      !\trefs/heads/dev:refs/heads/dev\t[remote rejected] (protected branch hook declined)\n\
                      Done\n";
        let stderr = "remote: error: GH006: Protected branch update failed for refs/heads/dev.        \n\
                      To https://github.com/o/r.git\n ! [remote rejected] dev -> dev (protected branch hook declined)\n";
        let r = parse_push_output(&["dev"], stdout, stderr, &failed_output(stderr)).unwrap();
        assert!(
            matches!(&r[0].1, PushOutcome::RejectedProtected(s) if s.contains("protected branch"))
        );
    }

    #[test]
    fn parses_push_gh013_rule_violations() {
        let stdout = "To https://github.com/o/r.git\n\
                      !\trefs/heads/dev:refs/heads/dev\t[remote rejected] (push declined due to repository rule violations)\n\
                      Done\n";
        let stderr = "remote: error: GH013: Repository rule violations found for refs/heads/dev.\n\
                      remote: - Cannot update this protected ref.\n";
        let r = parse_push_output(&["dev"], stdout, stderr, &failed_output(stderr)).unwrap();
        assert!(matches!(&r[0].1, PushOutcome::RejectedProtected(_)));
    }

    #[test]
    fn push_protected_marker_attributed_per_branch() {
        // Two rejections; stderr names only `dev`. `other` must not be flagged.
        let stdout = "!\trefs/heads/dev:refs/heads/dev\t[remote rejected] (hook declined)\n\
                      !\trefs/heads/other:refs/heads/other\t[remote rejected] (hook declined)\n";
        let stderr = "remote: error: GH006: Protected branch update failed for refs/heads/dev.\n";
        let r =
            parse_push_output(&["dev", "other"], stdout, stderr, &failed_output(stderr)).unwrap();
        assert!(matches!(&r[0].1, PushOutcome::RejectedProtected(_)));
        assert!(matches!(&r[1].1, PushOutcome::RejectedOther(_)));
    }

    #[test]
    fn push_process_failure_is_net_error_without_credentials() {
        let stderr = "fatal: Authentication failed for 'https://user:secret@github.com/o/r.git/'\n";
        let e = parse_push_output(&["dev"], "", stderr, &failed_output(stderr)).unwrap_err();
        assert_eq!(e.kind, NetErrorKind::Auth);
        assert!(!e.message.contains("secret"), "{}", e.message);
        assert!(e.message.contains("github.com"));
        // success with no lines -> per-branch fallback, not a panic
        let r = parse_push_output(&["dev"], "", "", &ok_output()).unwrap();
        assert!(matches!(&r[0].1, PushOutcome::RejectedOther(_)));
    }

    // ----- runner -------------------------------------------------------

    #[test]
    fn run_git_kills_on_timeout() {
        let tmp = TempDir::new().unwrap();
        g(tmp.path(), &["init", "-q"]);
        let opts = RunOpts {
            env: &[],
            stdin: None,
            timeout: Duration::from_millis(300),
        };
        let started = Instant::now();
        let r = run_git(
            &gb(),
            tmp.path(),
            &["-c", "alias.slow=!sleep 10", "slow"],
            &opts,
        );
        assert!(matches!(r, Err(RunError::Timeout { .. })), "{r:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(RunError::Timeout {
            cmd: "fetch".into(),
            after: Duration::from_secs(60)
        }
        .to_string()
        .contains("timed out"));
        assert_eq!(
            net_error_from_run(RunError::Timeout {
                cmd: "fetch".into(),
                after: Duration::from_secs(60)
            })
            .kind,
            NetErrorKind::Timeout
        );
    }

    #[test]
    fn run_git_handles_large_output_and_stdin() {
        let fx = fixture();
        // ~1.5 MB through stdin and back through hash-object / cat-file.
        let big = vec![b'x'; 1_500_000];
        let sha = hash_object_write(&gb(), &fx.a, "big.txt", &big).unwrap();
        let out = run_ok(&gb(), &fx.a, &["cat-file", "blob", sha.as_str()]).unwrap();
        assert_eq!(out.stdout.len(), big.len());
    }

    // ----- fetch ---------------------------------------------------------

    #[test]
    fn fetch_ok_and_unreachable() {
        let fx = fixture();
        let b = clone_b(&fx);
        commit_file(&fx.a, "n.txt", "1", "more");
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        fetch(&gb(), &b, "origin").unwrap();
        assert_eq!(
            rev_parse(&gb(), &b, "origin/dev").unwrap().unwrap(),
            g(&fx.a, &["rev-parse", "HEAD"])
        );

        g(
            &b,
            &[
                "remote",
                "add",
                "ghost",
                fx._tmp.path().join("nope.git").to_str().unwrap(),
            ],
        );
        let e = fetch(&gb(), &b, "ghost").unwrap_err();
        assert_eq!(e.kind, NetErrorKind::Network, "{e}");
        assert!(!e.message.is_empty());
    }

    #[test]
    fn fetch_does_not_leak_token_from_remote_url() {
        let fx = fixture();
        let b = clone_b(&fx);
        // Nothing listens on port 9 -> connection failure naming the URL.
        g(
            &b,
            &[
                "remote",
                "add",
                "tok",
                "http://user:supersecret@127.0.0.1:9/x.git",
            ],
        );
        let e = fetch_with_timeout(&gb(), &b, "tok", Duration::from_secs(20)).unwrap_err();
        assert!(!e.message.contains("supersecret"), "{}", e.message);
        assert!(matches!(
            e.kind,
            NetErrorKind::Network
                | NetErrorKind::Timeout
                | NetErrorKind::Auth
                | NetErrorKind::Other
        ));
    }

    // ----- queries -------------------------------------------------------

    #[test]
    fn rev_parse_merge_base_is_ancestor() {
        let fx = fixture();
        let first = g(&fx.a, &["rev-parse", "HEAD"]);
        let second = commit_file(&fx.a, "n.txt", "1", "two");
        assert_eq!(rev_parse(&gb(), &fx.a, "HEAD").unwrap().unwrap(), second);
        assert_eq!(rev_parse(&gb(), &fx.a, "nope").unwrap(), None);
        assert_eq!(
            merge_base(&gb(), &fx.a, &first, &second).unwrap().unwrap(),
            first
        );
        assert!(is_ancestor(&gb(), &fx.a, &first, &second).unwrap());
        assert!(!is_ancestor(&gb(), &fx.a, &second, &first).unwrap());
        assert!(rev_parse(&gb(), &fx.a, "--all").is_err());
        g(&fx.a, &["checkout", "-q", "--orphan", "island"]);
        let island = commit_file(&fx.a, "i.txt", "i", "island");
        assert_eq!(merge_base(&gb(), &fx.a, &second, &island).unwrap(), None);
    }

    #[test]
    fn show_file_and_blob_sha() {
        let fx = fixture();
        fs::create_dir_all(fx.a.join("docs")).unwrap();
        fs::write(fx.a.join("docs/todo.md"), b"line\r\nbytes\n").unwrap();
        g(&fx.a, &["add", "."]);
        g(&fx.a, &["commit", "-q", "-m", "docs"]);
        let sha = blob_sha_at(&gb(), &fx.a, "HEAD", "docs/todo.md")
            .unwrap()
            .unwrap();
        assert_eq!(sha, g(&fx.a, &["rev-parse", "HEAD:docs/todo.md"]));
        assert_eq!(
            show_file(&gb(), &fx.a, "HEAD", "docs/todo.md")
                .unwrap()
                .unwrap(),
            b"line\r\nbytes\n"
        );
        assert_eq!(
            show_file(&gb(), &fx.a, "HEAD", "docs/none.md").unwrap(),
            None
        );
        assert_eq!(
            blob_sha_at(&gb(), &fx.a, "HEAD~1", "docs/todo.md").unwrap(),
            None
        );
        assert!(show_file(&gb(), &fx.a, "HEAD", "../x").is_err());
    }

    // ----- update_ref_cas / fast_forward_ref ------------------------------

    #[test]
    fn update_ref_cas_match_and_mismatch() {
        let fx = fixture();
        let first = g(&fx.a, &["rev-parse", "HEAD"]);
        let second = commit_file(&fx.a, "n.txt", "1", "two");
        // create: must not exist
        assert!(update_ref_cas(&gb(), &fx.a, "refs/heads/t1", &first, None).unwrap());
        assert!(!update_ref_cas(&gb(), &fx.a, "refs/heads/t1", &second, None).unwrap());
        // match
        assert!(update_ref_cas(&gb(), &fx.a, "refs/heads/t1", &second, Some(&first)).unwrap());
        // mismatch
        assert!(!update_ref_cas(&gb(), &fx.a, "refs/heads/t1", &first, Some(&first)).unwrap());
        assert_eq!(g(&fx.a, &["rev-parse", "t1"]), second);
        // protected refused before git
        let nogit = GitBinary(PathBuf::from("/definitely/not/git"));
        let e = update_ref_cas(&nogit, &fx.a, "refs/heads/main", &first, None).unwrap_err();
        assert!(e.contains("protected"), "{e}");
    }

    #[test]
    fn fast_forward_ref_outcomes() {
        let fx = fixture();
        let b = clone_b(&fx);
        let base = g(&b, &["rev-parse", "HEAD"]);
        // Remote moves `shared` ahead via clone A.
        g(&fx.a, &["checkout", "-q", "shared"]);
        let ahead = commit_file(&fx.a, "s.txt", "s", "shared ahead");
        g(&fx.a, &["push", "-q", "origin", "shared"]);
        fetch(&gb(), &b, "origin").unwrap();

        // B has no local `shared` -> Missing -> created.
        assert_eq!(
            fast_forward_ref(&gb(), &b, "shared", "origin/shared").unwrap(),
            FfOutcome::FastForwarded
        );
        assert_eq!(g(&b, &["rev-parse", "shared"]), ahead);
        // now equal
        assert_eq!(
            fast_forward_ref(&gb(), &b, "shared", "origin/shared").unwrap(),
            FfOutcome::UpToDate
        );
        // roll the local branch back -> fast-forward
        g(&b, &["branch", "-f", "shared", &base]);
        assert_eq!(
            fast_forward_ref(&gb(), &b, "shared", "origin/shared").unwrap(),
            FfOutcome::FastForwarded
        );
        assert_eq!(g(&b, &["rev-parse", "shared"]), ahead);
        // local ahead of target
        g(&b, &["checkout", "-q", "shared"]);
        commit_file(&b, "l.txt", "l", "local only");
        g(&b, &["checkout", "-q", "dev"]);
        assert_eq!(
            fast_forward_ref(&gb(), &b, "shared", "origin/shared").unwrap(),
            FfOutcome::LocalAhead
        );
        // diverged: remote also moves
        g(&fx.a, &["checkout", "-q", "shared"]);
        commit_file(&fx.a, "r.txt", "r", "remote too");
        g(&fx.a, &["push", "-q", "origin", "shared"]);
        fetch(&gb(), &b, "origin").unwrap();
        let before = g(&b, &["rev-parse", "shared"]);
        assert_eq!(
            fast_forward_ref(&gb(), &b, "shared", "origin/shared").unwrap(),
            FfOutcome::Diverged
        );
        assert_eq!(g(&b, &["rev-parse", "shared"]), before);
    }

    #[test]
    fn fast_forward_ref_refuses_current_protected_and_bad_target() {
        let fx = fixture();
        let e = fast_forward_ref(&gb(), &fx.a, "dev", "shared").unwrap_err();
        assert!(e.contains("checked-out"), "{e}");
        let e = fast_forward_ref(&gb(), &fx.a, "Dev", "shared").unwrap_err();
        assert!(e.contains("checked-out"), "{e}");
        let nogit = GitBinary(PathBuf::from("/definitely/not/git"));
        for p in [
            "master",
            "main",
            "refs/heads/main",
            "Master",
            "MAIN",
            "refs/heads/Main",
        ] {
            let e = fast_forward_ref(&nogit, &fx.a, p, "shared").unwrap_err();
            assert!(e.contains("protected"), "{e}");
        }
        assert!(fast_forward_ref(&gb(), &fx.a, "shared", "no-such-rev").is_err());
    }

    #[test]
    fn merge_ff_only_outcomes() {
        let fx = fixture();
        let b = clone_b(&fx);
        assert_eq!(
            merge_ff_only(&gb(), &b, "origin/dev").unwrap(),
            FfOutcome::UpToDate
        );
        let new = commit_file(&fx.a, "n.txt", "1", "ahead");
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        fetch(&gb(), &b, "origin").unwrap();
        assert_eq!(
            merge_ff_only(&gb(), &b, "origin/dev").unwrap(),
            FfOutcome::FastForwarded
        );
        assert_eq!(g(&b, &["rev-parse", "HEAD"]), new);
        assert!(b.join("n.txt").exists());
        // local ahead
        commit_file(&b, "l.txt", "l", "local");
        assert_eq!(
            merge_ff_only(&gb(), &b, "origin/dev").unwrap(),
            FfOutcome::LocalAhead
        );
        // diverge
        commit_file(&fx.a, "r.txt", "r", "remote");
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        fetch(&gb(), &b, "origin").unwrap();
        let head = g(&b, &["rev-parse", "HEAD"]);
        assert_eq!(
            merge_ff_only(&gb(), &b, "origin/dev").unwrap(),
            FfOutcome::Diverged
        );
        assert_eq!(g(&b, &["rev-parse", "HEAD"]), head);
    }

    #[test]
    fn merge_fns_refuse_protected_current_branch() {
        let fx = fixture();
        g(&fx.a, &["checkout", "-q", "-b", "master"]);
        assert!(merge_ff_only(&gb(), &fx.a, "shared")
            .unwrap_err()
            .contains("protected"));
        assert!(merge_or_abort(&gb(), &fx.a, "shared")
            .unwrap_err()
            .contains("protected"));
    }

    // ----- commit_files_to_branch ----------------------------------------

    struct Snapshot {
        status: String,
        head: String,
        branch: String,
        index_bytes: Vec<u8>,
        cached: String,
    }

    fn snapshot(repo: &Path) -> Snapshot {
        let status = g(repo, &["status", "--porcelain"]); // may refresh the index
        Snapshot {
            status,
            head: g(repo, &["rev-parse", "HEAD"]),
            branch: g(repo, &["rev-parse", "--abbrev-ref", "HEAD"]),
            index_bytes: fs::read(repo.join(".git/index")).unwrap(),
            cached: g(repo, &["diff", "--cached"]),
        }
    }

    fn assert_untouched(before: &Snapshot, repo: &Path) {
        let after_index = fs::read(repo.join(".git/index")).unwrap();
        assert_eq!(before.index_bytes, after_index, "real index changed");
        let after = snapshot(repo);
        assert_eq!(before.status, after.status);
        assert_eq!(before.head, after.head);
        assert_eq!(before.branch, after.branch);
        assert_eq!(before.cached, after.cached);
        let leftovers: Vec<_> = fs::read_dir(repo.join(".git"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("sdh-index"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp index left behind: {leftovers:?}"
        );
    }

    #[test]
    fn commit_to_branch_add_modify_delete() {
        let fx = fixture();
        // Dirty the work tree + stage something so "untouched" is meaningful.
        fs::write(fx.a.join("untracked.txt"), "u").unwrap();
        fs::write(fx.a.join("README.md"), "edited locally\n").unwrap();
        fs::write(fx.a.join("staged.txt"), "s").unwrap();
        g(&fx.a, &["add", "staged.txt"]);
        let snap = snapshot(&fx.a);
        let shared_before = g(&fx.a, &["rev-parse", "shared"]);

        // add
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[
                change("docs/todo.md", Some("- [ ] T-1\n")),
                change("docs/done.md", Some("d\n")),
            ],
            "chore(sdh-sync): add",
            "Helper",
            "helper@example.com",
        )
        .unwrap();
        let BranchCommit::Committed { sha } = r else {
            panic!("{r:?}")
        };
        assert_eq!(g(&fx.a, &["rev-parse", "shared"]), sha);
        assert_eq!(g(&fx.a, &["rev-parse", &format!("{sha}^")]), shared_before);
        assert_eq!(g(&fx.a, &["show", "shared:docs/todo.md"]), "- [ ] T-1");
        // README on the branch is the committed one, not the dirty work-tree one.
        assert_eq!(g(&fx.a, &["show", "shared:README.md"]), "hello");
        assert_untouched(&snap, &fx.a);

        // modify
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[change("docs/todo.md", Some("- [x] T-1\n"))],
            "chore(sdh-sync): mod",
            "Helper",
            "helper@example.com",
        )
        .unwrap();
        assert!(matches!(r, BranchCommit::Committed { .. }));
        assert_eq!(g(&fx.a, &["show", "shared:docs/todo.md"]), "- [x] T-1");
        assert_eq!(g(&fx.a, &["show", "shared:docs/done.md"]), "d");

        // delete
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[
                change("docs/done.md", None),
                change("docs/never-existed.md", None),
            ],
            "chore(sdh-sync): del",
            "Helper",
            "helper@example.com",
        )
        .unwrap();
        assert!(matches!(r, BranchCommit::Committed { .. }));
        assert_eq!(
            blob_sha_at(&gb(), &fx.a, "shared", "docs/done.md").unwrap(),
            None
        );
        assert!(blob_sha_at(&gb(), &fx.a, "shared", "docs/todo.md")
            .unwrap()
            .is_some());
        assert_untouched(&snap, &fx.a);
        assert_eq!(
            fs::read_to_string(fx.a.join("README.md")).unwrap(),
            "edited locally\n"
        );
    }

    #[test]
    fn commit_to_branch_nothing_to_commit() {
        let fx = fixture();
        let before = g(&fx.a, &["rev-parse", "shared"]);
        // identical content
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[change("README.md", Some("hello\n"))],
            "m",
            "H",
            "h@example.com",
        )
        .unwrap();
        assert_eq!(r, BranchCommit::NothingToCommit);
        // delete of an absent file
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[change("nope.md", None)],
            "m",
            "H",
            "h@example.com",
        )
        .unwrap();
        assert_eq!(r, BranchCommit::NothingToCommit);
        // no changes at all
        let r =
            commit_files_to_branch(&gb(), &fx.a, "shared", &[], "m", "H", "h@example.com").unwrap();
        assert_eq!(r, BranchCommit::NothingToCommit);
        assert_eq!(g(&fx.a, &["rev-parse", "shared"]), before);
    }

    #[test]
    fn commit_to_branch_ref_moved() {
        let fx = fixture();
        let snap = snapshot(&fx.a);
        let mut moved_to = String::new();
        let r = commit_files_to_branch_inner(
            &gb(),
            &fx.a,
            "shared",
            &[change("a.txt", Some("a"))],
            "mine",
            "H",
            "h@example.com",
            &mut || {
                // Somebody else commits to the same branch inside the window.
                let r = commit_files_to_branch(
                    &gb(),
                    &fx.a,
                    "shared",
                    &[change("other.txt", Some("o"))],
                    "theirs",
                    "Other",
                    "o@example.com",
                )
                .unwrap();
                let BranchCommit::Committed { sha } = r else {
                    panic!("{r:?}")
                };
                moved_to = sha;
            },
        )
        .unwrap();
        assert_eq!(r, BranchCommit::RefMoved);
        // Their commit stands; ours was not applied.
        assert_eq!(g(&fx.a, &["rev-parse", "shared"]), moved_to);
        assert_eq!(blob_sha_at(&gb(), &fx.a, "shared", "a.txt").unwrap(), None);
        assert_untouched(&snap, &fx.a);
    }

    #[test]
    fn commit_to_branch_identity_and_message() {
        let fx = fixture();
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[change("a.txt", Some("a"))],
            "chore(sdh-sync): who",
            "Sync Bot",
            "bot@example.org",
        )
        .unwrap();
        assert!(matches!(r, BranchCommit::Committed { .. }));
        assert_eq!(
            g(&fx.a, &["log", "-1", "--format=%an|%ae|%cn|%ce", "shared"]),
            "Sync Bot|bot@example.org|Sync Bot|bot@example.org"
        );
        assert_eq!(
            g(&fx.a, &["log", "-1", "--format=%s", "shared"]),
            "chore(sdh-sync): who"
        );
        // repo config untouched
        assert_eq!(g(&fx.a, &["config", "user.name"]), "Repo User");
    }

    #[test]
    fn commit_to_branch_refusals() {
        let fx = fixture();
        let nogit = GitBinary(PathBuf::from("/definitely/not/git"));
        let ch = [change("a.txt", Some("a"))];
        for p in [
            "main",
            "master",
            "refs/heads/main",
            "Master",
            "MAIN",
            "refs/heads/Main",
        ] {
            let e = commit_files_to_branch(&nogit, &fx.a, p, &ch, "m", "H", "h@e.com").unwrap_err();
            assert!(e.contains("protected"), "{e}");
        }
        // current branch
        let e = commit_files_to_branch(&gb(), &fx.a, "dev", &ch, "m", "H", "h@e.com").unwrap_err();
        assert!(e.contains("checked-out"), "{e}");
        let e = commit_files_to_branch(&gb(), &fx.a, "Dev", &ch, "m", "H", "h@e.com").unwrap_err();
        assert!(e.contains("checked-out"), "{e}");
        // missing branch
        let e =
            commit_files_to_branch(&gb(), &fx.a, "ghost", &ch, "m", "H", "h@e.com").unwrap_err();
        assert!(e.contains("does not exist"), "{e}");
        // bad path -> error, no temp index left
        let snap = snapshot(&fx.a);
        let e = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[change("../evil", Some("x"))],
            "m",
            "H",
            "h@e.com",
        )
        .unwrap_err();
        assert!(e.contains("repo-relative"), "{e}");
        assert_untouched(&snap, &fx.a);
    }

    #[test]
    fn commit_to_branch_removes_temp_index_on_error() {
        let fx = fixture();
        // Force a failure after the temp index exists: break update-index via a
        // path git refuses in the index (embedded `.git` handled earlier, so use
        // a name that hash-object accepts but update-index --cacheinfo rejects).
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[change("a//b", Some("x"))],
            "m",
            "H",
            "h@e.com",
        );
        // Whether git normalises or rejects the path, no temp index may remain.
        let _ = r;
        let leftovers = fs::read_dir(fx.a.join(".git"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("sdh-index"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn hash_object_applies_path_filters() {
        let fx = fixture();
        fs::write(fx.a.join(".gitattributes"), "*.md text eol=lf\n").unwrap();
        let sha = hash_object_write(&gb(), &fx.a, "x.md", b"a\r\nb\r\n").unwrap();
        let blob = run_ok(&gb(), &fx.a, &["cat-file", "blob", sha.as_str()]).unwrap();
        assert_eq!(blob.stdout, b"a\nb\n");
        let sha = hash_object_write(&gb(), &fx.a, "x.bin", b"a\r\nb\r\n").unwrap();
        let blob = run_ok(&gb(), &fx.a, &["cat-file", "blob", sha.as_str()]).unwrap();
        assert_eq!(blob.stdout, b"a\r\nb\r\n");
    }

    // ----- push -----------------------------------------------------------

    #[test]
    fn push_branches_outcomes() {
        let fx = fixture();
        let b = clone_b(&fx);

        // New branch + existing up-to-date one.
        g(&fx.a, &["branch", "fresh"]);
        let r = push_branches(&gb(), &fx.a, "origin", &["fresh", "shared"]).unwrap();
        assert_eq!(r[0], ("fresh".to_string(), PushOutcome::Pushed));
        assert_eq!(r[1], ("shared".to_string(), PushOutcome::UpToDate));

        // Fast-forward push.
        let new = commit_file(&fx.a, "n.txt", "1", "ahead");
        let r = push_branches(&gb(), &fx.a, "origin", &["dev"]).unwrap();
        assert_eq!(r, vec![("dev".to_string(), PushOutcome::Pushed)]);
        assert_eq!(g(&fx.remote, &["rev-parse", "dev"]), new);

        // B did not fetch -> its commit on dev is non-fast-forward.
        commit_file(&b, "other.txt", "o", "b side");
        let r = push_branches(&gb(), &b, "origin", &["dev"]).unwrap();
        assert_eq!(
            r,
            vec![("dev".to_string(), PushOutcome::RejectedNonFastForward)]
        );
        assert_eq!(
            g(&fx.remote, &["rev-parse", "dev"]),
            new,
            "remote must be unchanged"
        );

        // One call, mixed result.
        g(&b, &["branch", "only-b"]);
        let r = push_branches(&gb(), &b, "origin", &["dev", "only-b"]).unwrap();
        assert_eq!(r[0].1, PushOutcome::RejectedNonFastForward);
        assert_eq!(r[1].1, PushOutcome::Pushed);
    }

    #[test]
    fn push_branches_refuses_protected_before_spawning() {
        let fx = fixture();
        let nogit = GitBinary(PathBuf::from("/definitely/not/git"));
        for p in [
            "main",
            "master",
            "refs/heads/master",
            "Master",
            "MAIN",
            "refs/heads/Main",
        ] {
            let e = push_branches(&nogit, &fx.a, "origin", &["dev", p]).unwrap_err();
            assert_eq!(e.kind, NetErrorKind::Other);
            assert!(e.message.contains("protected"), "{}", e.message);
        }
        assert!(push_branches(&gb(), &fx.a, "origin", &[])
            .unwrap()
            .is_empty());
        // nothing reached the remote
        assert!(g(&fx.remote, &["branch", "--list", "main", "master"]).is_empty());
    }

    #[test]
    fn push_to_unreachable_remote_is_net_error() {
        let fx = fixture();
        g(
            &fx.a,
            &[
                "remote",
                "add",
                "ghost",
                fx._tmp.path().join("nope.git").to_str().unwrap(),
            ],
        );
        let e = push_branches(&gb(), &fx.a, "ghost", &["dev"]).unwrap_err();
        assert_eq!(e.kind, NetErrorKind::Network, "{e}");
    }

    // ----- config ---------------------------------------------------------

    #[test]
    fn config_get_set_local() {
        let fx = fixture();
        assert_eq!(config_get(&gb(), &fx.a, "sdh.branches").unwrap(), None);
        config_set(&gb(), &fx.a, "sdh.branches", "dev,shared").unwrap();
        assert_eq!(
            config_get(&gb(), &fx.a, "sdh.branches").unwrap().as_deref(),
            Some("dev,shared")
        );
        assert_eq!(
            g(&fx.a, &["config", "--local", "sdh.branches"]),
            "dev,shared"
        );
        config_set(&gb(), &fx.a, "sdh.branches", "-weird").unwrap();
        assert_eq!(
            config_get(&gb(), &fx.a, "sdh.branches").unwrap().as_deref(),
            Some("-weird")
        );
        assert!(config_get(&gb(), &fx.a, "--list").is_err());
    }

    // ----- merge_or_abort ---------------------------------------------------

    #[test]
    fn merge_or_abort_merged_and_up_to_date() {
        let fx = fixture();
        let b = clone_b(&fx);
        // A adds a file remotely, B adds a different one locally -> real merge.
        commit_file(&fx.a, "from-a.txt", "a", "a side");
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        commit_file(&b, "from-b.txt", "b", "b side");
        fetch(&gb(), &b, "origin").unwrap();
        assert_eq!(
            merge_or_abort(&gb(), &b, "origin/dev").unwrap(),
            MergeOutcome::Merged
        );
        assert!(b.join("from-a.txt").exists() && b.join("from-b.txt").exists());
        assert_eq!(
            g(&b, &["rev-list", "--parents", "-n1", "HEAD"])
                .split(' ')
                .count(),
            3
        );
        assert_eq!(
            merge_or_abort(&gb(), &b, "origin/dev").unwrap(),
            MergeOutcome::UpToDate
        );
    }

    #[test]
    fn merge_or_abort_conflict_restores_head() {
        let fx = fixture();
        let b = clone_b(&fx);
        commit_file(&fx.a, "clash.txt", "from a\n", "a clash");
        commit_file(&fx.a, "other-a.txt", "x\n", "a other");
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        commit_file(&b, "clash.txt", "from b\n", "b clash");
        fetch(&gb(), &b, "origin").unwrap();
        let head = g(&b, &["rev-parse", "HEAD"]);

        let r = merge_or_abort(&gb(), &b, "origin/dev").unwrap();
        assert_eq!(
            r,
            MergeOutcome::Conflict {
                files: vec!["clash.txt".to_string()]
            }
        );
        assert_eq!(g(&b, &["rev-parse", "HEAD"]), head);
        assert!(!b.join(".git/MERGE_HEAD").exists());
        assert_eq!(git_ops::detect_repo_state(&b), RepoState::Clean);
        assert_eq!(fs::read_to_string(b.join("clash.txt")).unwrap(), "from b\n");
        assert_eq!(g(&b, &["status", "--porcelain"]), "");
    }

    #[test]
    fn merge_or_abort_refuses_mid_merge() {
        let fx = fixture();
        fs::write(fx.a.join(".git/MERGE_HEAD"), "deadbeef\n").unwrap();
        let e = merge_or_abort(&gb(), &fx.a, "shared").unwrap_err();
        assert!(e.contains("middle of a merge"), "{e}");
        let e = merge_ff_only(&gb(), &fx.a, "shared").unwrap_err();
        assert!(e.contains("middle of a merge"), "{e}");
    }

    // ----- review follow-ups (T-000217) -----------------------------------

    #[test]
    fn subcommand_skips_config_pairs_and_options() {
        assert_eq!(subcommand(&["fetch", "origin"]), "fetch");
        assert_eq!(
            subcommand(&[
                "-c",
                "http.lowSpeedLimit=1000",
                "-c",
                "a=b",
                "push",
                "--porcelain"
            ]),
            "push"
        );
        assert_eq!(subcommand::<&str>(&[]), "");
    }

    #[test]
    fn timeout_kills_the_whole_process_tree() {
        let tmp = TempDir::new().unwrap();
        g(tmp.path(), &["init", "-q"]);
        let hb = tmp.path().join("heartbeat");
        let hb_arg = hb.to_str().unwrap().replace(std::path::MAIN_SEPARATOR, "/");
        // A shell loop (a grandchild of git.exe) that bumps a counter file. It
        // stops by itself after ~15 s so a failing test cannot leak it.
        let alias = format!(
            "alias.loop=!i=0; while [ $i -lt 150 ]; do i=$((i+1)); echo $i > '{hb_arg}'; sleep 0.1; done"
        );
        let opts = RunOpts {
            env: &[],
            stdin: None,
            timeout: Duration::from_millis(2500),
        };
        let r = run_git(&gb(), tmp.path(), &["-c", alias.as_str(), "loop"], &opts);
        assert!(matches!(r, Err(RunError::Timeout { .. })), "{r:?}");
        assert!(hb.exists(), "the loop never started; test proves nothing");
        std::thread::sleep(Duration::from_millis(600));
        let first = fs::read_to_string(&hb).unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let second = fs::read_to_string(&hb).unwrap();
        assert_eq!(first, second, "a process of the tree is still running");
    }

    #[test]
    fn relative_repo_path_is_refused() {
        let e = rev_parse(&gb(), Path::new("some/relative/dir"), "HEAD").unwrap_err();
        assert!(e.contains("absolute"), "{e}");
    }

    #[test]
    fn update_ref_cas_does_not_follow_symbolic_refs() {
        let fx = fixture();
        let first = g(&fx.a, &["rev-parse", "shared"]);
        let second = commit_file(&fx.a, "n.txt", "1", "two");
        g(
            &fx.a,
            &["symbolic-ref", "refs/heads/sym", "refs/heads/shared"],
        );
        let moved = update_ref_cas(&gb(), &fx.a, "refs/heads/sym", &second, Some(&first)).unwrap();
        assert!(moved);
        // The target branch must not have been moved through the symref.
        assert_eq!(g(&fx.a, &["rev-parse", "refs/heads/shared"]), first);
    }

    #[test]
    fn branch_checked_out_in_another_worktree_is_refused() {
        let fx = fixture();
        let wt = fx._tmp.path().join("wt");
        g(
            &fx.a,
            &["worktree", "add", "-q", wt.to_str().unwrap(), "shared"],
        );
        let ch = [change("a.txt", Some("a"))];
        for name in ["shared", "Shared", "SHARED"] {
            let e =
                commit_files_to_branch(&gb(), &fx.a, name, &ch, "m", "H", "h@e.com").unwrap_err();
            assert!(e.contains("checked-out"), "{name}: {e}");
            let e = fast_forward_ref(&gb(), &fx.a, name, "dev").unwrap_err();
            assert!(e.contains("checked-out"), "{name}: {e}");
        }
        // from inside the linked worktree, the main checkout's branch is refused too
        let e = commit_files_to_branch(&gb(), &wt, "dev", &ch, "m", "H", "h@e.com").unwrap_err();
        assert!(e.contains("checked-out"), "{e}");
        // an unrelated branch still works
        g(&fx.a, &["branch", "free"]);
        let r = commit_files_to_branch(&gb(), &fx.a, "free", &ch, "m", "H", "h@e.com").unwrap();
        assert!(matches!(r, BranchCommit::Committed { .. }));
    }

    #[test]
    fn mid_merge_guard_works_in_a_linked_worktree() {
        let fx = fixture();
        let wt = fx._tmp.path().join("wt");
        g(
            &fx.a,
            &["worktree", "add", "-q", "-b", "other", wt.to_str().unwrap()],
        );
        assert!(wt.join(".git").is_file(), "linked worktree has a .git file");
        assert_eq!(repo_state(&gb(), &wt).unwrap(), RepoState::Clean);
        let merge_head = g(&wt, &["rev-parse", "--git-path", "MERGE_HEAD"]);
        let merge_head = if Path::new(&merge_head).is_absolute() {
            PathBuf::from(merge_head)
        } else {
            wt.join(merge_head)
        };
        fs::write(&merge_head, "deadbeef\n").unwrap();
        assert_eq!(repo_state(&gb(), &wt).unwrap(), RepoState::MidMerge);
        let e = merge_or_abort(&gb(), &wt, "shared").unwrap_err();
        assert!(e.contains("middle of a merge"), "{e}");
        let e = merge_ff_only(&gb(), &wt, "shared").unwrap_err();
        assert!(e.contains("middle of a merge"), "{e}");
    }

    #[test]
    fn merge_or_abort_aborts_even_when_listing_conflicts_fails() {
        let fx = fixture();
        let b = clone_b(&fx);
        commit_file(&fx.a, "clash.txt", "from a\n", "a clash");
        g(&fx.a, &["push", "-q", "origin", "dev"]);
        commit_file(&b, "clash.txt", "from b\n", "b clash");
        fetch(&gb(), &b, "origin").unwrap();
        let head = g(&b, &["rev-parse", "HEAD"]);
        // `git diff` (the listing step) now fails; the merge itself does not use it.
        g(
            &b,
            &["config", "diff.orderFile", "/definitely/not/an/order/file"],
        );

        let e = merge_or_abort(&gb(), &b, "origin/dev").unwrap_err();
        assert!(e.contains("listing conflicted files"), "{e}");
        assert_eq!(g(&b, &["rev-parse", "HEAD"]), head);
        assert_eq!(repo_state(&gb(), &b).unwrap(), RepoState::Clean);
        assert!(!b.join(".git/MERGE_HEAD").exists());
    }

    #[test]
    fn commit_to_branch_normalises_crlf_like_git_add() {
        let fx = fixture();
        g(&fx.a, &["config", "core.autocrlf", "true"]);
        let r = commit_files_to_branch(
            &gb(),
            &fx.a,
            "shared",
            &[change("docs/todo.md", Some("a\r\nb\r\n"))],
            "m",
            "H",
            "h@e.com",
        )
        .unwrap();
        assert!(matches!(r, BranchCommit::Committed { .. }));
        let stored = show_file(&gb(), &fx.a, "shared", "docs/todo.md")
            .unwrap()
            .unwrap();
        assert_eq!(stored, b"a\nb\n");
        // ... exactly what `git add` would have stored for the same bytes.
        fs::create_dir_all(fx.a.join("docs")).unwrap();
        fs::write(fx.a.join("docs/other.md"), b"a\r\nb\r\n").unwrap();
        g(&fx.a, &["add", "docs/other.md"]);
        let added = g(&fx.a, &["rev-parse", ":docs/other.md"]);
        let ours = blob_sha_at(&gb(), &fx.a, "shared", "docs/todo.md")
            .unwrap()
            .unwrap();
        assert_eq!(added, ours);
    }
}
