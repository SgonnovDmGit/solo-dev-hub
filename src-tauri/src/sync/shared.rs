// F-000044 / T-000220: the sync of a shared repo (contributor mode).
//
// A repo is *shared* when the owner switched shared mode on (this install is
// "main") or when it carries a trusted `.sdh/project.json` of somebody else's
// project (this install is a "contributor", or a "pending" helper whose join
// request is not approved yet). `run_project_sync_in` drives two phases:
//
// * `run_phase_a` (before the skeleton writes): per shared repo register the
//   merge driver, `git fetch origin`, settle the role, fast-forward the
//   recipient branches, then the `import_incoming` hook (T-000221).
// * `SharedSync::finish` (after the other steps): write the structure / the
//   join request, commit the shared files into the current branch (pathspec
//   commit) and into the other recipient branches (plumbing, per-file 3-way
//   merge by id), push, and settle the repo status.
//
// Everything here refuses `master` / `main` through the `git_sync` layer.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::db::AppDb;
use crate::git_ops::{
    commit_paths, current_branch, is_git_repo, spawn_cmd, CommitOutcome, GitBinary,
};
use crate::git_sync::{self, BranchCommit, FfOutcome, FileChange, MergeOutcome, PushOutcome};
use crate::merge_driver::register_merge_driver;
use crate::models::{
    IgnoreLine, RepoSharedState, Repository, SharedDisableResult, SharedMergeResult,
    SharedPreflight, SyncIssue, SyncIssueCode as Code,
};
use crate::sync::autocommit::{
    CROSS_REPO_DIRS, MANAGED_ROOT_FILES, SDH_COMMIT_EMAIL, SDH_COMMIT_NAME,
};
use crate::sync::merge_by_id::{kind_for_path, merge_shared_bytes};
use crate::sync::shared_structure::{
    self as ss, DiscoverOutcome, JoinRequest, ProjectStructure, Role, REQUESTS_REL_DIR,
    STRUCTURE_REL_PATH,
};

const REMOTE: &str = "origin";

/// How often a repo that is not (yet) known to be shared is probed with a
/// `git fetch` to see whether its owner switched sharing on.
const PROBE_INTERVAL: Duration = Duration::from_secs(10 * 60);

// ---------------------------------------------------------------------------
// Statuses
// ---------------------------------------------------------------------------

/// Status of a shared repo. The declaration order is the priority order: the
/// first variant that applies wins (`shared_untrusted` > fetch / push failed >
/// push rejected > needs manual merge > no own branch > pending approval).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SharedStatus {
    Untrusted,
    FetchFailed,
    PushFailed,
    PushRejected,
    NeedsManualMerge,
    NoOwnBranch,
    PendingApproval,
    /// Only ever alone: the repo is not processed at all.
    NoLogin,
}

impl SharedStatus {
    /// The code stored in `repositories.shared_status`.
    pub fn as_str(&self) -> &'static str {
        match self {
            SharedStatus::Untrusted => "shared_untrusted",
            SharedStatus::FetchFailed => "fetch_failed",
            SharedStatus::PushFailed => "push_failed",
            SharedStatus::PushRejected => "push_rejected",
            SharedStatus::NeedsManualMerge => "needs_manual_merge",
            SharedStatus::NoOwnBranch => "no_own_branch",
            SharedStatus::PendingApproval => "pending_approval",
            SharedStatus::NoLogin => "shared_no_login",
        }
    }

    fn issue_code(&self) -> Code {
        match self {
            SharedStatus::Untrusted => Code::SharedUntrusted,
            SharedStatus::FetchFailed => Code::SharedFetchFailed,
            SharedStatus::PushFailed => Code::SharedPushFailed,
            SharedStatus::PushRejected => Code::SharedPushRejected,
            SharedStatus::NeedsManualMerge => Code::SharedNeedsManualMerge,
            SharedStatus::NoOwnBranch => Code::SharedNoOwnBranch,
            SharedStatus::PendingApproval => Code::SharedPendingApproval,
            SharedStatus::NoLogin => Code::SharedNoLogin,
        }
    }
}

#[derive(Debug, Clone)]
struct StatusNote {
    status: SharedStatus,
    detail: String,
}

/// What a shared pass reports back to the project sync.
#[derive(Debug, Default)]
pub struct SharedOutcome {
    pub issues: Vec<SyncIssue>,
    /// Display names of repos that received an SDH commit this pass.
    pub committed: Vec<String>,
}

// ---------------------------------------------------------------------------
// Is a repo shared? Managed blocks (spec R10)
// ---------------------------------------------------------------------------

/// A repo is shared when the owner's switch is on or this install has a cached
/// shared role for it (contributor / pending). The single answer used by every
/// caller (managed blocks, auto-commit, UI).
pub fn is_shared_state(state: &RepoSharedState) -> bool {
    state.shared_mode || state.shared_role.is_some()
}

pub fn is_shared_repo(db: &AppDb, repo_id: i64) -> bool {
    db.get_shared_state(repo_id)
        .map(|s| is_shared_state(&s))
        .unwrap_or(false)
}

/// `.gitignore` template lines dropped for a shared repo: the files that travel
/// through git (spec R10), plus `docs/sdh_skills/` (the owner commits them).
pub const SHARED_UNIGNORED_LINES: &[&str] = &[
    "docs/todo.md",
    "docs/done.md",
    "docs/bug-reports.md",
    "docs/backend-requirements/",
    "docs/client-requirements/",
    "docs/microservice-requirements/",
    "docs/server-requirements/",
    "docs/server-announcements/",
    "docs/microservice-announcements/",
    "docs/InOutBox/",
    "docs/sdh_skills/",
];

/// `.gitattributes` lines added for a shared repo.
pub const SHARED_ATTRIBUTE_LINES: &[&str] = &[
    "docs/todo.md merge=sdh",
    "docs/done.md merge=sdh",
    "docs/bug-reports.md merge=sdh",
];

/// The `.gitignore` template to render for a repo: unchanged when it is not
/// shared, minus the shared paths when it is. Identical for every role.
pub fn gitignore_template_for(template: &str, shared: bool) -> String {
    if !shared {
        return template.to_string();
    }
    let mut out: Vec<&str> = template
        .lines()
        .filter(|l| !SHARED_UNIGNORED_LINES.contains(&l.trim()))
        .collect();
    // A trailing newline is not part of any managed rule; keep the text tidy.
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// The `.gitattributes` template to render: unchanged when not shared, plus the
/// `merge=sdh` lines when shared.
pub fn gitattributes_template_for(template: &str, shared: bool) -> String {
    if !shared || template.trim().is_empty() {
        return template.to_string();
    }
    let mut out = template.trim_end().to_string();
    out.push_str("\n\n# Shared project files merge by id (Solo Dev Hub)\n");
    out.push_str(&SHARED_ATTRIBUTE_LINES.join("\n"));
    out
}

// ---------------------------------------------------------------------------
// Pathspecs
// ---------------------------------------------------------------------------

const SKILLS_DIR: &str = "docs/sdh_skills";
const SKILLS_EXCLUDES: [&str; 2] = [
    ":(exclude)docs/sdh_skills/*.approved",
    ":(exclude)docs/sdh_skills/*.conflict",
];

fn request_rel_path(login: &str) -> String {
    format!("{}/{}.json", REQUESTS_REL_DIR, login)
}

/// Files and folders a role commits (positive pathspecs only; see
/// `exclude_specs` for the exclusions).
pub fn shared_pathspec(role: Role, login: &str) -> Vec<String> {
    if role == Role::Pending {
        return vec![request_rel_path(login)];
    }
    let mut v: Vec<String> = CROSS_REPO_DIRS
        .iter()
        .chain(MANAGED_ROOT_FILES.iter())
        .map(|s| s.to_string())
        .collect();
    v.extend(
        [
            "docs/todo.md",
            "docs/done.md",
            "docs/bug-reports.md",
            "docs/InOutBox",
        ]
        .map(String::from),
    );
    if role == Role::Main {
        v.push(STRUCTURE_REL_PATH.to_string());
        v.push(REQUESTS_REL_DIR.to_string());
        v.push(SKILLS_DIR.to_string());
    }
    v
}

fn exclude_specs(pathspec: &[String]) -> Vec<String> {
    if pathspec.iter().any(|p| p == SKILLS_DIR) {
        SKILLS_EXCLUDES.iter().map(|s| s.to_string()).collect()
    } else {
        Vec::new()
    }
}

/// Repo files that never travel: rule hand-back markers.
fn is_excluded_file(path: &str) -> bool {
    path.starts_with("docs/sdh_skills/")
        && (path.ends_with(".approved") || path.ends_with(".conflict"))
}

// ---------------------------------------------------------------------------
// Small git helpers
// ---------------------------------------------------------------------------

fn git_bytes(
    git: &GitBinary,
    root: &Path,
    args: &[&str],
    paths: &[String],
) -> Result<Vec<u8>, String> {
    let out = spawn_cmd(&git.0)
        .args(args)
        .arg("--")
        .args(paths)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git {} failed to start: {}", args.first().unwrap_or(&""), e))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// `path -> blob sha` of every blob under `paths` at `rev`.
fn ls_tree(
    git: &GitBinary,
    root: &Path,
    rev: &str,
    paths: &[String],
) -> Result<BTreeMap<String, String>, String> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }
    let out = git_bytes(git, root, &["ls-tree", "-r", "-z", rev], paths)?;
    let mut map = BTreeMap::new();
    for rec in split_nul(&out) {
        // `<mode> SP <type> SP <sha> TAB <path>`
        let Some((meta, path)) = rec.split_once('\t') else {
            continue;
        };
        let mut it = meta.split(' ');
        let (_mode, kind, sha) = (it.next(), it.next(), it.next());
        if kind == Some("blob") {
            if let Some(sha) = sha {
                map.insert(path.to_string(), sha.to_string());
            }
        }
    }
    Ok(map)
}

/// Tracked and untracked-but-not-ignored files under `paths` in the work tree
/// (a tracked file deleted from disk is still listed).
fn ls_files(git: &GitBinary, root: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let out = git_bytes(
        git,
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
        paths,
    )?;
    Ok(split_nul(&out))
}

/// The subset of `pathspec` that has at least one file (tracked, or untracked
/// and not ignored). Pathspecs with nothing behind them would make `git add` /
/// `git commit` fail, so they are dropped before committing.
fn active_paths(git: &GitBinary, root: &Path, pathspec: &[String]) -> Result<Vec<String>, String> {
    let files = ls_files(git, root, pathspec)?;
    Ok(pathspec
        .iter()
        .filter(|p| {
            files
                .iter()
                .any(|f| f == *p || f.starts_with(&format!("{}/", p)))
        })
        .cloned()
        .collect())
}

/// Are any of the shared files modified in the work tree (staged, unstaged or
/// untracked)?
fn shared_paths_dirty(git: &GitBinary, root: &Path, pathspec: &[String]) -> Result<bool, String> {
    let mut specs = pathspec.to_vec();
    specs.extend(exclude_specs(pathspec));
    let out = git_bytes(git, root, &["status", "--porcelain", "-z"], &specs)?;
    Ok(!out.is_empty())
}

fn cat_blob(git: &GitBinary, root: &Path, sha: &str) -> Result<Vec<u8>, String> {
    let out = spawn_cmd(&git.0)
        .args(["cat-file", "blob", sha])
        .current_dir(root)
        .output()
        .map_err(|e| format!("git cat-file failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git cat-file failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// Run git with `input` on stdin. The input is written from a thread: git
/// answers as it reads, so a long list could otherwise fill the output pipe
/// while we are still writing.
fn run_with_stdin(
    git: &GitBinary,
    root: &Path,
    args: &[&str],
    input: Vec<u8>,
) -> Result<std::process::Output, String> {
    let mut child = spawn_cmd(&git.0)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("git {} failed to start: {}", args.first().unwrap_or(&""), e))?;
    let mut stdin = child.stdin.take().ok_or("no stdin for git")?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git {} failed: {}", args.first().unwrap_or(&""), e))?;
    let _ = writer.join();
    Ok(out)
}

/// Blob sha each existing regular file would get from `git add` (clean / eol
/// filters applied), in one `git hash-object --stdin-paths` call.
fn hash_files(
    git: &GitBinary,
    root: &Path,
    files: &[String],
) -> Result<HashMap<String, String>, String> {
    if files.is_empty() {
        return Ok(HashMap::new());
    }
    let input = (files.join(
        "
",
    ) + "
")
    .into_bytes();
    let out = run_with_stdin(git, root, &["hash-object", "--stdin-paths"], input)?;
    if !out.status.success() {
        return Err(format!(
            "git hash-object failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let shas: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .collect();
    if shas.len() != files.len() {
        return Err("git hash-object returned an unexpected number of lines".into());
    }
    Ok(files.iter().cloned().zip(shas).collect())
}

/// CRLF -> LF for valid UTF-8 text (an autocrlf checkout must not look changed).
fn normalize(bytes: Vec<u8>) -> Vec<u8> {
    match String::from_utf8(bytes) {
        Ok(s) if s.contains("\r\n") => s.replace("\r\n", "\n").into_bytes(),
        Ok(s) => s.into_bytes(),
        Err(e) => e.into_bytes(),
    }
}

fn read_worktree(root: &Path, rel: &str) -> Option<Vec<u8>> {
    let p = root.join(rel);
    let meta = std::fs::symlink_metadata(&p).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    std::fs::read(&p).ok()
}

fn same_branch(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn local_ref(branch: &str) -> String {
    format!("refs/heads/{}", branch)
}

fn remote_ref(branch: &str) -> String {
    format!("refs/remotes/{}/{}", REMOTE, branch)
}

// ---------------------------------------------------------------------------
// Plumbing: commit the shared files into a branch that is not checked out
// ---------------------------------------------------------------------------

fn commit_message(login: &str) -> (String, String) {
    (
        "chore(sdh-sync): sync shared files".to_string(),
        format!(
            "Shared project files merged and synced by Solo Dev Hub.\n\nSDH-Participant: {}",
            login
        ),
    )
}

/// The blob sha `path` had the last time HEAD's history held it: HEAD's own
/// copy, or the one before the commit that deleted it. `None` when HEAD never
/// had the file.
fn last_version_sha(git: &GitBinary, root: &Path, path: &str) -> Option<String> {
    if let Ok(Some(sha)) = git_sync::blob_sha_at(git, root, "HEAD", path) {
        return Some(sha);
    }
    let out = spawn_cmd(&git.0)
        .args(["rev-list", "-1", "HEAD", "--", path])
        .current_dir(root)
        .output()
        .ok()?;
    let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || commit.is_empty() {
        return None;
    }
    git_sync::blob_sha_at(git, root, &format!("{}^", commit), path)
        .ok()
        .flatten()
}

/// What to do with one file of the target branch.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    Skip,
    Write(Vec<u8>),
    Delete,
}

/// Commit our version of the shared files into `branch` (a local branch that is
/// not checked out). Task files (`todo.md` / `done.md` / `bug-reports.md`) are
/// merged by id (base = the file at the merge-base of HEAD and the branch,
/// ours = the work tree, theirs = the branch). Every other file is a plain
/// three-way copy: the work tree wins only when we changed the file since the
/// merge-base; a file we deleted (present at the merge-base, unchanged on the
/// branch) is deleted from the branch; a file the branch deleted is not
/// resurrected unless we changed it. Files outside `pathspec` are never touched.
fn plumb_into_branch(
    git: &GitBinary,
    root: &Path,
    branch: &str,
    pathspec: &[String],
    login: &str,
) -> Result<BranchCommit, String> {
    let refname = local_ref(branch);
    let base_rev = git_sync::merge_base(git, root, "HEAD", &refname)?;
    let br_tree = ls_tree(git, root, &refname, pathspec)?;
    let base_tree = match &base_rev {
        Some(b) => ls_tree(git, root, b, pathspec)?,
        None => BTreeMap::new(),
    };
    let existing: Vec<String> = ls_files(git, root, pathspec)?
        .into_iter()
        .filter(|f| !is_excluded_file(f))
        .filter(|f| {
            std::fs::symlink_metadata(root.join(f))
                .map(|m| m.file_type().is_file())
                .unwrap_or(false)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let wt_sha = hash_files(git, root, &existing)?;

    let mut all: BTreeSet<&String> = existing.iter().collect();
    all.extend(br_tree.keys());

    let mut changes: Vec<FileChange> = Vec::new();
    for path in all {
        if is_excluded_file(path) {
            continue;
        }
        let wt = wt_sha.get(path).map(String::as_str);
        let br = br_tree.get(path).map(String::as_str);
        let bs = base_tree.get(path).map(String::as_str);
        let plan = match (wt, br) {
            (Some(w), Some(b)) => {
                if w == b {
                    Plan::Skip
                } else if let Some(kind) = kind_for_path(path) {
                    let ours = normalize(read_worktree(root, path).unwrap_or_default());
                    let theirs = normalize(cat_blob(git, root, b)?);
                    let base = match bs {
                        Some(s) => normalize(cat_blob(git, root, s)?),
                        None => Vec::new(),
                    };
                    match merge_shared_bytes(kind, &base, &ours, &theirs) {
                        Ok(m) if m.as_bytes() == theirs.as_slice() => Plan::Skip,
                        Ok(m) => Plan::Write(m.into_bytes()),
                        // Not text: leave the branch's copy alone.
                        Err(_) => Plan::Skip,
                    }
                } else if Some(w) == bs {
                    Plan::Skip
                } else {
                    read_worktree(root, path).map_or(Plan::Skip, Plan::Write)
                }
            }
            (Some(w), None) => {
                if Some(w) == bs {
                    Plan::Skip
                } else {
                    read_worktree(root, path).map_or(Plan::Skip, Plan::Write)
                }
            }
            (None, Some(b)) => {
                // We deleted it iff the branch still holds the version we last
                // had: the merge-base's, or (the histories of two plumbed
                // branches share no recent base) the last one in HEAD's history.
                let ours_before = bs
                    .map(String::from)
                    .or_else(|| last_version_sha(git, root, path));
                if Some(b) == ours_before.as_deref() {
                    Plan::Delete
                } else {
                    Plan::Skip
                }
            }
            (None, None) => Plan::Skip,
        };
        match plan {
            Plan::Skip => {}
            Plan::Write(content) => changes.push(FileChange {
                rel_path: PathBuf::from(path),
                content: Some(content),
            }),
            Plan::Delete => changes.push(FileChange {
                rel_path: PathBuf::from(path),
                content: None,
            }),
        }
    }
    if changes.is_empty() {
        return Ok(BranchCommit::NothingToCommit);
    }
    let (subject, body) = commit_message(login);
    git_sync::commit_files_to_branch(
        git,
        root,
        branch,
        &changes,
        &format!("{}\n\n{}", subject, body),
        SDH_COMMIT_NAME,
        SDH_COMMIT_EMAIL,
    )
}

// ---------------------------------------------------------------------------
// T-000221 hook
// ---------------------------------------------------------------------------

/// Import what arrived by commit into the local database (spec R11 step 1).
///
/// Called by phase A once the recipient branches have been fast-forwarded, for
/// every shared repo. T-000221 fills it in; until then it does nothing, so the
/// shared files that arrive are read by the existing MD <-> DB sync only.
pub fn import_incoming(
    _db: &AppDb,
    _git: &GitBinary,
    _repo_id: i64,
    _root: &Path,
) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Phase A
// ---------------------------------------------------------------------------

struct RepoPass {
    repo: Repository,
    root: PathBuf,
    login: String,
    /// `None` when the repo is not processed (untrusted structure, no login).
    role: Option<Role>,
    structure: Option<ProjectStructure>,
    own_branch: Option<String>,
    recipients: Vec<String>,
    notes: Vec<StatusNote>,
    net_ok: bool,
}

impl RepoPass {
    fn note(&mut self, status: SharedStatus, detail: impl Into<String>) {
        self.notes.push(StatusNote {
            status,
            detail: detail.into(),
        });
    }
}

/// The result of phase A, consumed by `finish`.
pub struct SharedSync {
    passes: Vec<RepoPass>,
    /// Repos the shared path took over: the ordinary auto-commit skips them.
    pub handled: HashSet<i64>,
    outcome: SharedOutcome,
}

impl SharedSync {
    pub fn empty() -> Self {
        SharedSync {
            passes: Vec::new(),
            handled: HashSet::new(),
            outcome: SharedOutcome::default(),
        }
    }
}

fn probe_due(repo_id: i64, root: &Path) -> bool {
    static LAST: OnceLock<Mutex<HashMap<(i64, String), Instant>>> = OnceLock::new();
    let map = LAST.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (repo_id, root.to_string_lossy().into_owned());
    let mut m = map.lock().unwrap();
    let now = Instant::now();
    match m.get(&key) {
        Some(t) if now.duration_since(*t) < PROBE_INTERVAL => false,
        _ => {
            m.insert(key, now);
            true
        }
    }
}

fn repo_issue(code: Code, repo: &Repository) -> SyncIssue {
    SyncIssue::new(code, repo.display_name())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FetchMode {
    /// A repo known to be shared: a failed fetch is a status.
    Always,
    /// Not known to be shared: fetch at most every `PROBE_INTERVAL`, silently.
    Probe,
}

/// Phase A. `git` `None` (no git on this machine) yields an empty pass.
pub fn run_phase_a(db: &AppDb, git: Option<&GitBinary>) -> SharedSync {
    let mut sync = SharedSync::empty();
    let Some(git) = git else {
        return sync;
    };
    let repos = match db.list_all_repos() {
        Ok(r) => r,
        Err(_) => return sync,
    };
    let login = db.github_login();
    for repo in repos {
        let Some(gh) = repo.github_name.clone() else {
            continue;
        };
        let Some(lp) = repo.local_path.clone() else {
            continue;
        };
        let root = PathBuf::from(&lp);
        if !is_git_repo(&root) {
            continue;
        }
        let Ok(state) = db.get_shared_state(repo.id) else {
            continue;
        };
        let owned_by_me = login.as_deref().is_some_and(|l| ss::repo_owner_is(&gh, l));

        if state.shared_mode {
            let Some(login) = login.as_deref() else {
                // Shared mode on, but this install does not know its login.
                let mut pass = new_pass(&repo, &root, "");
                pass.note(SharedStatus::NoLogin, "");
                sync.handled.insert(repo.id);
                sync.passes.push(pass);
                continue;
            };
            sync.handled.insert(repo.id);
            let pass = phase_a_main(db, git, &repo, &root, login, &mut sync.outcome);
            sync.passes.push(pass);
            continue;
        }
        if state.shared_role.as_deref() == Some("main") {
            // Stale cache: the owner switched sharing off.
            let _ = db.set_shared_role(repo.id, None);
            let _ = db.set_shared_status(repo.id, None);
            continue;
        }
        let Some(login) = login.as_deref() else {
            continue;
        };
        if owned_by_me {
            continue;
        }
        let known = state.shared_role.is_some();
        let mode = if known {
            FetchMode::Always
        } else {
            FetchMode::Probe
        };
        if let Some(pass) =
            phase_a_foreign(db, git, &repo, &root, &gh, login, mode, &mut sync.outcome)
        {
            sync.handled.insert(repo.id);
            sync.passes.push(pass);
        }
    }
    sync
}

fn new_pass(repo: &Repository, root: &Path, login: &str) -> RepoPass {
    RepoPass {
        repo: repo.clone(),
        root: root.to_path_buf(),
        login: login.to_string(),
        role: None,
        structure: None,
        own_branch: None,
        recipients: Vec::new(),
        notes: Vec::new(),
        net_ok: true,
    }
}

fn fetch_into(git: &GitBinary, pass: &mut RepoPass) {
    if let Err(e) = git_sync::fetch(git, &pass.root, REMOTE) {
        pass.net_ok = false;
        pass.note(SharedStatus::FetchFailed, e.to_string());
    }
}

fn own_branch_of(db: &AppDb, repo_id: i64) -> Option<String> {
    db.get_autocommit_branch(repo_id)
        .ok()
        .flatten()
        .map(|b| b.trim().to_string())
        .filter(|b| ss::is_valid_branch_name(b))
}

fn phase_a_main(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
    root: &Path,
    login: &str,
    out: &mut SharedOutcome,
) -> RepoPass {
    let mut pass = new_pass(repo, root, login);
    let gh = repo.github_name.clone().unwrap_or_default();
    pass.role = Some(Role::Main);
    let _ = db.set_shared_role(repo.id, Some("main"));
    register_driver(git, &pass, out);
    fetch_into(git, &mut pass);
    match ss::read_structure_file(root, &gh) {
        Ok(s) => pass.structure = s,
        Err(reason) => {
            pass.note(SharedStatus::Untrusted, reason);
            pass.role = None;
            return pass;
        }
    }
    pass.own_branch = own_branch_of(db, repo.id);
    match &pass.own_branch {
        None => pass.note(SharedStatus::NoOwnBranch, ""),
        Some(own) => {
            let mut rec = vec![own.clone()];
            if let Some(s) = &pass.structure {
                rec.extend(s.participants.iter().map(|p| p.branch.clone()));
            }
            pass.recipients = clean_recipients(rec);
        }
    }
    after_fetch(db, git, &mut pass, out);
    pass
}

#[allow(clippy::too_many_arguments)]
fn phase_a_foreign(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
    root: &Path,
    gh: &str,
    login: &str,
    mode: FetchMode,
    out: &mut SharedOutcome,
) -> Option<RepoPass> {
    let mut pass = new_pass(repo, root, login);
    let fetch_now = match mode {
        FetchMode::Always => true,
        FetchMode::Probe => probe_due(repo.id, root),
    };
    if fetch_now {
        let ok = git_sync::fetch(git, root, REMOTE);
        if let Err(e) = ok {
            pass.net_ok = false;
            if mode == FetchMode::Always {
                pass.note(SharedStatus::FetchFailed, e.to_string());
            }
        }
    }
    let pinned = ss::read_shared_import(db, repo.id).pinned_dev_branch;
    let found = match ss::discover_structure(git, root, gh, pinned.as_deref()) {
        Ok(f) => f,
        Err(e) => {
            if mode == FetchMode::Always {
                out.issues.push(
                    repo_issue(Code::ManagedFileFailed, repo)
                        .target(STRUCTURE_REL_PATH)
                        .detail(e),
                );
                return Some(pass);
            }
            return None;
        }
    };
    match found {
        DiscoverOutcome::NotFound => {
            let st = db.get_shared_state(repo.id).ok();
            if st.is_some_and(|s| s.shared_role.is_some() || s.shared_status.is_some()) {
                // Repo is ordinary again (spec R15).
                let _ = db.set_shared_role(repo.id, None);
                let _ = db.set_shared_status(repo.id, None);
            }
            None
        }
        DiscoverOutcome::Untrusted(reason) => {
            pass.note(SharedStatus::Untrusted, reason);
            Some(pass)
        }
        DiscoverOutcome::Found(s, _source) => {
            if let Err(e) = ss::apply_structure_locally(db, repo.id, &s) {
                out.issues.push(
                    repo_issue(Code::ManagedFileFailed, repo)
                        .target(STRUCTURE_REL_PATH)
                        .detail(e),
                );
            }
            if pinned.is_none() {
                let _ = ss::pin_after_accept(db, repo.id, &s);
            }
            let role = ss::derive_role(Some(&s), Some(login));
            let Some(role) = role else {
                return Some(pass);
            };
            let _ = db.set_shared_role(repo.id, Some(role.as_str()));
            pass.role = Some(role);
            register_driver(git, &pass, out);
            pass.own_branch = own_branch_of(db, repo.id)
                .filter(|b| !same_branch(b, &s.main.dev_branch) || role != Role::Pending);
            let main_dev = s.main.dev_branch.clone();
            match (&pass.own_branch, role) {
                (None, _) => pass.note(SharedStatus::NoOwnBranch, ""),
                (Some(own), Role::Contributor) => {
                    pass.recipients = clean_recipients(vec![own.clone(), main_dev]);
                }
                (Some(_), _) => {
                    pass.recipients = clean_recipients(vec![main_dev]);
                }
            }
            if role == Role::Pending {
                pass.note(SharedStatus::PendingApproval, "");
            }
            if role == Role::Contributor {
                remove_own_stale_request(git, &pass);
            }
            pass.structure = Some(s);
            after_fetch(db, git, &mut pass, out);
            Some(pass)
        }
    }
}

fn register_driver(git: &GitBinary, pass: &RepoPass, out: &mut SharedOutcome) {
    if let Err(e) = register_merge_driver(git, &pass.root) {
        out.issues.push(
            repo_issue(Code::ManagedFileFailed, &pass.repo)
                .target(".git/config")
                .detail(e),
        );
    }
}

/// Recipients in order, case-insensitively deduplicated; names git or the
/// structure rules would not accept (and `master` / `main`) are dropped.
fn clean_recipients(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for b in list {
        let b = b.trim().to_string();
        if !ss::is_valid_branch_name(&b) || out.iter().any(|x| same_branch(x, &b)) {
            continue;
        }
        out.push(b);
    }
    out
}

/// A pending helper's own request file is no longer wanted once the helper is a
/// contributor; remove it when git does not track it.
fn remove_own_stale_request(git: &GitBinary, pass: &RepoPass) {
    let rel = request_rel_path(&pass.login);
    let abs = pass.root.join(&rel);
    if !abs.is_file() {
        return;
    }
    let tracked = spawn_cmd(&git.0)
        .args(["ls-files", "--error-unmatch", "--", &rel])
        .current_dir(&pass.root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(true);
    if !tracked {
        let _ = std::fs::remove_file(abs);
    }
}

/// Fast-forward the recipients, then the import hook.
fn after_fetch(db: &AppDb, git: &GitBinary, pass: &mut RepoPass, out: &mut SharedOutcome) {
    let Some(role) = pass.role else { return };
    let cur = current_branch(git, &pass.root).ok().flatten();
    let pathspec = shared_pathspec(role, &pass.login);
    let recipients = pass.recipients.clone();
    for b in &recipients {
        let remote = remote_ref(b);
        match git_sync::rev_parse(git, &pass.root, &remote) {
            Ok(Some(_)) => {}
            _ => continue,
        }
        let is_current = cur.as_deref().is_some_and(|c| same_branch(c, b));
        if is_current {
            match ff_current(git, &pass.root, &remote, &pathspec) {
                Ok(true) => {}
                Ok(false) => pass.note(SharedStatus::NeedsManualMerge, b.clone()),
                Err(e) => out
                    .issues
                    .push(repo_issue(Code::AutocommitFailed, &pass.repo).detail(e)),
            }
        } else {
            match git_sync::fast_forward_ref(git, &pass.root, b, &remote) {
                Ok(FfOutcome::Diverged) => pass.note(SharedStatus::NeedsManualMerge, b.clone()),
                Ok(_) => {}
                Err(e) => out
                    .issues
                    .push(repo_issue(Code::AutocommitFailed, &pass.repo).detail(e)),
            }
        }
    }
    if let Err(e) = import_incoming(db, git, pass.repo.id, &pass.root) {
        out.issues
            .push(repo_issue(Code::AutocommitFailed, &pass.repo).detail(e));
    }
}

/// Bring the CURRENT branch up to `remote`. `Ok(false)` = diverged (needs a
/// manual merge). A branch that is only behind is fast-forwarded when none of
/// the shared files is modified in the work tree; otherwise it waits.
fn ff_current(
    git: &GitBinary,
    root: &Path,
    remote: &str,
    pathspec: &[String],
) -> Result<bool, String> {
    if git_sync::is_ancestor(git, root, remote, "HEAD")? {
        return Ok(true);
    }
    if !git_sync::is_ancestor(git, root, "HEAD", remote)? {
        return Ok(false);
    }
    if shared_paths_dirty(git, root, pathspec)? {
        return Ok(true);
    }
    match git_sync::merge_ff_only(git, root, remote)? {
        FfOutcome::Diverged => Ok(false),
        _ => Ok(true),
    }
}

// ---------------------------------------------------------------------------
// Phase B
// ---------------------------------------------------------------------------

impl SharedSync {
    /// Phase B: write, commit, push and settle the statuses.
    pub fn finish(mut self, db: &AppDb, git: Option<&GitBinary>) -> SharedOutcome {
        let passes = std::mem::take(&mut self.passes);
        let mut out = std::mem::take(&mut self.outcome);
        for mut pass in passes {
            if let (Some(git), Some(_)) = (git, pass.role) {
                phase_b_repo(db, git, &mut pass, &mut out);
            }
            settle_status(db, &pass, &mut out);
        }
        out
    }
}

fn settle_status(db: &AppDb, pass: &RepoPass, out: &mut SharedOutcome) {
    let best = pass.notes.iter().min_by_key(|n| n.status);
    let new = best.map(|n| n.status.as_str());
    let old = db
        .get_shared_state(pass.repo.id)
        .ok()
        .and_then(|s| s.shared_status);
    if old.as_deref() == new {
        return;
    }
    let _ = db.set_shared_status(pass.repo.id, new);
    if let Some(n) = best {
        let mut issue = repo_issue(n.status.issue_code(), &pass.repo);
        if !n.detail.is_empty() {
            issue = issue.detail(&n.detail);
        }
        out.issues.push(issue);
    }
}

fn phase_b_repo(db: &AppDb, git: &GitBinary, pass: &mut RepoPass, out: &mut SharedOutcome) {
    let Some(role) = pass.role else { return };
    let root = pass.root.clone();
    let login = pass.login.clone();

    match role {
        Role::Main => {
            let written = if pass.own_branch.is_some() {
                write_main_structure(db, pass)
            } else {
                Ok(())
            };
            if let Err(e) = written {
                out.issues.push(
                    repo_issue(Code::ManagedFileFailed, &pass.repo)
                        .target(STRUCTURE_REL_PATH)
                        .detail(e),
                );
            }
            if let Some(s) = &pass.structure {
                prune_approved_requests(&root, s);
            }
        }
        Role::Pending => {
            if let (Some(own), true) = (pass.own_branch.clone(), pass.structure.is_some()) {
                let req = JoinRequest {
                    login: login.clone(),
                    branch: own,
                };
                if let Err(e) = ss::write_request_if_changed(&root, &req) {
                    out.issues.push(
                        repo_issue(Code::ManagedFileFailed, &pass.repo)
                            .target(REQUESTS_REL_DIR)
                            .detail(e),
                    );
                }
            }
        }
        Role::Contributor => {}
    }

    if pass.recipients.is_empty() {
        return;
    }
    let pathspec = shared_pathspec(role, &login);
    let active = match active_paths(git, &root, &pathspec) {
        Ok(a) => a,
        Err(e) => {
            out.issues
                .push(repo_issue(Code::AutocommitFailed, &pass.repo).detail(e));
            return;
        }
    };
    let cur = current_branch(git, &root).ok().flatten();
    let head_born = matches!(git_sync::rev_parse(git, &root, "HEAD"), Ok(Some(_)));
    if !head_born {
        return;
    }

    let recipients = pass.recipients.clone();
    let mut committed = false;
    for b in &recipients {
        let is_current = cur.as_deref().is_some_and(|c| same_branch(c, b));
        if is_current {
            if active.is_empty() {
                continue;
            }
            let mut paths: Vec<PathBuf> = active.iter().map(PathBuf::from).collect();
            paths.extend(exclude_specs(&active).into_iter().map(PathBuf::from));
            let (subject, body) = commit_message(&login);
            match commit_paths(
                git,
                &root,
                b,
                &paths,
                &subject,
                Some(&body),
                SDH_COMMIT_NAME,
                SDH_COMMIT_EMAIL,
            ) {
                Ok(CommitOutcome::Committed { .. }) => committed = true,
                Ok(_) => {}
                Err(e) => out
                    .issues
                    .push(repo_issue(Code::AutocommitFailed, &pass.repo).detail(e)),
            }
        } else {
            // The branch must exist locally (phase A creates it from origin).
            if !matches!(git_sync::rev_parse(git, &root, &local_ref(b)), Ok(Some(_))) {
                continue;
            }
            match plumb_into_branch(git, &root, b, &pathspec, &login) {
                Ok(BranchCommit::Committed { .. }) => committed = true,
                // RefMoved: left for the next sync.
                Ok(_) => {}
                Err(e) => out
                    .issues
                    .push(repo_issue(Code::AutocommitFailed, &pass.repo).detail(e)),
            }
        }
    }
    if committed {
        let name = pass.repo.display_name();
        if !out.committed.contains(&name) {
            out.committed.push(name);
        }
    }

    if pass.net_ok {
        push_recipients(git, pass);
    }
}

fn write_main_structure(db: &AppDb, pass: &mut RepoPass) -> Result<(), String> {
    let built = ss::build_structure(db, pass.repo.id, &pass.login, pass.structure.as_ref())?;
    ss::write_structure_if_changed(&pass.root, &built)?;
    pass.structure = Some(built);
    Ok(())
}

/// Request files of people who are participants by now are leftovers; main
/// removes them (the next commit carries the deletion).
fn prune_approved_requests(root: &Path, s: &ProjectStructure) {
    let dir = root.join(REQUESTS_REL_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let known = s
            .participants
            .iter()
            .any(|p| ss::logins_equal(&p.login, stem));
        if known && matches!(e.file_type(), Ok(t) if t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn push_recipients(git: &GitBinary, pass: &mut RepoPass) {
    let root = pass.root.clone();
    let mut to_push: Vec<String> = Vec::new();
    for b in &pass.recipients {
        let Ok(Some(local)) = git_sync::rev_parse(git, &root, &local_ref(b)) else {
            continue;
        };
        let remote = git_sync::rev_parse(git, &root, &remote_ref(b))
            .ok()
            .flatten();
        if remote.as_deref() != Some(local.as_str()) {
            to_push.push(b.clone());
        }
    }
    if to_push.is_empty() {
        return;
    }
    let refs: Vec<&str> = to_push.iter().map(String::as_str).collect();
    match git_sync::push_branches(git, &root, REMOTE, &refs) {
        Err(e) => pass.note(SharedStatus::PushFailed, e.to_string()),
        Ok(results) => {
            for (branch, outcome) in results {
                match outcome {
                    PushOutcome::Pushed | PushOutcome::UpToDate => {}
                    PushOutcome::RejectedNonFastForward => {
                        pass.note(SharedStatus::NeedsManualMerge, branch)
                    }
                    PushOutcome::RejectedProtected(m) => pass.note(SharedStatus::PushRejected, m),
                    PushOutcome::RejectedOther(m) => pass.note(SharedStatus::PushFailed, m),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Commands' logic
// ---------------------------------------------------------------------------

/// Paths probed for a user-section `.gitignore` line that still ignores them.
/// Folders are probed through a file inside, since a folder pattern
/// (`dir/`) only matches something git knows to be a folder.
fn probe_paths() -> Vec<String> {
    let mut v: Vec<String> = SHARED_UNIGNORED_LINES
        .iter()
        .map(|l| {
            if l.ends_with('/') {
                format!("{}sdh-probe.md", l)
            } else {
                l.to_string()
            }
        })
        .collect();
    v.push(STRUCTURE_REL_PATH.to_string());
    v.push(format!("{}/sdh-probe.json", REQUESTS_REL_DIR));
    v
}

fn managed_block_lines(text: &str) -> HashSet<u32> {
    let mut inside = false;
    let mut set = HashSet::new();
    for (i, l) in text.lines().enumerate() {
        let n = (i + 1) as u32;
        if l.contains("solo-dev-hub:begin") {
            inside = true;
        }
        if inside {
            set.insert(n);
        }
        if l.contains("solo-dev-hub:end") {
            inside = false;
        }
    }
    set
}

/// User-section lines of the repo's own `.gitignore` that still ignore a shared
/// path. Lines of the managed block are not reported (the sync rewrites it).
pub fn blocking_ignore_lines(git: &GitBinary, root: &Path) -> Result<Vec<IgnoreLine>, String> {
    let probes = probe_paths();
    let mut input = Vec::new();
    for p in &probes {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    let out = run_with_stdin(
        git,
        root,
        &["check-ignore", "-v", "-z", "--no-index", "--stdin"],
        input,
    )?;
    match out.status.code() {
        Some(0) | Some(1) => {}
        _ => {
            return Err(format!(
                "git check-ignore failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
    let text = std::fs::read_to_string(root.join(".gitignore")).unwrap_or_default();
    let managed = managed_block_lines(&text);
    let lines: Vec<&str> = text.lines().collect();
    let fields = split_nul(&out.stdout);
    let mut found: BTreeMap<u32, String> = BTreeMap::new();
    for rec in fields.chunks(4) {
        if rec.len() < 4 {
            break;
        }
        let (source, line_no, pattern) = (&rec[0], &rec[1], &rec[2]);
        if source != ".gitignore" || pattern.starts_with('!') {
            continue;
        }
        let Ok(n) = line_no.parse::<u32>() else {
            continue;
        };
        if managed.contains(&n) {
            continue;
        }
        let text = lines
            .get((n - 1) as usize)
            .map(|l| l.to_string())
            .unwrap_or_else(|| pattern.clone());
        found.insert(n, text);
    }
    Ok(found
        .into_iter()
        .map(|(line_no, text)| IgnoreLine { line_no, text })
        .collect())
}

fn repo_root(repo: &Repository) -> Result<PathBuf, String> {
    let p = repo
        .local_path
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "the repository has no local path".to_string())?;
    let root = PathBuf::from(p);
    if !is_git_repo(&root) {
        return Err("the repository is not a git checkout".into());
    }
    Ok(root)
}

pub fn shared_mode_preflight(
    db: &AppDb,
    git: &GitBinary,
    repo_id: i64,
) -> Result<SharedPreflight, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    let branch = db
        .get_autocommit_branch(repo_id)
        .map_err(|e| e.to_string())?
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());
    Ok(SharedPreflight {
        blocking_lines: blocking_ignore_lines(git, &root)?,
        missing_project: repo.project_id.is_none(),
        missing_dev_branch: branch.is_none(),
    })
}

/// Remove exactly the given 1-based lines (and only when they still read the
/// same) from `<root>/.gitignore`.
fn remove_ignore_lines(root: &Path, lines: &[IgnoreLine]) -> Result<(), String> {
    let path = root.join(".gitignore");
    let text = std::fs::read_to_string(&path).map_err(|e| format!(".gitignore: {}", e))?;
    let drop: HashMap<u32, &str> = lines.iter().map(|l| (l.line_no, l.text.as_str())).collect();
    let mut out = String::with_capacity(text.len());
    for (i, l) in text.split_inclusive('\n').enumerate() {
        let n = (i + 1) as u32;
        if let Some(expect) = drop.get(&n) {
            if l.trim_end_matches(['\r', '\n']) == *expect {
                continue;
            }
        }
        out.push_str(l);
    }
    std::fs::write(&path, out).map_err(|e| format!(".gitignore: {}", e))
}

pub fn enable_shared_mode(
    db: &AppDb,
    git: &GitBinary,
    repo_id: i64,
    remove_lines: bool,
) -> Result<(), String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    let login = db.github_login().ok_or_else(|| {
        "the GitHub login of this install is unknown - save the token again".to_string()
    })?;
    let gh = repo
        .github_name
        .clone()
        .ok_or_else(|| "shared mode needs a GitHub repository".to_string())?;
    let existing = ss::read_structure_file(&root, &gh)?;
    // Validates: owner, project, dev branch.
    let built = ss::build_structure(db, repo_id, &login, existing.as_ref())?;
    let blocking = blocking_ignore_lines(git, &root)?;
    if !blocking.is_empty() {
        if !remove_lines {
            return Err(format!(
                "{} .gitignore line(s) still ignore shared files",
                blocking.len()
            ));
        }
        remove_ignore_lines(&root, &blocking)?;
    }
    db.set_shared_mode(repo_id, true)
        .map_err(|e| e.to_string())?;
    db.set_shared_role(repo_id, Some("main"))
        .map_err(|e| e.to_string())?;
    ss::write_structure_if_changed(&root, &built)?;
    register_merge_driver(git, &root)?;
    Ok(())
}

/// Switch shared mode off (spec R15). The deletion of `.sdh/project.json` and
/// `.sdh/requests/*` is committed and pushed HERE, on the dev branch, rather
/// than by the next sync: once the switch is off the sync no longer treats the
/// repo as shared, so nothing there would commit or push the deletion.
pub fn disable_shared_mode(
    db: &AppDb,
    git: &GitBinary,
    repo_id: i64,
) -> Result<SharedDisableResult, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    db.set_shared_mode(repo_id, false)
        .map_err(|e| e.to_string())?;
    db.set_shared_role(repo_id, None)
        .map_err(|e| e.to_string())?;
    db.set_shared_status(repo_id, None)
        .map_err(|e| e.to_string())?;

    // Everything git knows under the two paths, before the files are removed.
    let sdh = vec![STRUCTURE_REL_PATH.to_string(), REQUESTS_REL_DIR.to_string()];
    let tracked: Vec<String> = ls_files(git, &root, &sdh)?;
    let _ = std::fs::remove_file(root.join(STRUCTURE_REL_PATH));
    if let Ok(entries) = std::fs::read_dir(root.join(REQUESTS_REL_DIR)) {
        for e in entries.flatten() {
            if matches!(e.file_type(), Ok(t) if t.is_file()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let _ = std::fs::remove_dir(root.join(REQUESTS_REL_DIR));
    let _ = std::fs::remove_dir(root.join(".sdh"));

    let dev = db
        .get_autocommit_branch(repo_id)
        .map_err(|e| e.to_string())?
        .map(|b| b.trim().to_string())
        .filter(|b| ss::is_valid_branch_name(b));
    let Some(dev) = dev else {
        return Ok(SharedDisableResult {
            pushed: false,
            note: Some("no dev branch is set: the deletion of .sdh/ is not committed".into()),
        });
    };
    let login = db.github_login().unwrap_or_default();
    let (_, body) = commit_message(&login);
    let subject = "chore(sdh-sync): remove shared project structure".to_string();
    let cur = current_branch(git, &root).ok().flatten();
    let committed = if cur.as_deref().is_some_and(|c| same_branch(c, &dev)) {
        // Only files git knows: a pathspec with nothing behind it fails the add.
        let paths: Vec<PathBuf> = tracked.iter().map(PathBuf::from).collect();
        if paths.is_empty() {
            Ok(false)
        } else {
            commit_paths(
                git,
                &root,
                &dev,
                &paths,
                &subject,
                Some(&body),
                SDH_COMMIT_NAME,
                SDH_COMMIT_EMAIL,
            )
            .map(|o| matches!(o, CommitOutcome::Committed { .. }))
        }
    } else if matches!(
        git_sync::rev_parse(git, &root, &local_ref(&dev)),
        Ok(Some(_))
    ) {
        let in_branch = ls_tree(git, &root, &local_ref(&dev), &sdh)?;
        let changes: Vec<FileChange> = in_branch
            .keys()
            .map(|p| FileChange {
                rel_path: PathBuf::from(p),
                content: None,
            })
            .collect();
        git_sync::commit_files_to_branch(
            git,
            &root,
            &dev,
            &changes,
            &format!("{}\n\n{}", subject, body),
            SDH_COMMIT_NAME,
            SDH_COMMIT_EMAIL,
        )
        .map(|o| matches!(o, BranchCommit::Committed { .. }))
    } else {
        Ok(false)
    };
    let committed = committed?;
    if !committed {
        return Ok(SharedDisableResult {
            pushed: false,
            note: Some("nothing to commit".into()),
        });
    }
    match git_sync::push_branches(git, &root, REMOTE, &[dev.as_str()]) {
        Ok(results) => {
            let ok = results
                .iter()
                .all(|(_, o)| matches!(o, PushOutcome::Pushed | PushOutcome::UpToDate));
            Ok(SharedDisableResult {
                pushed: ok,
                note: if ok {
                    None
                } else {
                    Some(format!("push of {} was refused: {:?}", dev, results))
                },
            })
        }
        Err(e) => Ok(SharedDisableResult {
            pushed: false,
            note: Some(format!("push failed: {}", e)),
        }),
    }
}

/// Main: approve the join request of `login`. The structure file gains the
/// participant (with a number band) and the request file is deleted; the next
/// sync commits both.
pub fn approve_join_request(
    db: &AppDb,
    repo_id: i64,
    login: &str,
) -> Result<ProjectStructure, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let state = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    if !state.shared_mode {
        return Err("only the owner of a shared repo can approve join requests".into());
    }
    let root = repo_root(&repo)?;
    let gh = repo
        .github_name
        .clone()
        .ok_or("the repository has no GitHub name")?;
    let me = db
        .github_login()
        .ok_or_else(|| "the GitHub login of this install is unknown".to_string())?;
    let existing = ss::read_structure_file(&root, &gh)?;
    let base = ss::build_structure(db, repo_id, &me, existing.as_ref())?;
    let req = ss::list_requests(&root, &gh)
        .into_iter()
        .find(|r| ss::logins_equal(&r.login, login))
        .ok_or_else(|| format!("no join request from {}", login))?;
    let next = ss::approve_request(&base, &req)?;
    ss::write_structure_if_changed(&root, &next)?;
    if let Ok(entries) = std::fs::read_dir(root.join(REQUESTS_REL_DIR)) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name
                .strip_suffix(".json")
                .is_some_and(|s| ss::logins_equal(s, &req.login))
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(next)
}

/// Contributor: trust the owner's current dev branch again (after the pin
/// refused a changed one).
pub fn accept_new_main_branch(db: &AppDb, repo_id: i64) -> Result<(), String> {
    let mut v = ss::read_shared_import(db, repo_id);
    v.pinned_dev_branch = None;
    ss::write_shared_import(db, repo_id, &v)?;
    db.set_shared_status(repo_id, None)
        .map_err(|e| e.to_string())
}

/// The "merge" button: merge `origin/<current branch>` into the current branch
/// (the registered driver merges the shared files by id). A conflict in code is
/// aborted and reported. The `needs_manual_merge` status is cleared only after
/// a real merge; the next sync recomputes it either way.
pub fn merge_shared_upstream(
    db: &AppDb,
    git: &GitBinary,
    repo_id: i64,
) -> Result<SharedMergeResult, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    let cur = current_branch(git, &root)?
        .ok_or_else(|| "HEAD is detached: check out a branch first".to_string())?;
    register_merge_driver(git, &root)?;
    let upstream = remote_ref(&cur);
    let outcome = git_sync::merge_or_abort(git, &root, &upstream)?;
    Ok(match outcome {
        MergeOutcome::UpToDate => SharedMergeResult {
            outcome: "up_to_date".into(),
            files: vec![],
        },
        MergeOutcome::Merged => {
            let st = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
            if st.shared_status.as_deref() == Some(SharedStatus::NeedsManualMerge.as_str()) {
                let _ = db.set_shared_status(repo_id, None);
            }
            SharedMergeResult {
                outcome: "merged".into(),
                files: vec![],
            }
        }
        MergeOutcome::Conflict { files } => SharedMergeResult {
            outcome: "conflict".into(),
            files,
        },
    })
}

pub fn list_join_requests(db: &AppDb, repo_id: i64) -> Result<Vec<JoinRequest>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let (Some(lp), Some(gh)) = (repo.local_path.as_deref(), repo.github_name.as_deref()) else {
        return Ok(Vec::new());
    };
    Ok(ss::list_requests(Path::new(lp), gh))
}

/// The trusted structure of a shared repo for the UI: the work-tree file, or
/// (for a helper whose join is pending) the owner's copy in the remote refs.
pub fn get_repo_structure(
    db: &AppDb,
    git: Option<&GitBinary>,
    repo_id: i64,
) -> Result<Option<ProjectStructure>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let (Some(lp), Some(gh)) = (repo.local_path.clone(), repo.github_name.clone()) else {
        return Ok(None);
    };
    let root = PathBuf::from(lp);
    if let Some(s) = ss::read_structure_file(&root, &gh)? {
        return Ok(Some(s));
    }
    let (Some(git), true) = (git, is_git_repo(&root)) else {
        return Ok(None);
    };
    let pinned = ss::read_shared_import(db, repo_id).pinned_dev_branch;
    let owner_is_me = db
        .github_login()
        .is_some_and(|l| ss::repo_owner_is(&gh, &l));
    if owner_is_me {
        return Ok(None);
    }
    Ok(
        match ss::discover_structure(git, &root, &gh, pinned.as_deref())? {
            DiscoverOutcome::Found(s, _) => Some(s),
            _ => None,
        },
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two installs (an owner and a helper) on two clones of one bare remote, with
/// two in-memory databases, driven in one process. Reused by the integration
/// test of T-000225.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    pub fn gb() -> GitBinary {
        GitBinary(PathBuf::from("git"))
    }

    pub fn g(dir: &Path, args: &[&str]) -> String {
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

    pub fn configure(dir: &Path) {
        g(dir, &["config", "user.email", "t@example.com"]);
        g(dir, &["config", "user.name", "T"]);
        g(dir, &["config", "core.autocrlf", "false"]);
        g(dir, &["config", "commit.gpgsign", "false"]);
    }

    pub fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, content).unwrap();
    }

    pub fn task(id: &str) -> String {
        format!("- [ ] {} | task {} | 1 | low | open | 2026-10-01\n", id, id)
    }

    pub fn todo(ids: &[&str]) -> String {
        let mut s = String::from("## v1.0.0 \u{2014} first\n\n");
        for id in ids {
            s.push_str(&task(id));
        }
        s
    }

    pub struct TwoInstalls {
        pub _tmp: TempDir,
        pub origin: PathBuf,
        pub main_dir: PathBuf,
        pub helper_dir: PathBuf,
        pub main_db: AppDb,
        pub helper_db: AppDb,
        pub main_repo: i64,
        pub helper_repo: i64,
    }

    impl TwoInstalls {
        /// A file of a ref in the bare remote, `None` when absent.
        pub fn remote_file(&self, rev: &str, path: &str) -> Option<String> {
            let out = Command::new("git")
                .args(["show", &format!("{}:{}", rev, path)])
                .current_dir(&self.origin)
                .output()
                .unwrap();
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        }

        pub fn remote_rev(&self, rev: &str) -> Option<String> {
            let out = Command::new("git")
                .args(["rev-parse", "--verify", "--quiet", rev])
                .current_dir(&self.origin)
                .output()
                .unwrap();
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        }

        pub fn sync_main(&self) -> SharedOutcome {
            sync_once(&self.main_db)
        }

        pub fn sync_helper(&self) -> SharedOutcome {
            sync_once(&self.helper_db)
        }

        pub fn main_status(&self) -> Option<String> {
            self.main_db
                .get_shared_state(self.main_repo)
                .unwrap()
                .shared_status
        }

        pub fn helper_status(&self) -> Option<String> {
            self.helper_db
                .get_shared_state(self.helper_repo)
                .unwrap()
                .shared_status
        }

        /// Owner switches sharing on and syncs once (structure pushed).
        pub fn enable_main(&self) {
            enable_shared_mode(&self.main_db, &gb(), self.main_repo, false).unwrap();
            self.sync_main();
        }

        /// Owner enabled sharing, the helper joined and the owner approved:
        /// the helper is a contributor with the band 500000, on `helper-dev`.
        /// Fast path: the request and the approval are made by hand, with one
        /// sync (the helper's) instead of the six of the full flow.
        pub fn make_participant(&self) {
            enable_shared_mode(&self.main_db, &gb(), self.main_repo, false).unwrap();
            write(
                &self.main_dir,
                ".sdh/requests/helper.json",
                "{\"login\":\"helper\",\"branch\":\"helper-dev\"}
",
            );
            approve_join_request(&self.main_db, self.main_repo, "helper").unwrap();
            g(&self.main_dir, &["add", ".sdh"]);
            g(&self.main_dir, &["commit", "-q", "-m", "structure"]);
            g(&self.main_dir, &["push", "-q", "origin", "dev"]);
            self.sync_helper(); // contributor now
        }

        /// The same state reached through real syncs of both installs.
        pub fn make_participant_via_syncs(&self) {
            self.enable_main();
            self.sync_helper(); // pending: the request lands in origin/dev
            self.sync_main(); // the request arrives
            approve_join_request(&self.main_db, self.main_repo, "helper").unwrap();
            self.sync_main(); // approval committed + pushed
            self.sync_helper(); // contributor now
        }
    }

    pub fn sync_once(db: &AppDb) -> SharedOutcome {
        let git = gb();
        run_phase_a(db, Some(&git)).finish(db, Some(&git))
    }

    pub fn codes(out: &SharedOutcome) -> Vec<Code> {
        out.issues.iter().map(|i| i.code).collect()
    }

    pub fn two_installs() -> TwoInstalls {
        let tmp = TempDir::new().unwrap();
        let origin = tmp.path().join("origin.git");
        fs::create_dir(&origin).unwrap();
        g(&origin, &["init", "--bare", "-q"]);
        g(&origin, &["symbolic-ref", "HEAD", "refs/heads/dev"]);

        let main_dir = tmp.path().join("main");
        fs::create_dir(&main_dir).unwrap();
        g(&main_dir, &["init", "-q"]);
        g(&main_dir, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        configure(&main_dir);
        g(
            &main_dir,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        write(&main_dir, "README.md", "hello\n");
        g(&main_dir, &["add", "."]);
        g(&main_dir, &["commit", "-q", "-m", "init"]);
        g(&main_dir, &["branch", "master"]);
        g(&main_dir, &["push", "-q", "origin", "dev", "master"]);

        let helper_dir = tmp.path().join("helper");
        g(
            tmp.path(),
            &[
                "clone",
                "-q",
                "-b",
                "dev",
                origin.to_str().unwrap(),
                helper_dir.to_str().unwrap(),
            ],
        );
        configure(&helper_dir);
        g(&helper_dir, &["checkout", "-q", "-b", "helper-dev"]);

        let main_db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        main_db.set_setting("github_login", "Owner").unwrap();
        let project = main_db.create_project("Acme", None, "standard").unwrap();
        main_db
            .upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let main_repo = main_db.get_repository_by_name("Owner/api").unwrap().id;
        main_db
            .set_repo_local_path(main_repo, Some(main_dir.to_str().unwrap()))
            .unwrap();
        main_db
            .assign_repository(main_repo, Some(project.id), Some("server"))
            .unwrap();
        main_db
            .set_autocommit_branch(main_repo, Some("dev"))
            .unwrap();

        let helper_db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        helper_db.set_setting("github_login", "helper").unwrap();
        helper_db
            .upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let helper_repo = helper_db.get_repository_by_name("Owner/api").unwrap().id;
        helper_db
            .set_repo_local_path(helper_repo, Some(helper_dir.to_str().unwrap()))
            .unwrap();
        helper_db
            .set_autocommit_branch(helper_repo, Some("helper-dev"))
            .unwrap();

        TwoInstalls {
            _tmp: tmp,
            origin,
            main_dir,
            helper_dir,
            main_db,
            helper_db,
            main_repo,
            helper_repo,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::fs;

    // ----- managed blocks -------------------------------------------------

    const GITIGNORE_TMPL: &str = include_str!("../../templates/_global/.gitignore.tmpl");
    const GITATTRIBUTES_TMPL: &str = include_str!("../../templates/_global/.gitattributes.tmpl");

    #[test]
    fn gitignore_block_for_a_shared_repo_drops_exactly_the_shared_paths() {
        assert_eq!(
            gitignore_template_for(GITIGNORE_TMPL, false),
            GITIGNORE_TMPL
        );
        let shared = gitignore_template_for(GITIGNORE_TMPL, true);
        for line in SHARED_UNIGNORED_LINES {
            assert!(
                GITIGNORE_TMPL.lines().any(|l| l.trim() == *line),
                "template lacks {}",
                line
            );
            assert!(
                !shared.lines().any(|l| l.trim() == *line),
                "{} must not be ignored in a shared repo",
                line
            );
        }
        for kept in [
            ".env",
            "/CLAUDE.*",
            ".claude/",
            "docs/project.md",
            "docs/codex/",
            "docs/superpowers/",
        ] {
            assert!(
                shared.lines().any(|l| l.trim() == kept),
                "{} must stay",
                kept
            );
        }
    }

    #[test]
    fn gitattributes_block_gets_merge_sdh_only_when_shared() {
        assert_eq!(
            gitattributes_template_for(GITATTRIBUTES_TMPL, false),
            GITATTRIBUTES_TMPL
        );
        let shared = gitattributes_template_for(GITATTRIBUTES_TMPL, true);
        for line in SHARED_ATTRIBUTE_LINES {
            assert!(shared.lines().any(|l| l == *line), "{}", line);
        }
        assert!(shared.contains("* text=auto eol=lf"));
    }

    #[test]
    fn rendered_gitignore_block_is_stable_on_rewrite() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tpl = gitignore_template_for(GITIGNORE_TMPL, true);
        assert!(crate::sync::sync_gitignore_section(&tpl, tmp.path()).unwrap());
        let first = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(!first.contains("docs/todo.md"));
        assert!(first.contains("docs/codex/"));
        // The other install renders the same text: nothing is rewritten.
        assert!(!crate::sync::sync_gitignore_section(&tpl, tmp.path()).unwrap());
        assert_eq!(
            first,
            fs::read_to_string(tmp.path().join(".gitignore")).unwrap()
        );
    }

    #[test]
    fn shared_state_decides_sharedness() {
        let st = |mode, role: Option<&str>| RepoSharedState {
            repo_id: 1,
            shared_mode: mode,
            shared_role: role.map(String::from),
            shared_status: None,
        };
        assert!(!is_shared_state(&st(false, None)));
        assert!(is_shared_state(&st(true, None)));
        assert!(is_shared_state(&st(false, Some("contributor"))));
        assert!(is_shared_state(&st(false, Some("pending"))));
    }

    // ----- pathspec per role ------------------------------------------------

    #[test]
    fn pathspec_per_role() {
        let main = shared_pathspec(Role::Main, "Owner");
        for p in [
            "docs/todo.md",
            "docs/done.md",
            "docs/bug-reports.md",
            "docs/InOutBox",
            "docs/server-api",
            "docs/backend-requirements",
            ".gitignore",
            ".gitattributes",
            ".sdh/project.json",
            ".sdh/requests",
            "docs/sdh_skills",
        ] {
            assert!(main.iter().any(|x| x == p), "main lacks {}", p);
        }
        assert_eq!(exclude_specs(&main).len(), 2);

        let contributor = shared_pathspec(Role::Contributor, "helper");
        assert!(contributor.iter().any(|x| x == "docs/todo.md"));
        assert!(!contributor.iter().any(|x| x.starts_with(".sdh")));
        assert!(!contributor.iter().any(|x| x == "docs/sdh_skills"));
        assert!(exclude_specs(&contributor).is_empty());

        assert_eq!(
            shared_pathspec(Role::Pending, "helper"),
            vec![".sdh/requests/helper.json".to_string()]
        );
    }

    // ----- owner side ---------------------------------------------------------

    #[test]
    fn main_commits_the_current_branch_and_pushes() {
        let fx = two_installs();
        fx.enable_main();
        assert!(fx
            .remote_file("dev", ".sdh/project.json")
            .unwrap()
            .contains("\"Owner\""));

        write(&fx.main_dir, "docs/todo.md", &todo(&["T-000001"]));
        let out = fx.sync_main();
        assert_eq!(out.committed, vec!["api".to_string()]);
        assert!(fx
            .remote_file("dev", "docs/todo.md")
            .unwrap()
            .contains("T-000001"));
        // The commit carries the SDH author and the participant trailer.
        let body = g(&fx.main_dir, &["log", "-1", "--format=%an|%B"]);
        assert!(
            body.starts_with("Solo Dev Hub|chore(sdh-sync): sync shared files"),
            "{}",
            body
        );
        assert!(body.contains("SDH-Participant: Owner"), "{}", body);
        // Nothing left to do: a second sync commits nothing and reports nothing.
        let again = fx.sync_main();
        assert!(again.committed.is_empty());
        assert!(again.issues.is_empty(), "{:?}", again.issues);
        // master was never touched.
        assert_eq!(
            fx.remote_rev("master").unwrap(),
            g(&fx.main_dir, &["rev-parse", "master"])
        );
    }

    #[test]
    fn main_plumbs_a_merged_todo_into_a_participant_branch() {
        let fx = two_installs();
        fx.make_participant();
        // The helper adds a task on its own branch.
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.sync_helper();
        assert!(fx
            .remote_file("helper-dev", "docs/todo.md")
            .unwrap()
            .contains("T-500000"));
        assert!(fx
            .remote_file("dev", "docs/todo.md")
            .unwrap()
            .contains("T-500000"));
        fx.sync_main(); // main's dev fast-forwards to the helper's task
        assert!(fs::read_to_string(fx.main_dir.join("docs/todo.md"))
            .unwrap()
            .contains("T-500000"));
        // Main also wrote the structure file into the participant's branch; the
        // helper's next sync fast-forwards onto it (its files are not modified).
        fx.sync_helper();
        assert!(fx.helper_dir.join(".sdh/project.json").exists());
        assert_eq!(fx.helper_status(), None);

        // The helper adds one more and pushes; main adds its own, unsynced.
        write(
            &fx.helper_dir,
            "docs/todo.md",
            &todo(&["T-500000", "T-500001"]),
        );
        fx.sync_helper();
        write(
            &fx.main_dir,
            "docs/todo.md",
            &todo(&["T-000001", "T-500000"]),
        );
        let out = fx.sync_main();
        assert!(out.committed.contains(&"api".to_string()));

        let helper_todo = fx.remote_file("helper-dev", "docs/todo.md").unwrap();
        for id in ["T-000001", "T-500000", "T-500001"] {
            assert!(
                helper_todo.contains(id),
                "participant branch lacks {}:\n{}",
                id,
                helper_todo
            );
        }
        // Main's own dev diverged from the helper's push to it: manual merge.
        assert_eq!(fx.main_status().as_deref(), Some("needs_manual_merge"));
        assert!(codes(&out).contains(&Code::SharedNeedsManualMerge));
        // Neither checkout was touched by the plumbing.
        assert_eq!(
            g(&fx.helper_dir, &["branch", "--show-current"]),
            "helper-dev"
        );
        assert_eq!(g(&fx.main_dir, &["branch", "--show-current"]), "dev");
    }

    // ----- contributor side -----------------------------------------------------

    #[test]
    fn contributor_commits_own_branch_and_mains_dev_never_master() {
        let fx = two_installs();
        fx.make_participant();
        let master = fx.remote_rev("master").unwrap();
        assert_eq!(
            fx.helper_db
                .get_shared_state(fx.helper_repo)
                .unwrap()
                .shared_role
                .as_deref(),
            Some("contributor")
        );

        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        write(&fx.helper_dir, "docs/server-api/api.md", "# API\n");
        write(&fx.helper_dir, "src/lib.rs", "// code\n");
        let out = fx.sync_helper();
        assert_eq!(out.committed, vec!["api".to_string()]);
        for branch in ["helper-dev", "dev"] {
            assert!(
                fx.remote_file(branch, "docs/todo.md")
                    .unwrap()
                    .contains("T-500000"),
                "{}",
                branch
            );
            assert_eq!(
                fx.remote_file(branch, "docs/server-api/api.md").as_deref(),
                Some("# API\n"),
                "{}",
                branch
            );
            assert_eq!(
                fx.remote_file(branch, "src/lib.rs"),
                None,
                "code must never travel ({})",
                branch
            );
        }
        assert_eq!(fx.remote_rev("master").unwrap(), master);
        assert!(fx.helper_status().is_none(), "{:?}", fx.helper_status());
        // The local code file is still just an untracked file.
        assert!(g(&fx.helper_dir, &["status", "--porcelain"]).contains("src/"));
    }

    #[test]
    fn contributor_without_an_own_branch_commits_nothing() {
        let fx = two_installs();
        fx.make_participant();
        fx.helper_db
            .set_autocommit_branch(fx.helper_repo, None)
            .unwrap();
        let dev_before = fx.remote_rev("dev").unwrap();
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        let out = fx.sync_helper();
        assert!(out.committed.is_empty());
        assert_eq!(fx.remote_rev("dev").unwrap(), dev_before);
        assert_eq!(fx.helper_status().as_deref(), Some("no_own_branch"));
        assert!(codes(&out).contains(&Code::SharedNoOwnBranch));
    }

    #[test]
    fn pending_helper_sends_only_its_request_to_mains_dev() {
        let fx = two_installs();
        fx.enable_main();
        let before = fx.remote_rev("dev").unwrap();
        let head_before = g(&fx.helper_dir, &["rev-parse", "HEAD"]);
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        write(&fx.helper_dir, "docs/server-api/api.md", "# API\n");
        let out = fx.sync_helper();

        let changed = g(&fx.origin, &["diff", "--name-only", &before, "dev"]);
        assert_eq!(
            changed.lines().collect::<Vec<_>>(),
            vec![".sdh/requests/helper.json"]
        );
        assert!(fx
            .remote_file("dev", ".sdh/requests/helper.json")
            .unwrap()
            .contains("helper-dev"));
        assert_eq!(fx.remote_rev("helper-dev"), None);
        assert_eq!(g(&fx.helper_dir, &["rev-parse", "HEAD"]), head_before);
        assert_eq!(fx.helper_status().as_deref(), Some("pending_approval"));
        assert!(codes(&out).contains(&Code::SharedPendingApproval));
        // Reported once: the next sync raises nothing new.
        assert!(fx.sync_helper().issues.is_empty());
        // The helper joined the project locally from the owner's structure.
        let repo = fx.helper_db.get_repository(fx.helper_repo).unwrap();
        assert_eq!(repo.role.as_deref(), Some("server"));
        assert!(repo.project_id.is_some());
    }

    #[test]
    fn approval_makes_the_pending_helper_a_contributor() {
        let fx = two_installs();
        fx.make_participant_via_syncs();
        let s = get_repo_structure(&fx.main_db, Some(&gb()), fx.main_repo)
            .unwrap()
            .unwrap();
        assert_eq!(s.participants.len(), 1);
        assert_eq!(s.participants[0].login, "helper");
        assert_eq!(s.participants[0].band_start, 500_000);
        // The request file is gone from main's checkout and from the remote.
        assert!(!fx.main_dir.join(".sdh/requests/helper.json").exists());
        assert_eq!(fx.remote_file("dev", ".sdh/requests/helper.json"), None);
        assert!(list_join_requests(&fx.main_db, fx.main_repo)
            .unwrap()
            .is_empty());
        // The helper's leftover request file is cleaned up and its status is clear.
        assert!(!fx.helper_dir.join(".sdh/requests/helper.json").exists());
        assert_eq!(fx.helper_status(), None);
    }

    // ----- statuses ---------------------------------------------------------------

    #[test]
    fn protected_branch_push_is_reported_once() {
        let fx = two_installs();
        fx.make_participant();
        // An `update` hook decides per ref (a pre-receive hook would decline
        // every ref of the push at once).
        let hook = fx.origin.join("hooks/update");
        fs::write(
            &hook,
            "#!/bin/sh\nif [ \"$1\" = \"refs/heads/dev\" ]; then\n  echo \"error: GH006: Protected branch update failed for $1\" >&2\n  exit 1\nfi\nexit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dev_before = fx.remote_rev("dev").unwrap();

        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        let first = fx.sync_helper();
        assert_eq!(fx.helper_status().as_deref(), Some("push_rejected"));
        assert_eq!(
            codes(&first)
                .iter()
                .filter(|c| **c == Code::SharedPushRejected)
                .count(),
            1,
            "{:?}",
            first.issues
        );
        assert!(first
            .issues
            .iter()
            .any(|i| i.detail.as_deref().is_some_and(|d| d.contains("GH006"))));
        // The own branch still went through; main's dev did not move.
        assert!(fx.remote_file("helper-dev", "docs/todo.md").is_some());
        assert_eq!(fx.remote_rev("dev").unwrap(), dev_before);

        let second = fx.sync_helper();
        assert!(
            !codes(&second).contains(&Code::SharedPushRejected),
            "toast on appearance only: {:?}",
            second.issues
        );
        assert_eq!(fx.helper_status().as_deref(), Some("push_rejected"));
    }

    #[test]
    fn diverged_current_branch_needs_a_manual_merge_and_the_button_merges() {
        let fx = two_installs();
        fx.enable_main();
        fx.sync_helper(); // pending: pushes its request to origin/dev
                          // Main commits code locally, unpushed.
        write(&fx.main_dir, "src.rs", "fn main() {}\n");
        g(&fx.main_dir, &["add", "src.rs"]);
        g(&fx.main_dir, &["commit", "-q", "-m", "code"]);
        let out = fx.sync_main();
        assert_eq!(fx.main_status().as_deref(), Some("needs_manual_merge"));
        assert!(codes(&out).contains(&Code::SharedNeedsManualMerge));
        assert!(!fx.main_dir.join(".sdh/requests/helper.json").exists());
        // Nothing was forced.
        assert!(fx.remote_file("dev", "src.rs").is_none());

        let res = merge_shared_upstream(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(res.outcome, "merged");
        assert!(fx.main_dir.join(".sdh/requests/helper.json").exists());
        assert_eq!(fx.main_status(), None);
        // The next sync pushes the merge.
        fx.sync_main();
        assert!(fx.remote_file("dev", "src.rs").is_some());
        assert!(fx.remote_file("dev", ".sdh/requests/helper.json").is_some());
    }

    #[test]
    fn untrusted_structure_sets_a_status_and_commits_nothing() {
        let fx = two_installs();
        // A structure that names somebody else as the repo owner.
        write(
            &fx.main_dir,
            ".sdh/project.json",
            r#"{"format":1,"project":{"name":"Acme","type":"standard"},"main":{"login":"Mallory","dev_branch":"dev"},"repos":[{"github":"Owner/api","role":"server"}],"microservices":[],"participants":[]}"#,
        );
        g(&fx.main_dir, &["add", ".sdh"]);
        g(&fx.main_dir, &["commit", "-q", "-m", "structure"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
        let dev_before = fx.remote_rev("dev").unwrap();
        let head_before = g(&fx.helper_dir, &["rev-parse", "HEAD"]);

        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        let out = fx.sync_helper();
        assert_eq!(fx.helper_status().as_deref(), Some("shared_untrusted"));
        let issue = out
            .issues
            .iter()
            .find(|i| i.code == Code::SharedUntrusted)
            .unwrap();
        assert!(issue.detail.is_some());
        assert!(out.committed.is_empty());
        assert_eq!(fx.remote_rev("dev").unwrap(), dev_before);
        assert_eq!(fx.remote_rev("helper-dev"), None);
        assert_eq!(g(&fx.helper_dir, &["rev-parse", "HEAD"]), head_before);
        // The repo did not join anything locally.
        assert!(fx
            .helper_db
            .get_repository(fx.helper_repo)
            .unwrap()
            .project_id
            .is_none());
    }

    #[test]
    fn no_login_marks_a_shared_repo_and_skips_it() {
        let fx = two_installs();
        fx.enable_main();
        fx.main_db.delete_setting("github_login").unwrap();
        let out = fx.sync_main();
        assert_eq!(fx.main_status().as_deref(), Some("shared_no_login"));
        assert!(codes(&out).contains(&Code::SharedNoLogin));
        assert!(fx.sync_main().issues.is_empty());
    }

    #[test]
    fn a_repo_that_stops_being_shared_becomes_ordinary_again() {
        let fx = two_installs();
        fx.make_participant();
        assert!(is_shared_repo(&fx.helper_db, fx.helper_repo));
        // The owner removes the structure from the remote.
        g(&fx.main_dir, &["rm", "-q", "-r", ".sdh"]);
        g(&fx.main_dir, &["commit", "-q", "-m", "unshare"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
        fx.sync_helper();
        let st = fx.helper_db.get_shared_state(fx.helper_repo).unwrap();
        assert_eq!(st.shared_role, None);
        assert!(!is_shared_repo(&fx.helper_db, fx.helper_repo));
    }

    #[test]
    fn phase_a_leaves_ordinary_repos_alone() {
        let fx = two_installs();
        let sync = run_phase_a(&fx.main_db, Some(&gb()));
        assert!(sync.handled.is_empty());
        let out = sync.finish(&fx.main_db, Some(&gb()));
        assert!(out.issues.is_empty());
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);
    }

    // ----- switching on and off ----------------------------------------------------

    #[test]
    fn preflight_finds_a_user_section_line_and_enable_removes_exactly_it() {
        let fx = two_installs();
        write(
            &fx.main_dir,
            ".gitignore",
            "node_modules\ndocs/todo.md\ntarget\n# --- solo-dev-hub:begin ---\ndocs/done.md\n# --- solo-dev-hub:end ---\n",
        );
        let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(
            pre.blocking_lines,
            vec![IgnoreLine {
                line_no: 2,
                text: "docs/todo.md".into()
            }],
            "the managed block's own lines are not reported"
        );
        assert!(!pre.missing_project && !pre.missing_dev_branch);

        let err = enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).unwrap_err();
        assert!(err.contains("ignore"), "{}", err);
        assert!(
            !fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .shared_mode
        );

        enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, true).unwrap();
        assert_eq!(
            fs::read_to_string(fx.main_dir.join(".gitignore")).unwrap(),
            "node_modules\ntarget\n# --- solo-dev-hub:begin ---\ndocs/done.md\n# --- solo-dev-hub:end ---\n"
        );
        let st = fx.main_db.get_shared_state(fx.main_repo).unwrap();
        assert!(st.shared_mode);
        assert_eq!(st.shared_role.as_deref(), Some("main"));
        assert!(fx.main_dir.join(".sdh/project.json").exists());
        assert!(
            g(&fx.main_dir, &["config", "--local", "merge.sdh.driver"]).contains("--merge-driver")
        );
    }

    #[test]
    fn preflight_flags_a_missing_project_and_dev_branch_and_enable_refuses() {
        let fx = two_installs();
        fx.main_db
            .set_autocommit_branch(fx.main_repo, None)
            .unwrap();
        fx.main_db
            .assign_repository(fx.main_repo, None, None)
            .unwrap();
        let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(pre.missing_project && pre.missing_dev_branch);
        assert!(enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, true).is_err());
        assert!(
            !fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .shared_mode
        );
    }

    #[test]
    fn enable_refuses_a_repo_owned_by_somebody_else() {
        let fx = two_installs();
        assert!(enable_shared_mode(&fx.helper_db, &gb(), fx.helper_repo, true).is_err());
        assert!(
            !fx.helper_db
                .get_shared_state(fx.helper_repo)
                .unwrap()
                .shared_mode
        );
    }

    #[test]
    fn disable_deletes_the_structure_and_commits_and_pushes_it() {
        let fx = two_installs();
        fx.make_participant();
        assert!(fx.remote_file("dev", ".sdh/project.json").is_some());
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(res.pushed, "{:?}", res);
        assert!(!fx.main_dir.join(".sdh").exists());
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);
        assert_eq!(
            g(&fx.main_dir, &["log", "-1", "--format=%s"]),
            "chore(sdh-sync): remove shared project structure"
        );
        let st = fx.main_db.get_shared_state(fx.main_repo).unwrap();
        assert!(!st.shared_mode && st.shared_role.is_none());
        // The repo is ordinary now: the shared pass ignores it.
        assert!(run_phase_a(&fx.main_db, Some(&gb())).handled.is_empty());
    }

    #[test]
    fn disable_from_another_branch_plumbs_the_deletion_into_dev() {
        let fx = two_installs();
        fx.enable_main();
        g(&fx.main_dir, &["checkout", "-q", "-b", "feature"]);
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(res.pushed, "{:?}", res);
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);
        assert_eq!(g(&fx.main_dir, &["branch", "--show-current"]), "feature");
    }

    #[test]
    fn accept_new_main_branch_clears_the_pin_and_status() {
        let fx = two_installs();
        fx.make_participant();
        let v = ss::read_shared_import(&fx.helper_db, fx.helper_repo);
        assert_eq!(v.pinned_dev_branch.as_deref(), Some("dev"));
        fx.helper_db
            .set_shared_status(fx.helper_repo, Some("shared_untrusted"))
            .unwrap();
        accept_new_main_branch(&fx.helper_db, fx.helper_repo).unwrap();
        assert_eq!(
            ss::read_shared_import(&fx.helper_db, fx.helper_repo).pinned_dev_branch,
            None
        );
        assert_eq!(fx.helper_status(), None);
    }

    #[test]
    fn join_requests_are_listed_for_the_owner() {
        let fx = two_installs();
        fx.enable_main();
        fx.sync_helper();
        fx.sync_main();
        let reqs = list_join_requests(&fx.main_db, fx.main_repo).unwrap();
        assert_eq!(
            reqs,
            vec![JoinRequest {
                login: "helper".into(),
                branch: "helper-dev".into()
            }]
        );
        // Only the owner can approve.
        assert!(approve_join_request(&fx.helper_db, fx.helper_repo, "helper").is_err());
        // A pending helper sees the owner's structure from the remote refs.
        let s = get_repo_structure(&fx.helper_db, Some(&gb()), fx.helper_repo)
            .unwrap()
            .unwrap();
        assert_eq!(s.main.login, "Owner");
    }

    // ----- plumbing rules --------------------------------------------------------------

    #[test]
    fn plumbing_is_a_three_way_copy_for_other_shared_files() {
        let fx = two_installs();
        fx.make_participant();
        // Both branches start with an api contract.
        write(&fx.helper_dir, "docs/server-api/api.md", "# v1\n");
        fx.sync_helper();
        fx.sync_main();
        assert_eq!(
            fs::read_to_string(fx.main_dir.join("docs/server-api/api.md")).unwrap(),
            "# v1\n"
        );
        // The helper changes it; main did not touch it: main's push must not
        // revert the helper's branch to main's stale copy.
        write(&fx.helper_dir, "docs/server-api/api.md", "# v2 by helper\n");
        fx.sync_helper();
        fx.sync_main();
        assert_eq!(
            fx.remote_file("helper-dev", "docs/server-api/api.md")
                .as_deref(),
            Some("# v2 by helper\n")
        );
        // Main changes it later: that change reaches the helper's branch.
        write(&fx.main_dir, "docs/server-api/api.md", "# v3 by main\n");
        fx.sync_main();
        assert_eq!(
            fx.remote_file("helper-dev", "docs/server-api/api.md")
                .as_deref(),
            Some("# v3 by main\n")
        );
        // Main deletes it: the deletion reaches the helper's branch too.
        fs::remove_file(fx.main_dir.join("docs/server-api/api.md")).unwrap();
        fx.sync_main();
        assert_eq!(fx.remote_file("helper-dev", "docs/server-api/api.md"), None);
    }

    #[test]
    fn rule_marker_files_never_travel_with_the_skills_folder() {
        let fx = two_installs();
        fx.enable_main();
        write(&fx.main_dir, "docs/sdh_skills/sdh-x.md", "# rule\n");
        write(&fx.main_dir, "docs/sdh_skills/sdh-x.md.approved", "");
        write(&fx.main_dir, "docs/sdh_skills/sdh-x.md.conflict", "");
        fx.sync_main();
        assert!(fx.remote_file("dev", "docs/sdh_skills/sdh-x.md").is_some());
        assert_eq!(
            fx.remote_file("dev", "docs/sdh_skills/sdh-x.md.approved"),
            None
        );
        assert_eq!(
            fx.remote_file("dev", "docs/sdh_skills/sdh-x.md.conflict"),
            None
        );
    }

    #[test]
    fn contributors_never_commit_the_skills_folder() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.helper_dir, "docs/sdh_skills/sdh-y.md", "# rule\n");
        fx.sync_helper();
        assert_eq!(
            fx.remote_file("helper-dev", "docs/sdh_skills/sdh-y.md"),
            None
        );
        assert_eq!(fx.remote_file("dev", "docs/sdh_skills/sdh-y.md"), None);
    }
}
