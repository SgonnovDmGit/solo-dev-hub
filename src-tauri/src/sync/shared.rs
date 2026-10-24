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
    AdminCheck, IgnoreLine, ParticipantActivity, RepoSharedState, Repository, SharedDisableResult,
    SharedMergeResult, SharedPreflight, SyncIssue, SyncIssueCode as Code,
};
use crate::sync::autocommit::{
    CROSS_REPO_DIRS, MANAGED_ROOT_FILES, SDH_COMMIT_EMAIL, SDH_COMMIT_NAME,
};
use crate::sync::merge_by_id::{kind_for_path, merge_shared_bytes, merge_shared_file_per_entity};
use crate::sync::shared_structure::{
    self as ss, DiscoverOutcome, JoinRequest, ProjectStructure, Role, REQUESTS_REL_DIR,
    STRUCTURE_REL_PATH,
};

const REMOTE: &str = "origin";

/// Subject of the commit that deletes `.sdh/` from a branch (switch-off). It counts
/// as one of our own commits wherever the sync judges a branch.
const REMOVAL_SUBJECT: &str = "chore(sdh-sync): remove shared project structure";

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
    /// `NeedsManualMerge` only: the diverged branch and whether it is checked
    /// out here (drives the Merge button of the UI).
    diverged: Option<(String, bool)>,
    /// `PushRejected` only: the branch the push was refused on.
    rejected_branch: Option<String>,
    /// A coded detail for the UI that replaces the human `detail`
    /// (`Untrusted` with a dev-branch-changed reason).
    ui: Option<String>,
}

impl StatusNote {
    /// The detail stored next to the status code for the UI
    /// (`repositories.shared_import.status_detail`, see
    /// `AppDb::set_shared_status_with_detail`).
    ///
    /// * `needs_manual_merge`: `{"branch":"dev","current":true}`
    /// * `push_rejected`: `{"branch":"dev","message":"<git message>"}`
    /// * the others: the plain reason / message, if any.
    fn ui_detail(&self) -> Option<String> {
        if let Some(ui) = &self.ui {
            return Some(ui.clone());
        }
        if let Some((branch, current)) = &self.diverged {
            return Some(serde_json::json!({ "branch": branch, "current": current }).to_string());
        }
        if let Some(branch) = &self.rejected_branch {
            return Some(
                serde_json::json!({ "branch": branch, "message": self.detail.trim() }).to_string(),
            );
        }
        let d = self.detail.trim();
        (!d.is_empty()).then(|| d.to_string())
    }

    /// Among notes of one status the one the UI should show: a diverged
    /// current branch (it has the Merge button) comes first.
    fn rank(&self) -> (SharedStatus, bool) {
        let current = self.diverged.as_ref().is_some_and(|(_, c)| *c);
        (self.status, !current)
    }
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

pub(super) const SKILLS_DIR: &str = "docs/sdh_skills";
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

pub(super) fn git_bytes(
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
pub(super) fn ls_tree(
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

/// Files only the owner writes: the owner's copy always wins.
fn sole_writer(path: &str) -> bool {
    path == STRUCTURE_REL_PATH || path.starts_with("docs/sdh_skills/")
}

/// `.sdh/requests/<login>.json` of a login that is a participant by now.
fn is_approved_request(path: &str, approved: &[String]) -> bool {
    path.strip_prefix(&format!("{}/", REQUESTS_REL_DIR))
        .and_then(|n| n.strip_suffix(".json"))
        .is_some_and(|stem| approved.iter().any(|l| ss::logins_equal(l, stem)))
}

/// How many commits of a path's history are looked at.
const HISTORY_CAP: usize = 200;

/// The blobs `path` had in the history of `rev`, newest first (at most
/// `HISTORY_CAP` commits, a blob once per commit that gave it to the file): one
/// `git log --raw` call.
fn blobs_in_history(git: &GitBinary, root: &Path, rev: &str, path: &str) -> Vec<String> {
    let cap = format!("-n{}", HISTORY_CAP);
    let Ok(out) = spawn_cmd(&git.0)
        .args([
            "log",
            "--raw",
            "--no-abbrev",
            "--topo-order",
            "-m",
            "--format=%H",
            cap.as_str(),
            rev,
            "--",
            path,
        ])
        .current_dir(root)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let zero = "0".repeat(40);
    let mut blobs: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !line.starts_with(':') {
            continue;
        }
        // :<old mode> <new mode> <old sha> <new sha> <status>
        if let Some(new) = line.split_whitespace().nth(3) {
            if new != zero {
                blobs.push(new.to_string());
            }
        }
    }
    blobs
}

/// Index of the OLDEST occurrence of every blob of a newest-first list: the
/// point where that content first appeared in the history.
fn oldest_positions(blobs: &[String]) -> HashMap<&str, usize> {
    let mut m = HashMap::new();
    for (i, b) in blobs.iter().enumerate() {
        m.insert(b.as_str(), i);
    }
    m
}

/// The last version of `path` the histories of `ours_rev` and `theirs_rev`
/// agreed on (each looked at to `HISTORY_CAP` commits). Two branches that only
/// ever exchange plumbed commits share no merge-base after the file was born,
/// but they do share its blobs; this is the base of a three-way merge between
/// them. Content recurs (a task moves out of a file and the file equals an
/// older version), so a blob is judged by where it FIRST appeared on each side,
/// not by its latest occurrence: the shared blob whose first appearances are
/// the most recent on both sides wins (smallest sum of ages, ours first on a
/// tie). `None` when they never held the same version. Production code calls
/// `blobs_in_history` + `common_of` directly (T-000226); this wrapper is kept
/// for the tests of the heuristic.
#[cfg(test)]
fn last_common_blob(
    git: &GitBinary,
    root: &Path,
    ours_rev: &str,
    theirs_rev: &str,
    path: &str,
) -> Option<String> {
    let ours = blobs_in_history(git, root, ours_rev, path);
    if ours.is_empty() {
        return None;
    }
    let theirs = blobs_in_history(git, root, theirs_rev, path);
    common_of(&ours, &theirs)
}

/// The heuristic of `last_common_blob` over two newest-first blob lists.
fn common_of(ours: &[String], theirs: &[String]) -> Option<String> {
    let (po, pt) = (oldest_positions(ours), oldest_positions(theirs));
    po.iter()
        .filter_map(|(b, i)| pt.get(b).map(|j| (i + j, *i, *b)))
        .min()
        .map(|(_, _, b)| b.to_string())
}

/// Contents of blobs in one `git cat-file --batch` call; a blob git does not
/// have is left out.
fn cat_blobs(
    git: &GitBinary,
    root: &Path,
    shas: &[&str],
) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut got = HashMap::new();
    if shas.is_empty() {
        return Ok(got);
    }
    let input = (shas.join("\n") + "\n").into_bytes();
    let out = run_with_stdin(git, root, &["cat-file", "--batch"], input)?;
    if !out.status.success() {
        return Err(format!(
            "git cat-file failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // <sha> <type> <size>\n<content>\n, or <sha> missing\n
    let buf = out.stdout;
    let mut pos = 0;
    while pos < buf.len() {
        let Some(nl) = buf[pos..].iter().position(|&c| c == b'\n') else {
            break;
        };
        let header = String::from_utf8_lossy(&buf[pos..pos + nl]).into_owned();
        pos += nl + 1;
        let mut it = header.split_whitespace();
        let (Some(sha), Some(kind)) = (it.next(), it.next()) else {
            break;
        };
        if kind == "missing" {
            continue;
        }
        let Some(size) = it.next().and_then(|s| s.parse::<usize>().ok()) else {
            break;
        };
        if pos + size > buf.len() {
            break;
        }
        got.insert(sha.to_string(), buf[pos..pos + size].to_vec());
        pos += size + 1;
    }
    Ok(got)
}

/// The versions of a shared file the two histories held, as the inputs of the
/// per-entity merge: the distinct normalized texts and, per side, the index of
/// each commit's version OLDEST first. A blob that is missing or not text is
/// left out of both histories; `None` only when git cannot be asked at all (the
/// caller then merges against the single whole-file base).
fn history_versions(
    git: &GitBinary,
    root: &Path,
    ours: &[String],
    theirs: &[String],
) -> Option<(Vec<String>, Vec<usize>, Vec<usize>)> {
    let mut uniq: Vec<&str> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for b in ours.iter().chain(theirs.iter()) {
        if seen.insert(b.as_str()) {
            uniq.push(b.as_str());
        }
    }
    let blobs = cat_blobs(git, root, &uniq).ok()?;
    let mut versions: Vec<String> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for sha in &uniq {
        // A blob that is missing or not text drops out of the histories alone.
        let Some(raw) = blobs.get(*sha) else {
            continue;
        };
        let Ok(text) = String::from_utf8(normalize(raw.clone())) else {
            continue;
        };
        index.insert(sha, versions.len());
        versions.push(text);
    }
    let seq = |list: &[String]| -> Vec<usize> {
        list.iter()
            .rev()
            .filter_map(|b| index.get(b.as_str()).copied())
            .collect()
    };
    Some((versions, seq(ours), seq(theirs)))
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
/// merged by id (ours = the work tree, theirs = the branch, base = per row the
/// last state of that row both histories held, with the last blob of the file
/// both held, else the file at the merge-base, else nothing as the fallback for
/// structure: a row that is in the base but missing on one side was deleted
/// there and stays deleted). Every other file is a three-way copy with the same
/// base: the work tree wins only when we changed the file since the base AND
/// the branch is still at the base (a version this checkout never had is newer
/// foreign work and is never clobbered; the owner's own
/// `.sdh/project.json` and rule files are exempt). A file we deleted (the
/// branch still holds the version we last had) is deleted from the branch, and
/// so is the request file of an approved participant. A file the branch deleted
/// is not resurrected unless we changed it. Files outside `pathspec` are never
/// touched; the caller leaves the managed root files out of it.
fn plumb_into_branch(
    git: &GitBinary,
    root: &Path,
    branch: &str,
    pathspec: &[String],
    login: &str,
    approved: &[String],
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
        let merge_base_blob = base_tree.get(path).map(String::as_str);
        // The version both sides last had; only looked up for a file that
        // differs (equal files need no base).
        let hist: Option<(Vec<String>, Vec<String>)> = if wt != br {
            let ours = blobs_in_history(git, root, "HEAD", path);
            let theirs = if ours.is_empty() {
                Vec::new()
            } else {
                blobs_in_history(git, root, &refname, path)
            };
            Some((ours, theirs))
        } else {
            None
        };
        let common = hist
            .as_ref()
            .and_then(|(o, t)| if o.is_empty() { None } else { common_of(o, t) });
        let bs: Option<&str> = common.as_deref().or(merge_base_blob);
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
                    // Each row gets its own base from the two histories.
                    let per_entity = hist
                        .as_ref()
                        .filter(|(o, t)| !o.is_empty() && !t.is_empty())
                        .and_then(|(o, t)| history_versions(git, root, o, t))
                        .and_then(|(versions, os, ts)| {
                            let (b, o, t) = (
                                std::str::from_utf8(&base).ok()?,
                                std::str::from_utf8(&ours).ok()?,
                                std::str::from_utf8(&theirs).ok()?,
                            );
                            Some(merge_shared_file_per_entity(
                                kind, b, &versions, &os, &ts, o, t,
                            ))
                        });
                    let merged = match per_entity {
                        Some(r) => r,
                        None => merge_shared_bytes(kind, &base, &ours, &theirs),
                    };
                    match merged {
                        Ok(m) if m.as_bytes() == theirs.as_slice() => Plan::Skip,
                        Ok(m) => Plan::Write(m.into_bytes()),
                        // Not text: leave the branch's copy alone.
                        Err(_) => Plan::Skip,
                    }
                } else if Some(w) == bs {
                    // Unchanged on our side: the branch's version stands.
                    Plan::Skip
                } else if Some(b) == bs || sole_writer(path) {
                    read_worktree(root, path).map_or(Plan::Skip, Plan::Write)
                } else {
                    // The branch holds a version this checkout never had: newer
                    // foreign work, never clobbered.
                    Plan::Skip
                }
            }
            (Some(w), None) => {
                if Some(w) == bs {
                    Plan::Skip
                } else {
                    read_worktree(root, path).map_or(Plan::Skip, Plan::Write)
                }
            }
            (None, Some(_)) if is_approved_request(path, approved) => Plan::Delete,
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
// T-000221: import what arrived by commit (spec R11, R14)
// ---------------------------------------------------------------------------

const TODO_REL: &str = "docs/todo.md";
const DONE_REL: &str = "docs/done.md";
const BUGS_REL: &str = "docs/bug-reports.md";
/// The task files whose committed content is imported as truth.
const IMPORT_FILES: [&str; 3] = [TODO_REL, DONE_REL, BUGS_REL];

/// `"<blob>@<commit>"` as stored in `SharedImport::blobs`.
fn stored_pair(imp: &ss::SharedImport, path: &str) -> Option<(String, Option<String>)> {
    let v = imp.blobs.get(path)?;
    Some(match v.split_once('@') {
        Some((b, c)) => (b.to_string(), Some(c.to_string()).filter(|c| !c.is_empty())),
        None => (v.clone(), None),
    })
}

fn store_pair(imp: &mut ss::SharedImport, path: &str, blob: &str, commit: &str) {
    imp.blobs
        .insert(path.to_string(), format!("{}@{}", blob, commit));
}

pub(super) fn blob_text(git: &GitBinary, root: &Path, sha: &str) -> Result<String, String> {
    Ok(String::from_utf8_lossy(&cat_blob(git, root, sha)?).into_owned())
}

/// Whose events a committed change causes (spec R14).
enum AuthorRule {
    /// One answer for everything (`None` = this install).
    Fixed(Option<String>),
    /// Per entity: the owner of the band its id lies in.
    ByBand,
}

struct Authors {
    me: String,
    structure: Option<ProjectStructure>,
}

impl Authors {
    /// The `SDH-Participant:` logins of the commits in `from..HEAD` that touched
    /// `path`. `None` when the range cannot be used (no recorded commit, or it
    /// is no longer an ancestor of HEAD).
    fn trailer_logins(
        git: &GitBinary,
        root: &Path,
        from: Option<&str>,
        path: &str,
    ) -> Option<Vec<String>> {
        let from = from?;
        if !git_sync::is_ancestor(git, root, from, "HEAD").unwrap_or(false) {
            return None;
        }
        let out = spawn_cmd(&git.0)
            .args([
                "log",
                "--format=%(trailers:key=SDH-Participant,valueonly)",
                &format!("{}..HEAD", from),
                "--",
                path,
            ])
            .current_dir(root)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::trim)
                .filter(|l| ss::is_valid_login(l))
                .map(String::from)
                .collect(),
        )
    }

    /// This install's own login does not count. Exactly one other distinct
    /// login -> that login; none -> `None`; several (or an unusable range) ->
    /// decided per entity by band.
    fn rule(&self, logins: Option<Vec<String>>) -> AuthorRule {
        let Some(logins) = logins else {
            return AuthorRule::ByBand;
        };
        let mut distinct: Vec<String> = Vec::new();
        for l in logins {
            if !ss::logins_equal(&l, &self.me) && !distinct.iter().any(|d| ss::logins_equal(d, &l))
            {
                distinct.push(l);
            }
        }
        match distinct.len() {
            0 => AuthorRule::Fixed(None),
            1 => AuthorRule::Fixed(distinct.pop()),
            _ => AuthorRule::ByBand,
        }
    }

    fn resolve(&self, rule: &AuthorRule, number: Option<i64>) -> Option<String> {
        match rule {
            AuthorRule::Fixed(a) => a.clone(),
            AuthorRule::ByBand => {
                let (s, n) = (self.structure.as_ref()?, number?);
                let owner = ss::band_owner(s, n);
                (!ss::logins_equal(&owner, &self.me)).then_some(owner)
            }
        }
    }
}

/// `T-500001` / `B-000042` -> the number.
fn id_number(id: &str) -> Option<i64> {
    id.split_once('-')?.1.parse().ok()
}

/// Import what arrived by commit into the local database (spec R11 step 1).
///
/// Called by phase A once the recipient branches have been fast-forwarded, for
/// every shared repo. For each of `todo.md`, `done.md` and `bug-reports.md`
/// whose blob in HEAD differs from the one this install last imported or
/// committed (`SharedImport::blobs`), the difference "stored -> HEAD" is applied
/// to the database as truth, protected fields and new rows included, with the
/// author of Р14. The bugs file is then reconciled against the work tree like
/// always, so an UNCOMMITTED local edit of a protected field is restored while a
/// committed one sticks.
///
/// The first sync (nothing stored yet) is the baseline: tasks apply nothing and
/// leave it to the ordinary disk run; bugs run the one-time migration when it
/// has not happened, otherwise only the rows that differ from what the database
/// renders are applied. A file that fails to import keeps its old stored blob,
/// so the next sync retries it.
pub fn import_incoming(
    db: &AppDb,
    git: &GitBinary,
    repo_id: i64,
    root: &Path,
) -> Result<(), String> {
    let Some(head) = git_sync::rev_parse(git, root, "HEAD")? else {
        return Ok(());
    };
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let imp = ss::read_shared_import(db, repo_id);
    let authors = Authors {
        me: db.github_login().unwrap_or_default(),
        structure: repo.github_name.as_deref().and_then(|gh| {
            ss::read_structure_file(root, gh, &ss::AdminVerified::load(db))
                .ok()
                .flatten()
        }),
    };
    let mut head_blob: BTreeMap<&str, Option<String>> = BTreeMap::new();
    for p in IMPORT_FILES {
        head_blob.insert(p, git_sync::blob_sha_at(git, root, "HEAD", p)?);
    }
    let pending = |imp: &ss::SharedImport, p: &str| -> bool {
        match (&head_blob[p], stored_pair(imp, p)) {
            (Some(h), Some((s, _))) => *h != s,
            (Some(_), None) => true,
            (None, _) => false,
        }
    };
    let mut errors: Vec<String> = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    let mut skip_reconcile = false;

    // Tasks.
    if pending(&imp, TODO_REL) || pending(&imp, DONE_REL) {
        match import_tasks(db, git, root, repo_id, &imp, &head_blob, &authors) {
            Ok(()) => done.extend([TODO_REL, DONE_REL]),
            Err(e) => errors.push(format!("tasks: {}", e)),
        }
    }
    // Bugs.
    if pending(&imp, BUGS_REL) {
        match import_bugs(db, git, root, repo_id, &imp, &head_blob, &authors) {
            Ok(refused) => {
                done.push(BUGS_REL);
                if !refused.is_empty() {
                    skip_reconcile = true;
                    errors.push(format!(
                        "bugs: {} row(s) with a value the schema refuses were not imported: {}",
                        refused.len(),
                        refused.join(", ")
                    ));
                }
            }
            Err(e) => {
                // Reconciling now would revert the incoming change in the
                // work tree and commit that.
                skip_reconcile = true;
                errors.push(format!("bugs: {}", e));
            }
        }
    }
    if !done.is_empty() {
        let mut fresh = ss::read_shared_import(db, repo_id);
        for p in done {
            if let Some(Some(h)) = head_blob.get(p) {
                store_pair(&mut fresh, p, h, &head);
            }
        }
        ss::write_shared_import(db, repo_id, &fresh)?;
    }

    // The work-tree diff against what is in the database now: local AI edits.
    if !skip_reconcile
        && db
            .get_bugs_migrated_at(repo_id)
            .map_err(|e| e.to_string())?
            .is_some()
    {
        if let Err(e) = crate::sync::bugs::reconcile_bugs_for_repo(db, repo_id) {
            errors.push(format!("reconcile: {}", e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn import_tasks(
    db: &AppDb,
    git: &GitBinary,
    root: &Path,
    repo_id: i64,
    imp: &ss::SharedImport,
    head_blob: &BTreeMap<&str, Option<String>>,
    authors: &Authors,
) -> Result<(), String> {
    let (st_todo, st_done) = (stored_pair(imp, TODO_REL), stored_pair(imp, DONE_REL));
    if st_todo.is_none() && st_done.is_none() {
        // Baseline: the ordinary disk run mirrors the files.
        return Ok(());
    }
    if db
        .get_tasks_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        // The mirror was never built; the first disk run suppresses `created`.
        return Ok(());
    }
    let prev_text = |st: &Option<(String, Option<String>)>| -> Result<String, String> {
        match st {
            Some((b, _)) => blob_text(git, root, b).or_else(|_| Ok(String::new())),
            None => Ok(String::new()),
        }
    };
    let head_text = |p: &str| -> Result<Option<String>, String> {
        match &head_blob[p] {
            Some(b) => Ok(Some(blob_text(git, root, b)?)),
            None => Ok(None),
        }
    };
    let (prev_todo, prev_done) = (prev_text(&st_todo)?, prev_text(&st_done)?);
    let (head_todo, head_done) = (head_text(TODO_REL)?, head_text(DONE_REL)?);

    let mut logins: Option<Vec<String>> = Some(Vec::new());
    for (p, st) in [(TODO_REL, &st_todo), (DONE_REL, &st_done)] {
        if head_blob[p].as_deref() == st.as_ref().map(|s| s.0.as_str()) {
            continue;
        }
        let from = st.as_ref().and_then(|s| s.1.as_deref());
        match (Authors::trailer_logins(git, root, from, p), logins.as_mut()) {
            (Some(l), Some(all)) => all.extend(l),
            _ => logins = None,
        }
    }
    let rule = authors.rule(logins);
    let author_of = |id: &str| authors.resolve(&rule, id_number(id));
    crate::sync::tasks::sync_tasks_incoming(
        db,
        repo_id,
        (&prev_todo, &prev_done),
        (head_todo.as_deref(), head_done.as_deref()),
        &author_of,
    )?;
    Ok(())
}

/// Import the bugs file; returns the ids of rows that were refused.
///
/// The first sync (nothing stored) of an already migrated repo applies the
/// database-render baseline only when the work-tree file equals HEAD's copy (a
/// clean tree: the database is merely stale). A file that differs holds local
/// work that is not committed yet, so HEAD is recorded as the baseline without
/// applying anything.
fn import_bugs(
    db: &AppDb,
    git: &GitBinary,
    root: &Path,
    repo_id: i64,
    imp: &ss::SharedImport,
    head_blob: &BTreeMap<&str, Option<String>>,
    authors: &Authors,
) -> Result<Vec<String>, String> {
    use crate::sync::bugs as bugs_sync;
    if db
        .get_bugs_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        // Never migrated: the one-time import reads the work tree, which equals
        // HEAD after a clean fast-forward.
        bugs_sync::migrate_bugs_for_repo(db, repo_id)?;
        return Ok(Vec::new());
    }
    let Some(head_sha) = &head_blob[BUGS_REL] else {
        return Ok(Vec::new());
    };
    let stored = stored_pair(imp, BUGS_REL);
    let prev_text = match &stored {
        Some((b, _)) => blob_text(git, root, b).ok(),
        None => None,
    };
    let prev_text = match prev_text {
        Some(t) => t,
        None => {
            let on_disk = if root.join(BUGS_REL).is_file() {
                hash_files(git, root, &[BUGS_REL.to_string()])?.remove(BUGS_REL)
            } else {
                None
            };
            if on_disk.as_ref() != Some(head_sha) {
                return Ok(Vec::new());
            }
            // Baseline: what the database renders now, so only real differences apply.
            let notes: Vec<crate::models::FileBugNote> = db
                .list_bugs_for_md(repo_id)
                .map_err(|e| e.to_string())?
                .iter()
                .map(bugs_sync::bug_to_file_note)
                .collect();
            crate::export::generate_bug_reports(&notes)
        }
    };
    let head_text = blob_text(git, root, head_sha)?;
    let from = stored.as_ref().and_then(|s| s.1.as_deref());
    let rule = authors.rule(Authors::trailer_logins(git, root, from, BUGS_REL));
    let author_of = |n: i64| authors.resolve(&rule, Some(n));
    Ok(bugs_sync::import_bug_delta(db, repo_id, &prev_text, &head_text, &author_of)?.refused)
}

/// After this install's own commit on the current branch: the new HEAD's copies
/// of the shared task files are what it committed itself and need no import.
/// A file whose stored blob is neither absent nor the blob it had before the
/// commit (`pre_head`) has a pending import and keeps its old record.
fn record_own_commit(
    db: &AppDb,
    git: &GitBinary,
    root: &Path,
    repo_id: i64,
    pre_head: Option<&str>,
) {
    let Ok(Some(head)) = git_sync::rev_parse(git, root, "HEAD") else {
        return;
    };
    let mut imp = ss::read_shared_import(db, repo_id);
    let mut touched = false;
    for p in IMPORT_FILES {
        let Ok(Some(now)) = git_sync::blob_sha_at(git, root, "HEAD", p) else {
            continue;
        };
        let before = pre_head
            .and_then(|h| git_sync::blob_sha_at(git, root, h, p).ok())
            .flatten();
        let ok = match stored_pair(&imp, p) {
            None => true,
            Some((s, _)) => s == now || Some(&s) == before.as_ref(),
        };
        if ok {
            store_pair(&mut imp, p, &now, &head);
            touched = true;
        }
    }
    if touched {
        let _ = ss::write_shared_import(db, repo_id, &imp);
    }
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
    /// Recurring non-status problems of this pass (reported when they change).
    issues: Vec<SyncIssue>,
    /// No fetch or push of this pass failed.
    net_ok: bool,
    /// `git fetch` really ran (and succeeded) in this pass.
    fetched: bool,
    /// Lower-cased non-current recipient branches that hold commits that are
    /// not ours: never plumbed into, never pushed.
    blocked: HashSet<String>,
    committed: bool,
    /// Rules whose canon this pass took from the owner (T-000224).
    canon_changed: Vec<String>,
    /// (repo, login) pairs whose trust rests on a GitHub administrator check the
    /// frontend has to make (T-000227).
    admin_checks: Vec<AdminCheck>,
}

impl RepoPass {
    /// Ask the frontend to verify `login` as an administrator of `gh`, unless
    /// `login` is the owner segment of the repo (then nothing rests on the API).
    fn ask_admin_check(&mut self, gh: &str, login: &str) {
        if !ss::needs_admin_check(gh, login) {
            return;
        }
        let check = AdminCheck {
            repo: gh.to_string(),
            login: login.to_string(),
        };
        if !self.admin_checks.iter().any(|c| {
            c.repo.eq_ignore_ascii_case(&check.repo) && ss::logins_equal(&c.login, &check.login)
        }) {
            self.admin_checks.push(check);
        }
    }

    fn note(&mut self, status: SharedStatus, detail: impl Into<String>) {
        self.notes.push(StatusNote {
            status,
            detail: detail.into(),
            diverged: None,
            rejected_branch: None,
            ui: None,
        });
    }

    /// `Untrusted` with the reason of the structure check. A coded reason (a
    /// changed dev branch) goes to the UI as is and is spelled out for the issue.
    fn note_untrusted(&mut self, reason: String) {
        let coded = ss::parse_dev_branch_changed(&reason).is_some();
        self.notes.push(StatusNote {
            status: SharedStatus::Untrusted,
            detail: ss::humanize_untrusted_reason(&reason),
            diverged: None,
            rejected_branch: None,
            ui: coded.then_some(reason),
        });
    }

    /// `PushRejected` (protected branch) for `branch`.
    fn note_push_rejected(&mut self, branch: &str, message: String) {
        self.notes.push(StatusNote {
            status: SharedStatus::PushRejected,
            detail: message,
            diverged: None,
            rejected_branch: Some(branch.to_string()),
            ui: None,
        });
    }

    /// `NeedsManualMerge` for `branch`; `current` is true when it is checked out.
    fn note_diverged(&mut self, branch: &str, current: bool) {
        self.notes.push(StatusNote {
            status: SharedStatus::NeedsManualMerge,
            detail: diverged_detail(branch, current),
            diverged: Some((branch.to_string(), current)),
            rejected_branch: None,
            ui: None,
        });
    }

    fn issue(&mut self, issue: SyncIssue) {
        self.issues.push(issue);
    }

    fn is_blocked(&self, branch: &str) -> bool {
        self.blocked.contains(&branch.to_ascii_lowercase())
    }
}

/// The result of phase A, consumed by `finish`.
pub struct SharedSync {
    passes: Vec<RepoPass>,
    /// Repos the shared path took over: the ordinary auto-commit skips them.
    pub handled: HashSet<i64>,
    /// Rules whose canon changed because an owner's committed copy differs
    /// (T-000224): the project sync renders everywhere when this is not empty.
    pub canon_changed: Vec<String>,
    /// Every shared repo of this run whose structure names a main that is not
    /// the repo owner, with that main: the frontend verifies (or re-verifies)
    /// them against GitHub (T-000227). Covers both the structures refused for
    /// lack of a verification and those currently trusted through one.
    pub admin_checks: Vec<AdminCheck>,
}

impl SharedSync {
    pub fn empty() -> Self {
        SharedSync {
            passes: Vec::new(),
            handled: HashSet::new(),
            canon_changed: Vec::new(),
            admin_checks: Vec::new(),
        }
    }

    /// The trusted structure phase A settled for `repo_id`, if it did: what the
    /// CLAUDE.md section's shared block is written from (a pending helper has
    /// no structure file in the work tree yet).
    pub fn structure_of(&self, repo_id: i64) -> Option<&ProjectStructure> {
        self.passes
            .iter()
            .find(|p| p.repo.id == repo_id)
            .and_then(|p| p.structure.as_ref())
    }
}

// ----- throttles ------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum NetKind {
    Fetch,
    Push,
}

/// Minimum time between two fetches (and between two pushes) of one repo, so
/// two project syncs in the same tick do not redo the network work.
fn min_net_interval() -> Duration {
    if cfg!(test) {
        Duration::ZERO
    } else {
        Duration::from_secs(30)
    }
}

/// True (and the slot is taken) when `interval` has passed since the last slot
/// of this kind for this repo.
fn net_slot(kind: NetKind, repo_id: i64, root: &Path, interval: Duration) -> bool {
    static LAST: OnceLock<Mutex<HashMap<(NetKind, i64, String), Instant>>> = OnceLock::new();
    let map = LAST.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (kind, repo_id, root.to_string_lossy().into_owned());
    let mut m = map.lock().unwrap();
    let now = Instant::now();
    match m.get(&key) {
        Some(t) if now.duration_since(*t) < interval => false,
        _ => {
            m.insert(key, now);
            true
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Is a role-less foreign repo due for a probe? Takes the slot (persisted in
/// `SharedImport`) when it is.
fn probe_due(db: &AppDb, repo_id: i64, now: i64) -> bool {
    let mut imp = ss::read_shared_import(db, repo_id);
    if let Some(t) = imp.last_probe_unix {
        if now - t < PROBE_INTERVAL.as_secs() as i64 {
            return false;
        }
    }
    imp.last_probe_unix = Some(now);
    let _ = ss::write_shared_import(db, repo_id, &imp);
    true
}

// ----- recurring issues ---------------------------------------------------------

fn fingerprint(i: &SyncIssue) -> String {
    format!(
        "{:?}|{}|{}",
        i.code,
        i.target.as_deref().unwrap_or(""),
        i.detail.as_deref().unwrap_or("")
    )
}

/// The issues of this pass that were not already reported by the previous pass
/// of the same repo: a problem that keeps recurring is shown once.
fn fresh_issues(repo_id: i64, root: &Path, issues: Vec<SyncIssue>) -> Vec<SyncIssue> {
    static SEEN: OnceLock<Mutex<HashMap<(i64, String), HashSet<String>>>> = OnceLock::new();
    let map = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (repo_id, root.to_string_lossy().into_owned());
    let current: HashSet<String> = issues.iter().map(fingerprint).collect();
    let previous = map.lock().unwrap().insert(key, current).unwrap_or_default();
    issues
        .into_iter()
        .filter(|i| !previous.contains(&fingerprint(i)))
        .collect()
}

fn repo_issue(code: Code, repo: &Repository) -> SyncIssue {
    SyncIssue::new(code, repo.display_name())
}

// ----- safety guards ----------------------------------------------------------------

/// The repo may be touched: no merge or rebase in progress, HEAD on a branch,
/// no unmerged paths.
fn repo_ready(git: &GitBinary, root: &Path) -> bool {
    if !matches!(
        git_sync::repo_state(git, root),
        Ok(crate::git_ops::RepoState::Clean)
    ) {
        return false;
    }
    if !matches!(current_branch(git, root), Ok(Some(_))) {
        return false;
    }
    match spawn_cmd(&git.0)
        .args(["ls-files", "-u"])
        .current_dir(root)
        .output()
    {
        Ok(o) => o.status.success() && o.stdout.is_empty(),
        Err(_) => false,
    }
}

fn path_in_specs(file: &str, specs: &[String]) -> bool {
    specs
        .iter()
        .any(|s| file == s || file.starts_with(&format!("{}/", s)))
}

/// Are all commits of `branch` that origin does not have commits this app made?
/// Each must be authored by the SDH identity, have exactly one parent, carry the
/// exact sync subject and an `SDH-Participant:` trailer, and touch shared files
/// only. `true` when there are none.
fn local_only_commits_are_ours(git: &GitBinary, root: &Path, branch: &str) -> Result<bool, String> {
    Ok(local_tip_if_ours(git, root, branch)?.is_some())
}

/// `local_only_commits_are_ours`, answering with the tip it judged: `Some(sha)`
/// when every local-only commit up to that sha is ours. The judgement is of the
/// sha, not of the ref, so a caller that moves the branch can use the sha as the
/// compare-and-swap old value.
fn local_tip_if_ours(git: &GitBinary, root: &Path, branch: &str) -> Result<Option<String>, String> {
    let tip = git_sync::rev_parse(git, root, &local_ref(branch))?
        .ok_or_else(|| format!("branch '{}' does not exist", branch))?;
    let remote = remote_ref(branch);
    let mut args: Vec<String> = vec!["rev-list".into(), "--reverse".into()];
    if git_sync::rev_parse(git, root, &remote)?.is_some() {
        args.push(format!("{}..{}", remote, tip));
    } else {
        args.push(tip.clone());
        args.push("--not".into());
        args.push(format!("--remotes={}", REMOTE));
    }
    let out = spawn_cmd(&git.0)
        .args(&args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("git rev-list failed to start: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git rev-list failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let specs = shared_pathspec(Role::Main, "");
    for commit in String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        let shown = spawn_cmd(&git.0)
            .args(["show", "-s", "--format=%an%n%ae%n%P%n%s%n%b", commit])
            .current_dir(root)
            .output()
            .map_err(|e| format!("git show failed to start: {}", e))?;
        if !shown.status.success() {
            return Err(format!(
                "git show failed: {}",
                String::from_utf8_lossy(&shown.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&shown.stdout).into_owned();
        let mut lines = text.lines();
        let (name, email, parents, subject) = (
            lines.next().unwrap_or(""),
            lines.next().unwrap_or(""),
            lines.next().unwrap_or(""),
            lines.next().unwrap_or(""),
        );
        let has_trailer = lines.any(|l| l.starts_with("SDH-Participant:"));
        if name != SDH_COMMIT_NAME
            || email != SDH_COMMIT_EMAIL
            || parents.split_whitespace().count() != 1
            || (subject != "chore(sdh-sync): sync shared files" && subject != REMOVAL_SUBJECT)
            || !has_trailer
        {
            return Ok(None);
        }
        let files = git_bytes(
            git,
            root,
            &[
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-r",
                "-z",
                commit,
            ],
            &[],
        )?;
        if split_nul(&files).iter().any(|f| !path_in_specs(f, &specs)) {
            return Ok(None);
        }
    }
    Ok(Some(tip))
}

fn diverged_detail(branch: &str, current: bool) -> String {
    if current {
        format!(
            "{b}: the local branch and origin/{b} have diverged (use the Merge button)",
            b = branch
        )
    } else {
        format!(
            "{b}: the branch is not checked out and has diverged or holds commits that are not sync commits - run `git fetch`, then merge origin/{b} into {b} by hand",
            b = branch
        )
    }
}

// ----- phase A ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum FetchMode {
    /// A repo known to be shared: a failed fetch is a status.
    Always,
    /// Not known to be shared: a failed fetch is silent.
    Probe,
}

/// Phase A for the shared repos of `project_id` and a throttled probe of the
/// role-less foreign repos of the portfolio. `git` `None` (no git on this
/// machine) yields an empty pass.
pub fn run_phase_a(db: &AppDb, git: Option<&GitBinary>, project_id: Option<i64>) -> SharedSync {
    run_phase_a_with(db, git, project_id, false)
}

/// Phase A for the role-less foreign repos only (no project in scope), the probe
/// of the sidebar "Sync" button. The user pressed the button on purpose, so the
/// 10-minute probe throttle is bypassed (the slot is still taken, so the
/// automatic probe of a project sync does not repeat it right away).
pub fn probe_role_less(db: &AppDb, git: Option<&GitBinary>) -> SharedSync {
    run_phase_a_with(db, git, None, true)
}

fn run_phase_a_with(
    db: &AppDb,
    git: Option<&GitBinary>,
    project_id: Option<i64>,
    force_probe: bool,
) -> SharedSync {
    let mut sync = SharedSync::empty();
    let Some(git) = git else {
        return sync;
    };
    let repos = match db.list_all_repos() {
        Ok(r) => r,
        Err(_) => return sync,
    };
    let login = db.github_login();
    let now = unix_now();
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
        let in_project = project_id.is_some() && repo.project_id == project_id;
        let owned_by_me = login.as_deref().is_some_and(|l| ss::repo_owner_is(&gh, l));

        if state.shared_mode {
            if !in_project {
                continue;
            }
            let Some(login) = login.as_deref() else {
                // Shared mode on, but this install does not know its login.
                let mut pass = new_pass(&repo, &root, "");
                pass.note(SharedStatus::NoLogin, "");
                sync.handled.insert(repo.id);
                sync.passes.push(pass);
                continue;
            };
            sync.handled.insert(repo.id);
            sync.passes.push(phase_a_main(db, git, &repo, &root, login));
            continue;
        }
        if ss::read_shared_import(db, repo.id).disabling.is_some() {
            // Switched off, but the deletion of `.sdh/` did not reach the remote.
            if let Some(pass) = retry_disabling(db, git, &repo, &root) {
                sync.passes.push(pass);
            }
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
        if known && !in_project {
            continue;
        }
        // A repo that was shared and is not any more may still have a clean-up to
        // finish (the contributor's own branch still tracks `.sdh/`): that is read
        // from facts and retried on every sync, not on the probe's schedule.
        if !known
            && in_project
            && ss::read_shared_import(db, repo.id)
                .pinned_dev_branch
                .is_some()
        {
            let mut pass = new_pass(&repo, &root, login);
            fetch_into(git, &mut pass, FetchMode::Probe);
            drop_stale_structure(db, git, &repo, &root, pass.net_ok);
        }
        // A role-less repo is probed at most every 10 minutes (persisted); one
        // that was refused as untrusted is looked at again on every sync.
        if !known && state.shared_status.is_none() {
            let due = probe_due(db, repo.id, now);
            if !due && !force_probe {
                continue;
            }
        }
        let mode = if known {
            FetchMode::Always
        } else {
            FetchMode::Probe
        };
        if let Some(pass) = phase_a_foreign(db, git, &repo, &root, &gh, login, mode) {
            sync.handled.insert(repo.id);
            for rule in &pass.canon_changed {
                if !sync.canon_changed.contains(rule) {
                    sync.canon_changed.push(rule.clone());
                }
            }
            sync.passes.push(pass);
        }
    }
    for pass in &sync.passes {
        for c in &pass.admin_checks {
            if !sync.admin_checks.contains(c) {
                sync.admin_checks.push(c.clone());
            }
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
        issues: Vec::new(),
        net_ok: true,
        fetched: false,
        blocked: HashSet::new(),
        committed: false,
        canon_changed: Vec::new(),
        admin_checks: Vec::new(),
    }
}

/// `git fetch origin`, unless this repo was fetched less than 30 s ago.
fn fetch_into(git: &GitBinary, pass: &mut RepoPass, mode: FetchMode) {
    if !net_slot(NetKind::Fetch, pass.repo.id, &pass.root, min_net_interval()) {
        return;
    }
    match git_sync::fetch(git, &pass.root, REMOTE) {
        Ok(()) => pass.fetched = true,
        Err(e) => {
            pass.net_ok = false;
            if mode == FetchMode::Always {
                pass.note(SharedStatus::FetchFailed, e.to_string());
            }
        }
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
) -> RepoPass {
    let mut pass = new_pass(repo, root, login);
    let gh = repo.github_name.clone().unwrap_or_default();
    pass.role = Some(Role::Main);
    let _ = db.set_shared_role(repo.id, Some("main"));
    register_driver(git, &mut pass);
    fetch_into(git, &mut pass, FetchMode::Always);
    let admins = ss::AdminVerified::load(db);
    // The main's own trust may rest on an administrator verification (an
    // organization's repo): ask the frontend to (re)verify this login on every
    // sync, so a revoked admin is noticed.
    pass.ask_admin_check(&gh, login);
    match ss::read_structure_file(root, &gh, &admins) {
        Ok(s) => pass.structure = s,
        Err(reason) => {
            pass.note_untrusted(reason);
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
    after_fetch(db, git, &mut pass);
    pass
}

fn phase_a_foreign(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
    root: &Path,
    gh: &str,
    login: &str,
    mode: FetchMode,
) -> Option<RepoPass> {
    let mut pass = new_pass(repo, root, login);
    fetch_into(git, &mut pass, mode);
    let pinned = ss::read_shared_import(db, repo.id).pinned_dev_branch;
    let admins = ss::AdminVerified::load(db);
    let found = match ss::discover_structure(git, root, gh, pinned.as_deref(), &admins) {
        Ok(f) => f,
        Err(e) => {
            if mode == FetchMode::Always {
                pass.issue(
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
            // Facts decide whether anything is left to clean up (nothing, for a repo
            // that never carried a structure).
            drop_stale_structure(db, git, repo, root, pass.net_ok);
            None
        }
        DiscoverOutcome::Untrusted(reason) => {
            // Refused for lack of a verification? Ask about every non-owner
            // main the files name.
            for l in ss::candidate_admin_logins(git, root, gh, pinned.as_deref()) {
                pass.ask_admin_check(gh, &l);
            }
            pass.note_untrusted(reason);
            Some(pass)
        }
        DiscoverOutcome::Found(s, _source) => {
            // Trusted through a verification? Re-verify it on every sync.
            pass.ask_admin_check(gh, &s.main.login);
            // A structure whose main is this very install while shared mode is
            // off here: this install switched it off. (The owner's repos never
            // get here -- `owned_by_me` skips them -- but a verified admin's do.)
            if ss::derive_role(Some(&s), Some(login)) == Some(Role::Main) {
                return None;
            }
            if let Err(e) = ss::apply_structure_locally(db, repo.id, &s) {
                pass.issue(
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
            register_driver(git, &mut pass);
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
            // Р16: this install does not render rule copies here; copies an
            // earlier version (or the time before the join) left untracked
            // would stop the owner's from being fast-forwarded in.
            if role != Role::Main {
                drop_untracked_rule_copies(git, root);
            }
            pass.structure = Some(s);
            after_fetch(db, git, &mut pass);
            Some(pass)
        }
    }
}

/// Does `rev` exist and carry `.sdh/project.json`? `None` when `rev` is unknown.
fn tracks_structure(git: &GitBinary, root: &Path, rev: &str) -> Option<bool> {
    if !matches!(git_sync::rev_parse(git, root, rev), Ok(Some(_))) {
        return None;
    }
    Some(matches!(
        git_sync::blob_sha_at(git, root, rev, STRUCTURE_REL_PATH),
        Ok(Some(_))
    ))
}

/// Delete the tracked `.sdh/` files from the checked-out `branch` with a commit.
/// `Ok(false)` when nothing is tracked. The files are put back on a failure.
fn remove_sdh_from_checked_out(
    git: &GitBinary,
    root: &Path,
    branch: &str,
    sdh: &[String],
    login: &str,
) -> Result<bool, String> {
    let tracked = ls_files_tracked(git, root, sdh)?;
    if tracked.is_empty() {
        return Ok(false);
    }
    let (_, body) = commit_message(login);
    remove_sdh_files(root);
    let paths: Vec<PathBuf> = tracked.iter().map(PathBuf::from).collect();
    match commit_paths(
        git,
        root,
        branch,
        &paths,
        REMOVAL_SUBJECT,
        Some(&body),
        SDH_COMMIT_NAME,
        SDH_COMMIT_EMAIL,
    ) {
        Ok(CommitOutcome::Committed { .. }) => Ok(true),
        other => {
            let _ = git_bytes(git, root, &["checkout", "HEAD"], &tracked);
            match other {
                Err(e) => Err(e),
                _ => Ok(false),
            }
        }
    }
}

/// BUG-3, contributor side. The condition is read from facts, not from the cached
/// role (which the switch-off clears at once): the pinned dev branch exists on
/// origin and no longer carries `.sdh/project.json`, while this contributor's own
/// branch -- or origin's copy of it -- still does. A pull request from such a
/// branch would switch sharing back on in the dev branch, so the file is deleted
/// from it (commit on the current branch, plumbing otherwise) and the branch is
/// pushed, best effort. Every step that fails leaves the facts as they are, so the
/// next sync does it again; the pin is dropped only once neither the branch, its
/// origin copy nor the work tree carries the file (until then it also keeps the
/// work-tree file from being taken for a live structure).
fn drop_stale_structure(db: &AppDb, git: &GitBinary, repo: &Repository, root: &Path, net_ok: bool) {
    let mut imp = ss::read_shared_import(db, repo.id);
    let Some(pin) = imp.pinned_dev_branch.clone() else {
        return;
    };
    if !repo_ready(git, root) {
        return;
    }
    // The switch-off must be established by origin's pinned dev branch itself.
    if tracks_structure(git, root, &remote_ref(&pin)) != Some(false) {
        return;
    }
    let finish = |db: &AppDb, imp: &mut ss::SharedImport| {
        if !root.join(STRUCTURE_REL_PATH).exists() {
            imp.pinned_dev_branch = None;
            let _ = ss::write_shared_import(db, repo.id, imp);
        }
    };
    let Some(own) = own_branch_of(db, repo.id) else {
        finish(db, &mut imp);
        return;
    };
    let sdh = vec![STRUCTURE_REL_PATH.to_string(), REQUESTS_REL_DIR.to_string()];
    let login = db.github_login().unwrap_or_default();
    let remote = remote_ref(&own);
    let cur = current_branch(git, root).ok().flatten();
    let is_current = cur.as_deref().is_some_and(|c| same_branch(c, &own));
    if tracks_structure(git, root, &local_ref(&own)) != Some(true)
        && tracks_structure(git, root, &remote) != Some(true)
    {
        finish(db, &mut imp);
        return;
    }

    // Bring a local copy that only lags up to origin's first (the owner may have
    // removed the file there already; a contributor may have pushed more).
    let local = git_sync::rev_parse(git, root, &local_ref(&own))
        .ok()
        .flatten();
    let remote_sha = git_sync::rev_parse(git, root, &remote).ok().flatten();
    if let Some(remote_sha) = &remote_sha {
        match &local {
            None => {
                let _ = git_sync::fast_forward_ref(git, root, &own, &remote);
            }
            Some(l) if l != remote_sha => {
                if git_sync::is_ancestor(git, root, l, remote_sha).unwrap_or(false) {
                    if is_current {
                        if !shared_paths_dirty(git, root, &sdh).unwrap_or(true) {
                            let _ = git_sync::merge_ff_only(git, root, &remote);
                        }
                    } else {
                        let _ = git_sync::fast_forward_ref(git, root, &own, &remote);
                    }
                }
            }
            _ => {}
        }
    }
    if tracks_structure(git, root, &local_ref(&own)) == Some(true) {
        let removed = if is_current {
            remove_sdh_from_checked_out(git, root, &own, &sdh, &login)
        } else {
            remove_sdh_from_branch(git, root, &own, &sdh, &login)
        };
        if !matches!(removed, Ok(true)) {
            return;
        }
    }
    if net_ok && remote_sha.is_some() {
        let _ = git_sync::push_branches(git, root, REMOTE, &[own.as_str()]);
    }
    if tracks_structure(git, root, &local_ref(&own)) != Some(true)
        && tracks_structure(git, root, &remote) != Some(true)
    {
        finish(db, &mut imp);
    }
}

/// The owner's deletion of `.sdh/` was committed but not pushed: push it now.
/// The repo is not shared any more; this is the only thing left to do for it.
fn retry_disabling(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
    root: &Path,
) -> Option<RepoPass> {
    let imp = ss::read_shared_import(db, repo.id);
    let dev = imp.disabling.clone()?;
    let mut pass = new_pass(repo, root, "");
    if !net_slot(NetKind::Push, repo.id, root, min_net_interval()) {
        return None;
    }
    // The comparison below and the push both need an up-to-date view of origin.
    if let Err(e) = git_sync::fetch(git, root, REMOTE) {
        pass.note(SharedStatus::FetchFailed, e.to_string());
        return Some(pass);
    }
    // The dev branch and the participant branches whose deletion is pending.
    let mut branches: Vec<String> = vec![dev.clone()];
    branches.extend(imp.disabling_branches.iter().cloned());
    let branches = clean_recipients(branches);
    let sdh = vec![STRUCTURE_REL_PATH.to_string(), REQUESTS_REL_DIR.to_string()];
    let login = db.github_login().unwrap_or_default();
    let mut to_push: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for b in &branches {
        // A branch origin moved past meanwhile (a contributor pushed first) is
        // brought up to origin's copy and the deletion is made again on top, so
        // the push is a fast-forward and not a permanent rejection.
        if let Err(e) = refresh_pending_removal(git, root, b, &sdh, &login) {
            failures.push(format!("{}: {}", b, e));
            continue;
        }
        let Ok(Some(local)) = git_sync::rev_parse(git, root, &local_ref(b)) else {
            continue; // the branch is gone: nothing to push
        };
        let remote = git_sync::rev_parse(git, root, &remote_ref(b))
            .ok()
            .flatten();
        if remote.as_deref() != Some(local.as_str()) {
            to_push.push(b.clone());
        }
    }
    if to_push.is_empty() {
        if failures.is_empty() {
            let _ = clear_disabling(db, repo.id);
            return None;
        }
        pass.note(SharedStatus::PushFailed, failures.join("; "));
        return Some(pass);
    }
    let refs: Vec<&str> = to_push.iter().map(String::as_str).collect();
    match git_sync::push_branches(git, root, REMOTE, &refs) {
        Ok(results)
            if failures.is_empty()
                && results
                    .iter()
                    .all(|(_, o)| matches!(o, PushOutcome::Pushed | PushOutcome::UpToDate)) =>
        {
            let _ = clear_disabling(db, repo.id);
            None
        }
        Ok(results) => {
            failures.push(format!("{:?}", results));
            pass.note(SharedStatus::PushFailed, failures.join("; "));
            Some(pass)
        }
        Err(e) => {
            failures.push(e.to_string());
            pass.note(SharedStatus::PushFailed, failures.join("; "));
            Some(pass)
        }
    }
}

/// Before a pending deletion is pushed again: if origin moved past the local copy
/// of `branch`, bring the local copy up to origin's (fast-forward when it only
/// lags; when it has diverged, reset it -- keeping a backup -- provided every
/// local-only commit is ours) and make the deletion again on top. A branch that
/// is equal to or ahead of origin is pushable as it is.
fn refresh_pending_removal(
    git: &GitBinary,
    root: &Path,
    branch: &str,
    sdh: &[String],
    login: &str,
) -> Result<(), String> {
    let remote = remote_ref(branch);
    let (Some(remote_sha), Some(local_sha)) = (
        git_sync::rev_parse(git, root, &remote)?,
        git_sync::rev_parse(git, root, &local_ref(branch))?,
    ) else {
        return Ok(());
    };
    if local_sha == remote_sha || git_sync::is_ancestor(git, root, &remote_sha, &local_sha)? {
        return Ok(());
    }
    let is_current = current_branch(git, root)?
        .as_deref()
        .is_some_and(|c| same_branch(c, branch));
    if git_sync::is_ancestor(git, root, &local_sha, &remote_sha)? {
        if is_current {
            git_sync::merge_ff_only(git, root, &remote)?;
        } else {
            git_sync::fast_forward_ref(git, root, branch, &remote)?;
        }
    } else {
        if is_current {
            return Err(format!(
                "it has diverged from origin/{}: merge it first",
                branch
            ));
        }
        let tip = local_tip_if_ours(git, root, branch)?
            .ok_or_else(|| "it holds commits that are not sync commits".to_string())?;
        if !reset_to_origin(git, root, branch, &tip)? {
            return Err("it moved meanwhile".into());
        }
    }
    if is_current {
        remove_sdh_from_checked_out(git, root, branch, sdh, login)?;
    } else {
        remove_sdh_from_branch(git, root, branch, sdh, login)?;
    }
    Ok(())
}

fn clear_disabling(db: &AppDb, repo_id: i64) -> Result<(), String> {
    let mut imp = ss::read_shared_import(db, repo_id);
    imp.disabling = None;
    imp.disabling_branches.clear();
    ss::write_shared_import(db, repo_id, &imp)?;
    db.set_shared_status(repo_id, None)
        .map_err(|e| e.to_string())
}

fn register_driver(git: &GitBinary, pass: &mut RepoPass) {
    if let Err(e) = register_merge_driver(git, &pass.root) {
        let issue = repo_issue(Code::ManagedFileFailed, &pass.repo)
            .target(".git/config")
            .detail(e);
        pass.issue(issue);
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

/// Р16 / BUG-1: delete the rule copies in `docs/sdh_skills/` that this install
/// rendered itself -- `sdh-*.md` files git does not track. A contributor's
/// install renders none there any more, and the owner's committed ones cannot
/// be fast-forwarded over untracked files. Approval markers, reason files and
/// anything else are left alone. Skipped when git cannot say what is tracked.
fn drop_untracked_rule_copies(git: &GitBinary, root: &Path) {
    let dir = root.join(SKILLS_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let Ok(tracked) = ls_files_tracked(git, root, &[SKILLS_DIR.to_string()]) else {
        return;
    };
    let tracked: HashSet<String> = tracked.into_iter().collect();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("sdh-") && name.ends_with(".md")) {
            continue;
        }
        if !matches!(entry.file_type(), Ok(t) if t.is_file()) {
            continue;
        }
        if !tracked.contains(&format!("{}/{}", SKILLS_DIR, name)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Move the local branch back onto origin's, keeping the old tip in
/// `refs/sdh-backup/<branch>`. `observed` is the tip `local_tip_if_ours` judged
/// to hold nothing but our own sync commits: it is the compare-and-swap old
/// value of both ref updates, so a branch that moved since the judgement is
/// left alone. The branch must not be checked out in any worktree (re-checked
/// here). `Ok(false)` when the ref moved meanwhile.
fn reset_to_origin(
    git: &GitBinary,
    root: &Path,
    branch: &str,
    observed: &str,
) -> Result<bool, String> {
    git_sync::refuse_checked_out(git, root, branch, "it cannot be reset")?;
    let target = git_sync::rev_parse(git, root, &remote_ref(branch))?
        .ok_or_else(|| format!("origin/{} does not exist", branch))?;
    let backup = format!("refs/sdh-backup/{}", branch);
    let old_backup = git_sync::rev_parse(git, root, &backup)?;
    if !git_sync::update_ref_cas(git, root, &backup, observed, old_backup.as_deref())? {
        return Ok(false);
    }
    git_sync::update_ref_cas(git, root, &local_ref(branch), &target, Some(observed))
}

/// Fast-forward the recipients, guard the non-current ones, then the import hook.
/// Nothing is touched unless the repo is `repo_ready`.
fn after_fetch(db: &AppDb, git: &GitBinary, pass: &mut RepoPass) {
    let Some(role) = pass.role else { return };
    if !repo_ready(git, &pass.root) {
        return;
    }
    let cur = current_branch(git, &pass.root).ok().flatten();
    let pathspec = shared_pathspec(role, &pass.login);
    let recipients = pass.recipients.clone();
    for b in &recipients {
        let remote = remote_ref(b);
        let has_remote = matches!(git_sync::rev_parse(git, &pass.root, &remote), Ok(Some(_)));
        let is_current = cur.as_deref().is_some_and(|c| same_branch(c, b));
        if is_current {
            if !has_remote {
                continue;
            }
            match ff_current(git, &pass.root, &remote, &pathspec) {
                Ok(true) => {}
                Ok(false) => pass.note_diverged(b, true),
                Err(e) => {
                    let issue = repo_issue(Code::AutocommitFailed, &pass.repo).detail(e);
                    pass.issue(issue)
                }
            }
            continue;
        }
        // Not the current branch: it may only ever hold our own sync commits.
        let mut check = true;
        if has_remote {
            match git_sync::fast_forward_ref(git, &pass.root, b, &remote) {
                Ok(FfOutcome::Diverged) => {
                    check = false;
                    let tip = if pass.fetched {
                        local_tip_if_ours(git, &pass.root, b).unwrap_or(None)
                    } else {
                        None
                    };
                    let reset = tip
                        .is_some_and(|t| reset_to_origin(git, &pass.root, b, &t).unwrap_or(false));
                    if !reset {
                        pass.blocked.insert(b.to_ascii_lowercase());
                        pass.note_diverged(b, false);
                    }
                }
                Ok(FfOutcome::LocalAhead) => {}
                Ok(_) => check = false,
                Err(e) => {
                    check = false;
                    let issue = repo_issue(Code::AutocommitFailed, &pass.repo).detail(e);
                    pass.issue(issue)
                }
            }
        }
        let exists = matches!(
            git_sync::rev_parse(git, &pass.root, &local_ref(b)),
            Ok(Some(_))
        );
        if check && exists {
            match local_only_commits_are_ours(git, &pass.root, b) {
                Ok(true) => {}
                Ok(false) => {
                    pass.blocked.insert(b.to_ascii_lowercase());
                    pass.note_diverged(b, false);
                }
                Err(e) => {
                    pass.blocked.insert(b.to_ascii_lowercase());
                    let issue = repo_issue(Code::AutocommitFailed, &pass.repo).detail(e);
                    pass.issue(issue)
                }
            }
        }
    }
    if let Err(e) = import_incoming(db, git, pass.repo.id, &pass.root) {
        // Recurring: `fresh_issues` reports it once until the text changes.
        let issue = repo_issue(Code::ManagedFileFailed, &pass.repo)
            .target("docs/bug-reports.md")
            .detail(format!("import of the shared task files: {}", e));
        pass.issue(issue)
    }
    // T-000224 (В4): the owner's committed rule copies are this install's canon.
    if role == Role::Contributor {
        let owner = pass
            .structure
            .as_ref()
            .map(|s| (s.main.login.clone(), s.main.dev_branch.clone()));
        if let Some((main_login, dev_branch)) = owner {
            match crate::sync::shared_canon::adopt_main_canon(
                db,
                git,
                &pass.repo,
                &pass.root,
                &main_login,
                &dev_branch,
            ) {
                Ok(done) => {
                    pass.canon_changed.extend(done.changed);
                    if !done.errors.is_empty() {
                        let issue = repo_issue(Code::ManagedFileFailed, &pass.repo)
                            .target("docs/sdh_skills/")
                            .detail(format!(
                                "canon of the owner's rules: {}",
                                done.errors.join("; ")
                            ));
                        pass.issue(issue)
                    }
                }
                Err(e) => {
                    let issue = repo_issue(Code::ManagedFileFailed, &pass.repo)
                        .target("docs/sdh_skills/")
                        .detail(format!("canon of the owner's rules: {}", e));
                    pass.issue(issue)
                }
            }
        }
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
        let mut out = SharedOutcome::default();
        for mut pass in passes {
            if let (Some(git), Some(_)) = (git, pass.role) {
                phase_b_repo(db, git, &mut pass);
            }
            let issues = std::mem::take(&mut pass.issues);
            out.issues
                .extend(fresh_issues(pass.repo.id, &pass.root, issues));
            if pass.committed {
                let name = pass.repo.display_name();
                if !out.committed.contains(&name) {
                    out.committed.push(name);
                }
            }
            settle_status(db, &pass, &mut out);
        }
        out
    }
}

fn settle_status(db: &AppDb, pass: &RepoPass, out: &mut SharedOutcome) {
    let best = pass.notes.iter().min_by_key(|n| n.rank());
    let new = best.map(|n| n.status.as_str());
    let new_detail = best.and_then(|n| n.ui_detail());
    let old_state = db.get_shared_state(pass.repo.id).ok();
    let old = old_state.as_ref().and_then(|s| s.shared_status.clone());
    if old.as_deref() == new {
        // The code is the same; its detail may still have moved (another
        // branch, another reason): keep it fresh without announcing again.
        let old_detail = old_state.and_then(|s| s.shared_status_detail);
        if new.is_some() && old_detail != new_detail {
            let _ = db.set_shared_status_with_detail(pass.repo.id, new, new_detail.as_deref());
        }
        return;
    }
    let _ = db.set_shared_status_with_detail(pass.repo.id, new, new_detail.as_deref());
    if let Some(n) = best {
        let mut issue = repo_issue(n.status.issue_code(), &pass.repo);
        if !n.detail.is_empty() {
            issue = issue.detail(&n.detail);
        }
        out.issues.push(issue);
    }
}

/// What a plumbed commit may touch: the role's shared files without the managed
/// root files, which are committed on the current branch only.
fn plumb_pathspec(role: Role, login: &str) -> Vec<String> {
    shared_pathspec(role, login)
        .into_iter()
        .filter(|p| !MANAGED_ROOT_FILES.contains(&p.as_str()))
        .collect()
}

fn phase_b_repo(db: &AppDb, git: &GitBinary, pass: &mut RepoPass) {
    let Some(role) = pass.role else { return };
    // Same guard as phase A, checked again: the repo may have changed since.
    if !repo_ready(git, &pass.root) {
        return;
    }
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
                let issue = repo_issue(Code::ManagedFileFailed, &pass.repo)
                    .target(STRUCTURE_REL_PATH)
                    .detail(e);
                pass.issue(issue);
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
                    let issue = repo_issue(Code::ManagedFileFailed, &pass.repo)
                        .target(REQUESTS_REL_DIR)
                        .detail(e);
                    pass.issue(issue);
                }
            }
        }
        Role::Contributor => {}
    }

    if pass.recipients.is_empty() {
        return;
    }
    let pathspec = shared_pathspec(role, &login);
    let plumb_spec = plumb_pathspec(role, &login);
    let approved: Vec<String> = match (role, &pass.structure) {
        (Role::Main, Some(s)) => s.participants.iter().map(|p| p.login.clone()).collect(),
        _ => Vec::new(),
    };
    let active = match active_paths(git, &root, &pathspec) {
        Ok(a) => a,
        Err(e) => {
            let issue = repo_issue(Code::AutocommitFailed, &pass.repo).detail(e);
            pass.issue(issue);
            return;
        }
    };
    let cur = current_branch(git, &root).ok().flatten();
    let head_born = matches!(git_sync::rev_parse(git, &root, "HEAD"), Ok(Some(_)));
    if !head_born {
        return;
    }

    let recipients = pass.recipients.clone();
    for b in &recipients {
        let is_current = cur.as_deref().is_some_and(|c| same_branch(c, b));
        if is_current {
            if active.is_empty() {
                continue;
            }
            let mut paths: Vec<PathBuf> = active.iter().map(PathBuf::from).collect();
            paths.extend(exclude_specs(&active).into_iter().map(PathBuf::from));
            let (subject, body) = commit_message(&login);
            let pre_head = git_sync::rev_parse(git, &root, "HEAD").ok().flatten();
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
                Ok(CommitOutcome::Committed { .. }) => {
                    pass.committed = true;
                    // Our own content: it needs no import (spec R11 step 3).
                    record_own_commit(db, git, &root, pass.repo.id, pre_head.as_deref());
                }
                Ok(_) => {}
                Err(e) => {
                    let issue = repo_issue(Code::AutocommitFailed, &pass.repo).detail(e);
                    pass.issue(issue)
                }
            }
        } else {
            if pass.is_blocked(b) {
                continue;
            }
            // The branch must exist locally (phase A creates it from origin).
            if !matches!(git_sync::rev_parse(git, &root, &local_ref(b)), Ok(Some(_))) {
                continue;
            }
            match plumb_into_branch(git, &root, b, &plumb_spec, &login, &approved) {
                Ok(BranchCommit::Committed { .. }) => pass.committed = true,
                // RefMoved: left for the next sync.
                Ok(_) => {}
                Err(e) => {
                    let issue = repo_issue(Code::AutocommitFailed, &pass.repo).detail(e);
                    pass.issue(issue)
                }
            }
        }
    }

    if pass.net_ok {
        push_recipients(git, pass, cur.as_deref());
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

/// Push the recipient branches whose tip origin does not have. A non-current
/// branch that holds anything but our own sync commits is never pushed.
fn push_recipients(git: &GitBinary, pass: &mut RepoPass, cur: Option<&str>) {
    let root = pass.root.clone();
    let mut to_push: Vec<String> = Vec::new();
    for b in pass.recipients.clone() {
        let is_current = cur.is_some_and(|c| same_branch(c, &b));
        if !is_current && pass.is_blocked(&b) {
            continue;
        }
        let Ok(Some(local)) = git_sync::rev_parse(git, &root, &local_ref(&b)) else {
            continue;
        };
        let remote = git_sync::rev_parse(git, &root, &remote_ref(&b))
            .ok()
            .flatten();
        if remote.as_deref() == Some(local.as_str()) {
            continue;
        }
        if !is_current && !local_only_commits_are_ours(git, &root, &b).unwrap_or(false) {
            pass.blocked.insert(b.to_ascii_lowercase());
            pass.note_diverged(&b, false);
            continue;
        }
        to_push.push(b);
    }
    if to_push.is_empty() {
        return;
    }
    if !net_slot(NetKind::Push, pass.repo.id, &root, min_net_interval()) {
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
                        let current = cur.is_some_and(|c| same_branch(c, &branch));
                        pass.note_diverged(&branch, current)
                    }
                    PushOutcome::RejectedProtected(m) => pass.note_push_rejected(&branch, m),
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
    // T-000227: the owner or a verified administrator only (the frontend
    // verifies the install's own login against GitHub before it asks).
    if let (Some(gh), Some(login)) = (repo.github_name.as_deref(), db.github_login()) {
        if !ss::can_enable_shared(gh, &login, &ss::AdminVerified::load(db)) {
            return Err(ss::refusal_not_owner_or_admin(&login, gh));
        }
    }
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
    let existing = ss::read_structure_file(&root, &gh, &ss::AdminVerified::load(db))?;
    // Validates: owner or administrator, project, dev branch.
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
    // A deletion of `.sdh/` that never reached the remote is obsolete now: the
    // owner wants sharing on again, so it must not be pushed later.
    let mut imp = ss::read_shared_import(db, repo_id);
    if imp.disabling.take().is_some() {
        ss::write_shared_import(db, repo_id, &imp)?;
    }
    Ok(())
}

/// Tracked files only (no untracked ones) under `paths`.
pub(super) fn ls_files_tracked(
    git: &GitBinary,
    root: &Path,
    paths: &[String],
) -> Result<Vec<String>, String> {
    Ok(split_nul(&git_bytes(
        git,
        root,
        &["ls-files", "-z"],
        paths,
    )?))
}

fn remove_sdh_files(root: &Path) {
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
}

/// Switch shared mode off (spec R15). Only the owner (shared mode on, role main)
/// can. The deletion of the tracked `.sdh/project.json` and `.sdh/requests/*` is
/// committed HERE on the dev branch, after a fetch: once the switch is off the
/// sync no longer treats the repo as shared. The database is flipped LAST, after
/// the commit succeeded. If the push fails the repo keeps a `push_failed` status
/// and a `disabling` marker, and the next sync retries the push until the
/// deletion reached origin. With nothing tracked (never synced) only the files
/// are removed.
pub fn disable_shared_mode(
    db: &AppDb,
    git: &GitBinary,
    repo_id: i64,
) -> Result<SharedDisableResult, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    let state = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    if !state.shared_mode || state.shared_role.as_deref().is_some_and(|r| r != "main") {
        return Err("shared mode can only be switched off by the owner of the repo".into());
    }
    let sdh = vec![STRUCTURE_REL_PATH.to_string(), REQUESTS_REL_DIR.to_string()];
    let tracked = ls_files_tracked(git, &root, &sdh)?;
    // The participants' branches carry `.sdh/` too (the sync plumbed it there):
    // read them before the file is gone.
    let participant_branches: Vec<String> = repo
        .github_name
        .as_deref()
        .and_then(|gh| {
            ss::read_structure_file(&root, gh, &ss::AdminVerified::load(db))
                .ok()
                .flatten()
        })
        .map(|s| s.participants.iter().map(|p| p.branch.clone()).collect())
        .unwrap_or_default();
    let dev = own_branch_of(db, repo_id);
    let fetch_ok = git_sync::fetch(git, &root, REMOTE).is_ok();
    let login = db.github_login().unwrap_or_default();
    let (_, body) = commit_message(&login);
    let subject = REMOVAL_SUBJECT;
    let mut committed = false;

    if tracked.is_empty() {
        remove_sdh_files(&root);
    } else {
        let dev = dev.clone().ok_or_else(|| {
            "no dev branch is set: cannot commit the removal of .sdh/".to_string()
        })?;
        let cur = current_branch(git, &root).ok().flatten();
        if cur.as_deref().is_some_and(|c| same_branch(c, &dev)) {
            remove_sdh_files(&root);
            let paths: Vec<PathBuf> = tracked.iter().map(PathBuf::from).collect();
            match commit_paths(
                git,
                &root,
                &dev,
                &paths,
                subject,
                Some(&body),
                SDH_COMMIT_NAME,
                SDH_COMMIT_EMAIL,
            ) {
                Ok(o) => committed = matches!(o, CommitOutcome::Committed { .. }),
                Err(e) => {
                    // Put the files back: nothing was switched off.
                    let _ = git_bytes(git, &root, &["checkout", "HEAD"], &tracked);
                    return Err(e);
                }
            }
        } else {
            if !matches!(
                git_sync::rev_parse(git, &root, &local_ref(&dev)),
                Ok(Some(_))
            ) {
                return Err(format!("the dev branch '{}' does not exist locally", dev));
            }
            if matches!(
                git_sync::rev_parse(git, &root, &remote_ref(&dev)),
                Ok(Some(_))
            ) && git_sync::fast_forward_ref(git, &root, &dev, &remote_ref(&dev))?
                == FfOutcome::Diverged
            {
                return Err(format!(
                    "{} has diverged from origin/{}: merge it first",
                    dev, dev
                ));
            }
            let in_branch = ls_tree(git, &root, &local_ref(&dev), &sdh)?;
            let changes: Vec<FileChange> = in_branch
                .keys()
                .map(|p| FileChange {
                    rel_path: PathBuf::from(p),
                    content: None,
                })
                .collect();
            let res = git_sync::commit_files_to_branch(
                git,
                &root,
                &dev,
                &changes,
                &format!("{}\n\n{}", subject, body),
                SDH_COMMIT_NAME,
                SDH_COMMIT_EMAIL,
            )?;
            if matches!(res, BranchCommit::RefMoved) {
                return Err("the dev branch moved meanwhile: try again".into());
            }
            committed = matches!(res, BranchCommit::Committed { .. });
            remove_sdh_files(&root);
        }
    }

    // The commit is done: now the switch.
    db.set_shared_mode(repo_id, false)
        .map_err(|e| e.to_string())?;
    db.set_shared_role(repo_id, None)
        .map_err(|e| e.to_string())?;
    db.set_shared_status(repo_id, None)
        .map_err(|e| e.to_string())?;

    // The same deletion on every participant branch (BUG-3): a contributor
    // whose own branch kept `.sdh/project.json` would otherwise carry it for
    // good. Best effort per branch -- a branch that cannot be brought forward is
    // reported and left to the contributor's own clean-up.
    let mut extra: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    if !tracked.is_empty() {
        for b in clean_recipients(participant_branches) {
            if dev.as_deref().is_some_and(|d| same_branch(d, &b)) {
                continue;
            }
            match remove_sdh_from_branch(git, &root, &b, &sdh, &login) {
                Ok(true) => extra.push(b),
                Ok(false) => {}
                Err(e) => skipped.push(format!("{}: {}", b, e)),
            }
        }
    }
    let skipped_note = (!skipped.is_empty()).then(|| {
        format!(
            "`.sdh/` was not removed from participant branch(es): {}",
            skipped.join("; ")
        )
    });

    let dev = dev.filter(|_| committed);
    let mut to_push: Vec<String> = dev.iter().cloned().collect();
    to_push.extend(extra.iter().cloned());
    let Some(marker) = to_push.first().cloned() else {
        return Ok(SharedDisableResult {
            pushed: false,
            note: Some(match skipped_note {
                Some(s) => format!("nothing to commit; {}", s),
                None => "nothing to commit".into(),
            }),
        });
    };
    let refs: Vec<&str> = to_push.iter().map(String::as_str).collect();
    let failure = match git_sync::push_branches(git, &root, REMOTE, &refs) {
        Ok(results)
            if results
                .iter()
                .all(|(_, o)| matches!(o, PushOutcome::Pushed | PushOutcome::UpToDate)) =>
        {
            let mut notes: Vec<String> = Vec::new();
            if !fetch_ok {
                notes.push("fetch failed before committing".into());
            }
            notes.extend(skipped_note);
            return Ok(SharedDisableResult {
                pushed: true,
                note: (!notes.is_empty()).then(|| notes.join("; ")),
            });
        }
        Ok(results) => format!("push of {} was refused: {:?}", to_push.join(", "), results),
        Err(e) => format!("push failed: {}", e),
    };
    // Keep the repo in the shared pass until the deletion reaches origin.
    let mut imp = ss::read_shared_import(db, repo_id);
    imp.disabling = Some(marker);
    imp.disabling_branches = extra;
    ss::write_shared_import(db, repo_id, &imp)?;
    db.set_shared_status(repo_id, Some(SharedStatus::PushFailed.as_str()))
        .map_err(|e| e.to_string())?;
    Ok(SharedDisableResult {
        pushed: false,
        note: Some(format!("{} - it is retried on the next sync", failure)),
    })
}

/// Delete `.sdh/` from the local copy of `branch` (a branch that is not checked
/// out) with plumbing, first bringing it up to origin's copy. `Ok(false)` when
/// there is nothing to delete. Refused when the branch holds commits that are
/// not sync commits or has diverged from origin.
fn remove_sdh_from_branch(
    git: &GitBinary,
    root: &Path,
    branch: &str,
    sdh: &[String],
    login: &str,
) -> Result<bool, String> {
    let remote = remote_ref(branch);
    let has_remote = matches!(git_sync::rev_parse(git, root, &remote), Ok(Some(_)));
    if has_remote && git_sync::fast_forward_ref(git, root, branch, &remote)? == FfOutcome::Diverged
    {
        return Err(format!("it has diverged from origin/{}", branch));
    }
    if !matches!(
        git_sync::rev_parse(git, root, &local_ref(branch)),
        Ok(Some(_))
    ) {
        return Ok(false);
    }
    let in_branch = ls_tree(git, root, &local_ref(branch), sdh)?;
    if in_branch.is_empty() {
        return Ok(false);
    }
    if !local_only_commits_are_ours(git, root, branch)? {
        return Err("it holds commits that are not sync commits".into());
    }
    let changes: Vec<FileChange> = in_branch
        .keys()
        .map(|p| FileChange {
            rel_path: PathBuf::from(p),
            content: None,
        })
        .collect();
    let (_, body) = commit_message(login);
    let res = git_sync::commit_files_to_branch(
        git,
        root,
        branch,
        &changes,
        &format!(
            "chore(sdh-sync): remove shared project structure\n\n{}",
            body
        ),
        SDH_COMMIT_NAME,
        SDH_COMMIT_EMAIL,
    )?;
    match res {
        BranchCommit::Committed { .. } => Ok(true),
        BranchCommit::NothingToCommit => Ok(false),
        BranchCommit::RefMoved => Err("it moved meanwhile".into()),
    }
}

/// Join requests in the work tree plus, when the owner's current branch is not
/// the dev branch, those published on `origin/<dev>`.
fn all_join_requests(
    db: &AppDb,
    git: Option<&GitBinary>,
    root: &Path,
    gh: &str,
    dev: Option<&str>,
) -> Vec<JoinRequest> {
    let admins = ss::AdminVerified::load(db);
    let structure = ss::read_structure_file(root, gh, &admins).ok().flatten();
    let mut out = ss::list_requests(root, gh, &admins);
    let (Some(git), Some(dev)) = (git, dev) else {
        return out;
    };
    let cur = current_branch(git, root).ok().flatten();
    if cur.as_deref().is_some_and(|c| same_branch(c, dev)) {
        return out;
    }
    let remote = remote_ref(dev);
    let Ok(tree) = ls_tree(git, root, &remote, &[REQUESTS_REL_DIR.to_string()]) else {
        return out;
    };
    for path in tree.keys().take(ss::MAX_REQUEST_FILES) {
        let Some(stem) = path
            .rsplit('/')
            .next()
            .and_then(|n| n.strip_suffix(".json"))
        else {
            continue;
        };
        let Ok(Some(bytes)) =
            git_sync::show_file_capped(git, root, &remote, path, ss::MAX_REQUEST_FILE_BYTES)
        else {
            continue;
        };
        let Some(req) = ss::request_from_bytes(stem, &bytes, structure.as_ref()) else {
            continue;
        };
        if !out.iter().any(|r| ss::logins_equal(&r.login, &req.login)) {
            out.push(req);
        }
    }
    out.sort_by_key(|r| r.login.to_ascii_lowercase());
    out
}

/// Main: approve the join request of `login` (from the work tree or, when the
/// owner is on another branch, from `origin/<dev>`). The structure file gains
/// the participant (with a number band) and a work-tree request file is
/// deleted; the next sync commits the structure and prunes the request from
/// the dev branch.
pub fn approve_join_request(
    db: &AppDb,
    git: Option<&GitBinary>,
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
    let existing = ss::read_structure_file(&root, &gh, &ss::AdminVerified::load(db))?;
    let base = ss::build_structure(db, repo_id, &me, existing.as_ref())?;
    let req = all_join_requests(db, git, &root, &gh, Some(&base.main.dev_branch))
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
///
/// Only offered for what the status says: a `shared_untrusted` status whose
/// detail is a changed dev branch, on an install that is not the owner.
pub fn accept_new_main_branch(db: &AppDb, repo_id: i64) -> Result<(), String> {
    let st = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    if st.shared_mode || st.shared_role.as_deref() == Some("main") {
        return Err("only a contributor can accept a new dev branch of the owner".into());
    }
    let changed = st.shared_status.as_deref() == Some(SharedStatus::Untrusted.as_str())
        && st
            .shared_status_detail
            .as_deref()
            .is_some_and(|d| d.starts_with(ss::DEV_BRANCH_CHANGED_PREFIX));
    if !changed {
        return Err("the owner's dev branch has not changed: nothing to accept".into());
    }
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

pub fn list_join_requests(
    db: &AppDb,
    git: Option<&GitBinary>,
    repo_id: i64,
) -> Result<Vec<JoinRequest>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let (Some(lp), Some(gh)) = (repo.local_path.as_deref(), repo.github_name.as_deref()) else {
        return Ok(Vec::new());
    };
    let dev = own_branch_of(db, repo_id);
    Ok(all_join_requests(
        db,
        git,
        Path::new(lp),
        gh,
        dev.as_deref(),
    ))
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
    let admins = ss::AdminVerified::load(db);
    if let Some(s) = ss::read_structure_file(&root, &gh, &admins)? {
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
        match ss::discover_structure(git, &root, &gh, pinned.as_deref(), &admins)? {
            DiscoverOutcome::Found(s, _) => Some(s),
            _ => None,
        },
    )
}

/// Unit separator between the commit time and the trailer values in the log
/// format of `participants_activity`.
const ACTIVITY_SEP: char = '\u{1f}';

/// The newest commit time among the lines of `git log --format=%cI%x1f<trailers>`
/// whose `SDH-Participant` trailer names `login` (case-insensitive). Several
/// trailer values on one line are comma-separated. Malformed lines and
/// unparsable times are skipped.
fn latest_participant_commit(log: &str, login: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    log.lines()
        .filter_map(|line| {
            let (time, trailers) = line.split_once(ACTIVITY_SEP)?;
            if !trailers
                .split(',')
                .any(|t| !t.trim().is_empty() && ss::logins_equal(t.trim(), login))
            {
                return None;
            }
            chrono::DateTime::parse_from_rfc3339(time.trim())
                .ok()
                .map(|t| t.with_timezone(&chrono::Utc))
        })
        .max()
}

/// The last `HISTORY_CAP` commits of `origin/<branch>` as `time<US>trailers`
/// lines; empty when the ref is missing or git fails.
fn activity_log(git: &GitBinary, root: &Path, branch: &str) -> String {
    let cap = format!("-n{}", HISTORY_CAP);
    let rev = format!("{}/{}", REMOTE, branch);
    let Ok(out) = spawn_cmd(&git.0)
        .args([
            "log",
            cap.as_str(),
            "--format=%cI%x1f%(trailers:key=SDH-Participant,valueonly,separator=%x2C)",
            rev.as_str(),
            "--",
        ])
        .current_dir(root)
        .output()
    else {
        return String::new();
    };
    if !out.status.success() {
        return String::new();
    }
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Owner's view (T-000237): per participant, the time of the newest commit with
/// their `SDH-Participant` trailer on their own branch or on the owner's dev
/// branch. Does not fetch: the sync does.
pub fn participants_activity(
    db: &AppDb,
    git: &GitBinary,
    repo_id: i64,
) -> Result<Vec<ParticipantActivity>, String> {
    let Some(structure) = get_repo_structure(db, Some(git), repo_id)? else {
        return Ok(Vec::new());
    };
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(lp) = repo.local_path.as_deref() else {
        return Ok(Vec::new());
    };
    let root = Path::new(lp);
    let dev_log = activity_log(git, root, &structure.main.dev_branch);
    Ok(structure
        .participants
        .iter()
        .map(|p| {
            let own_log = activity_log(git, root, &p.branch);
            let last = latest_participant_commit(&own_log, &p.login)
                .into_iter()
                .chain(latest_participant_commit(&dev_log, &p.login))
                .max();
            ParticipantActivity {
                login: p.login.clone(),
                last_at: last.map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            }
        })
        .collect())
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
        pub project_id: i64,
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

        pub fn main_status_detail(&self) -> Option<String> {
            self.main_db
                .get_shared_state(self.main_repo)
                .unwrap()
                .shared_status_detail
        }

        pub fn helper_status_detail(&self) -> Option<String> {
            self.helper_db
                .get_shared_state(self.helper_repo)
                .unwrap()
                .shared_status_detail
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
            approve_join_request(&self.main_db, Some(&gb()), self.main_repo, "helper").unwrap();
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
            approve_join_request(&self.main_db, Some(&gb()), self.main_repo, "helper").unwrap();
            self.sync_main(); // approval committed + pushed
            self.sync_helper(); // contributor now
        }
    }

    pub fn sync_once(db: &AppDb) -> SharedOutcome {
        let git = gb();
        let project = db
            .list_all_repos()
            .unwrap()
            .into_iter()
            .find_map(|r| r.project_id);
        run_phase_a(db, Some(&git), project).finish(db, Some(&git))
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
            project_id: project.id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use std::fs;

    // ----- T-000227: administrator checks and eligibility ---------------------

    fn org_db(login: &str) -> (AppDb, Repository) {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.set_setting("github_login", login).unwrap();
        let repo = db
            .upsert_repository("Acme-Org/web", None, None, None, None, None)
            .unwrap();
        (db, repo)
    }

    #[test]
    fn a_pass_asks_for_admin_checks_only_when_main_is_not_the_owner() {
        let (_db, repo) = org_db("boss");
        let root = PathBuf::from("unused");
        let mut pass = new_pass(&repo, &root, "boss");
        // trusted by ownership: nothing rests on the API
        pass.ask_admin_check("Owner/web", "owner");
        assert!(pass.admin_checks.is_empty());
        // not the owner: asked once, however often and in whatever case
        pass.ask_admin_check("Acme-Org/web", "boss");
        pass.ask_admin_check("acme-org/WEB", "BOSS");
        pass.ask_admin_check("Acme-Org/web", "second-admin");
        let asked: Vec<_> = pass
            .admin_checks
            .iter()
            .map(|c| (c.repo.as_str(), c.login.as_str()))
            .collect();
        assert_eq!(
            asked,
            vec![("Acme-Org/web", "boss"), ("Acme-Org/web", "second-admin")]
        );
    }

    #[test]
    fn preflight_refuses_a_non_owner_without_a_verification() {
        let (db, repo) = org_db("boss");
        let err = shared_mode_preflight(&db, &gb(), repo.id).unwrap_err();
        assert!(err.contains("repo owner or an administrator"), "{}", err);
        // a non-admin answer changes nothing
        ss::record_admin_verification(&db, "Acme-Org/web", "boss", false).unwrap();
        assert!(shared_mode_preflight(&db, &gb(), repo.id)
            .unwrap_err()
            .contains("repo owner or an administrator"));
        // a verified admin gets past the eligibility check (and meets the next
        // requirement, the missing local checkout)
        ss::record_admin_verification(&db, "Acme-Org/web", "boss", true).unwrap();
        let err = shared_mode_preflight(&db, &gb(), repo.id).unwrap_err();
        assert!(!err.contains("administrator"), "{}", err);
    }

    #[test]
    fn preflight_does_not_ask_the_owner_for_a_verification() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.set_setting("github_login", "Owner").unwrap();
        let repo = db
            .upsert_repository("owner/api", None, None, None, None, None)
            .unwrap();
        let err = shared_mode_preflight(&db, &gb(), repo.id).unwrap_err();
        assert!(!err.contains("administrator"), "{}", err);
    }

    // ----- T-000227: an organization's repo (git, ignored) -------------------

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn an_org_repo_is_shared_only_through_the_admin_verification() {
        let fx = two_installs();
        fx.main_db.set_setting("github_login", "boss").unwrap();
        for (db, id) in [(&fx.main_db, fx.main_repo), (&fx.helper_db, fx.helper_repo)] {
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "UPDATE repositories SET github_name = 'Acme-Org/api' WHERE id = ?1",
                    [id],
                )
                .unwrap();
        }
        let boss = AdminCheck {
            repo: "Acme-Org/api".into(),
            login: "boss".into(),
        };

        // The main cannot switch sharing on until GitHub says he is an admin.
        let err = enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).unwrap_err();
        assert!(err.contains("administrator"), "{}", err);
        ss::record_admin_verification(&fx.main_db, "Acme-Org/api", "boss", true).unwrap();
        enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).unwrap();
        // His sync asks to re-verify him, and pushes the structure.
        let shared = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        assert_eq!(shared.admin_checks, vec![boss.clone()]);
        shared.finish(&fx.main_db, Some(&gb()));
        assert!(fx.remote_file("dev", ".sdh/project.json").is_some());

        // The helper's install does not trust the file yet, and names whom to ask.
        let probe = probe_role_less(&fx.helper_db, Some(&gb()));
        assert_eq!(probe.admin_checks, vec![boss.clone()]);
        probe.finish(&fx.helper_db, Some(&gb()));
        assert_eq!(fx.helper_status().as_deref(), Some("shared_untrusted"));
        let st = fx.helper_db.get_shared_state(fx.helper_repo).unwrap();
        assert_eq!(st.shared_role, None);

        // A non-admin answer changes nothing.
        ss::record_admin_verification(&fx.helper_db, "Acme-Org/api", "boss", false).unwrap();
        let probe = probe_role_less(&fx.helper_db, Some(&gb()));
        probe.finish(&fx.helper_db, Some(&gb()));
        assert_eq!(fx.helper_status().as_deref(), Some("shared_untrusted"));

        // Verified: the structure is trusted, the helper is a pending joiner,
        // and the check is still asked for on every sync.
        ss::record_admin_verification(&fx.helper_db, "Acme-Org/api", "boss", true).unwrap();
        let probe = probe_role_less(&fx.helper_db, Some(&gb()));
        assert_eq!(probe.admin_checks, vec![boss.clone()]);
        probe.finish(&fx.helper_db, Some(&gb()));
        let st = fx.helper_db.get_shared_state(fx.helper_repo).unwrap();
        assert_eq!(st.shared_role.as_deref(), Some("pending"));

        // Revoked: the next sync does not trust the file any more.
        ss::record_admin_verification(&fx.helper_db, "Acme-Org/api", "boss", false).unwrap();
        let out = fx.sync_helper();
        assert_eq!(fx.helper_status().as_deref(), Some("shared_untrusted"));
        drop(out);
    }

    // ----- participants activity (T-000237) --------------------------------

    fn ts(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn activity_takes_the_newest_commit_of_the_login() {
        let log = "2026-10-20T10:00:00+00:00\u{1f}helper\n\
                   2026-10-22T10:00:00+00:00\u{1f}helper\n\
                   2026-10-21T10:00:00+00:00\u{1f}helper\n";
        assert_eq!(
            latest_participant_commit(log, "helper"),
            Some(ts("2026-10-22T10:00:00Z"))
        );
    }

    #[test]
    fn activity_ignores_commits_of_another_login_and_empty_trailers() {
        let log = "2026-10-25T10:00:00+00:00\u{1f}someone-else\n\
                   2026-10-24T10:00:00+00:00\u{1f}\n\
                   2026-10-20T10:00:00+00:00\u{1f}helper\n";
        assert_eq!(
            latest_participant_commit(log, "helper"),
            Some(ts("2026-10-20T10:00:00Z"))
        );
        assert_eq!(latest_participant_commit(log, "nobody"), None);
    }

    #[test]
    fn activity_login_is_case_insensitive_and_handles_several_trailers() {
        let log = "2026-10-20T10:00:00+00:00\u{1f}Other,HeLPer\n";
        assert_eq!(
            latest_participant_commit(log, "helper"),
            Some(ts("2026-10-20T10:00:00Z"))
        );
    }

    #[test]
    fn activity_converts_offsets_before_comparing() {
        // 12:00+05:00 is 07:00Z, earlier than 10:00Z.
        let log = "2026-10-20T12:00:00+05:00\u{1f}helper\n\
                   2026-10-20T10:00:00+00:00\u{1f}helper\n";
        assert_eq!(
            latest_participant_commit(log, "helper"),
            Some(ts("2026-10-20T10:00:00Z"))
        );
    }

    #[test]
    fn activity_skips_malformed_lines_and_empty_output() {
        let log = "garbage without separator\n\
                   not-a-time\u{1f}helper\n\
                   \n\
                   2026-10-20T10:00:00+00:00\u{1f}helper\n";
        assert_eq!(
            latest_participant_commit(log, "helper"),
            Some(ts("2026-10-20T10:00:00Z"))
        );
        assert_eq!(latest_participant_commit("", "helper"), None);
    }

    #[test]
    fn activity_over_two_refs_is_the_maximum() {
        let own = "2026-10-20T10:00:00+00:00\u{1f}helper\n";
        let dev = "2026-10-23T10:00:00+00:00\u{1f}helper\n";
        let last = latest_participant_commit(own, "helper")
            .into_iter()
            .chain(latest_participant_commit(dev, "helper"))
            .max();
        assert_eq!(last, Some(ts("2026-10-23T10:00:00Z")));
    }

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
    fn status_note_ui_detail_is_coded_where_the_ui_needs_a_code() {
        let note = |detail: &str, ui: Option<&str>| StatusNote {
            status: SharedStatus::Untrusted,
            detail: detail.to_string(),
            diverged: None,
            rejected_branch: None,
            ui: ui.map(String::from),
        };
        // A changed dev branch: the UI gets the code, the issue keeps the text.
        let coded = ss::dev_branch_changed_reason("dev", "next");
        let n = note(&ss::humanize_untrusted_reason(&coded), Some(&coded));
        assert_eq!(n.ui_detail().as_deref(), Some(coded.as_str()));
        assert_eq!(n.detail, "dev branch changed from dev to next");
        // Any other reason is shown as text.
        assert_eq!(
            note("main is not the repo owner", None)
                .ui_detail()
                .as_deref(),
            Some("main is not the repo owner")
        );
        assert_eq!(note("  ", None).ui_detail(), None);
    }

    #[test]
    fn shared_state_decides_sharedness() {
        let st = |mode, role: Option<&str>| RepoSharedState {
            repo_id: 1,
            shared_mode: mode,
            shared_role: role.map(String::from),
            shared_status: None,
            shared_status_detail: None,
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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
        assert!(list_join_requests(&fx.main_db, Some(&gb()), fx.main_repo)
            .unwrap()
            .is_empty());
        // The helper's leftover request file is cleaned up and its status is clear.
        assert!(!fx.helper_dir.join(".sdh/requests/helper.json").exists());
        assert_eq!(fx.helper_status(), None);
    }

    // ----- statuses ---------------------------------------------------------------

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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
        // The UI names the branch the push was refused on (T-000223).
        let detail: serde_json::Value =
            serde_json::from_str(&fx.helper_status_detail().unwrap()).unwrap();
        assert_eq!(detail["branch"], "dev");
        assert!(detail["message"].as_str().unwrap().contains("GH006"));
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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
        let detail = out
            .issues
            .iter()
            .find(|i| i.code == Code::SharedNeedsManualMerge)
            .and_then(|i| i.detail.clone())
            .unwrap();
        assert!(
            detail.contains("dev") && detail.contains("Merge button"),
            "{}",
            detail
        );
        // The UI gets the branch and whether it is checked out (T-000223).
        assert_eq!(
            fx.main_status_detail().as_deref(),
            Some(r#"{"branch":"dev","current":true}"#)
        );
        assert!(!fx.main_dir.join(".sdh/requests/helper.json").exists());
        // Nothing was forced.
        assert!(fx.remote_file("dev", "src.rs").is_none());

        let res = merge_shared_upstream(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(res.outcome, "merged");
        assert!(fx.main_dir.join(".sdh/requests/helper.json").exists());
        assert_eq!(fx.main_status(), None);
        // The detail goes with the code.
        assert_eq!(fx.main_status_detail(), None);
        // The next sync pushes the merge.
        fx.sync_main();
        assert!(fx.remote_file("dev", "src.rs").is_some());
        assert!(fx.remote_file("dev", ".sdh/requests/helper.json").is_some());
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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
        // The reason is kept for the UI line.
        assert!(fx
            .helper_status_detail()
            .is_some_and(|d| d.contains("main is not the repo owner")));
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn phase_a_leaves_ordinary_repos_alone() {
        let fx = two_installs();
        let sync = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        assert!(sync.handled.is_empty());
        let out = sync.finish(&fx.main_db, Some(&gb()));
        assert!(out.issues.is_empty());
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);
    }

    // ----- switching on and off ----------------------------------------------------

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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
        assert!(run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id))
            .handled
            .is_empty());
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn accept_new_main_branch_clears_the_pin_and_status() {
        let fx = two_installs();
        fx.make_participant();
        let v = ss::read_shared_import(&fx.helper_db, fx.helper_repo);
        assert_eq!(v.pinned_dev_branch.as_deref(), Some("dev"));
        // Refused while the status is not a changed dev branch.
        assert!(accept_new_main_branch(&fx.helper_db, fx.helper_repo).is_err());
        fx.helper_db
            .set_shared_status_with_detail(
                fx.helper_repo,
                Some("shared_untrusted"),
                Some("main is not the repo owner"),
            )
            .unwrap();
        assert!(accept_new_main_branch(&fx.helper_db, fx.helper_repo).is_err());
        // Refused on the owner's own install.
        fx.main_db
            .set_shared_status_with_detail(
                fx.main_repo,
                Some("shared_untrusted"),
                Some("dev_branch_changed:dev\u{2192}next"),
            )
            .unwrap();
        assert!(accept_new_main_branch(&fx.main_db, fx.main_repo).is_err());
        fx.helper_db
            .set_shared_status_with_detail(
                fx.helper_repo,
                Some("shared_untrusted"),
                Some("dev_branch_changed:dev\u{2192}next"),
            )
            .unwrap();
        accept_new_main_branch(&fx.helper_db, fx.helper_repo).unwrap();
        assert_eq!(
            ss::read_shared_import(&fx.helper_db, fx.helper_repo).pinned_dev_branch,
            None
        );
        assert_eq!(fx.helper_status(), None);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn join_requests_are_listed_for_the_owner() {
        let fx = two_installs();
        fx.enable_main();
        fx.sync_helper();
        fx.sync_main();
        let reqs = list_join_requests(&fx.main_db, Some(&gb()), fx.main_repo).unwrap();
        assert_eq!(
            reqs,
            vec![JoinRequest {
                login: "helper".into(),
                branch: "helper-dev".into()
            }]
        );
        // Only the owner can approve.
        assert!(
            approve_join_request(&fx.helper_db, Some(&gb()), fx.helper_repo, "helper").is_err()
        );
        // A pending helper sees the owner's structure from the remote refs.
        let s = get_repo_structure(&fx.helper_db, Some(&gb()), fx.helper_repo)
            .unwrap()
            .unwrap();
        assert_eq!(s.main.login, "Owner");
    }

    // ----- plumbing rules --------------------------------------------------------------

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
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

    // ----- review round: guards, classifier, throttles ------------------------------

    fn block_pushes_to(fx: &TwoInstalls, refname: &str) -> PathBuf {
        let hook = fx.origin.join("hooks/update");
        fs::write(
            &hook,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"{}\" ]; then\n  echo \"error: GH006: Protected branch update failed for $1\" >&2\n  exit 1\nfi\nexit 0\n",
                refname
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        hook
    }

    #[test]
    fn net_slot_throttles_each_kind_and_repo_separately() {
        let dir = tempfile::TempDir::new().unwrap();
        let long = Duration::from_secs(3600);
        assert!(net_slot(NetKind::Fetch, 1, dir.path(), long));
        assert!(!net_slot(NetKind::Fetch, 1, dir.path(), long));
        // another kind, another repo, and a zero interval are independent
        assert!(net_slot(NetKind::Push, 1, dir.path(), long));
        assert!(net_slot(NetKind::Fetch, 2, dir.path(), long));
        assert!(net_slot(NetKind::Fetch, 1, dir.path(), Duration::ZERO));
    }

    #[test]
    fn a_recurring_issue_is_reported_once_until_it_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let issue = |d: &str| SyncIssue::new(Code::AutocommitFailed, "api").detail(d);
        assert_eq!(fresh_issues(7, dir.path(), vec![issue("boom")]).len(), 1);
        assert!(fresh_issues(7, dir.path(), vec![issue("boom")]).is_empty());
        // a different detail is a new problem
        assert_eq!(fresh_issues(7, dir.path(), vec![issue("other")]).len(), 1);
        // it went away and came back: reported again
        assert!(fresh_issues(7, dir.path(), vec![]).is_empty());
        assert_eq!(fresh_issues(7, dir.path(), vec![issue("other")]).len(), 1);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn head_history_lookups_for_plumbing() {
        let dir = tempfile::TempDir::new().unwrap();
        let d = dir.path();
        g(d, &["init", "-q"]);
        g(d, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        configure(d);
        write(d, "f.md", "v1\n");
        g(d, &["add", "."]);
        g(d, &["commit", "-q", "-m", "one"]);
        let v1 = g(d, &["rev-parse", "HEAD:f.md"]);
        write(d, "f.md", "v2\n");
        g(d, &["commit", "-q", "-am", "two"]);
        let v2 = g(d, &["rev-parse", "HEAD:f.md"]);
        let git = gb();
        // present: HEAD's own copy
        assert_eq!(last_version_sha(&git, d, "f.md"), Some(v2.clone()));
        assert!(blobs_in_history(&git, d, "HEAD", "f.md").contains(&v1));
        assert!(blobs_in_history(&git, d, "HEAD", "f.md").contains(&v2));
        // deleted: the version before the deleting commit
        g(d, &["rm", "-q", "f.md"]);
        g(d, &["commit", "-q", "-m", "gone"]);
        assert_eq!(last_version_sha(&git, d, "f.md"), Some(v2));
        assert!(blobs_in_history(&git, d, "HEAD", "f.md").contains(&v1));
        // a blob this history never held, and a file it never had
        let foreign = "1".repeat(40);
        assert!(!blobs_in_history(&git, d, "HEAD", "f.md").contains(&foreign));
        assert_eq!(last_version_sha(&git, d, "never.md"), None);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_repo_in_the_middle_of_a_merge_or_detached_is_left_alone() {
        let fx = two_installs();
        fx.make_participant();
        let dev_before = fx.remote_rev("dev").unwrap();
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        let head = g(&fx.helper_dir, &["rev-parse", "HEAD"]);
        let merge_head = fx.helper_dir.join(".git/MERGE_HEAD");
        fs::write(&merge_head, format!("{}\n", head)).unwrap();
        let out = fx.sync_helper();
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert!(out.committed.is_empty());
        assert_eq!(fx.remote_rev("dev").unwrap(), dev_before);
        assert_eq!(g(&fx.helper_dir, &["rev-parse", "HEAD"]), head);
        assert_eq!(fx.helper_status(), None);

        fs::remove_file(&merge_head).unwrap();
        g(&fx.helper_dir, &["checkout", "-q", "--detach"]);
        let out = fx.sync_helper();
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert_eq!(fx.remote_rev("dev").unwrap(), dev_before);

        g(&fx.helper_dir, &["checkout", "-q", "helper-dev"]);
        assert_eq!(fx.sync_helper().committed, vec!["api".to_string()]);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn the_old_autocommit_never_touches_a_repo_the_shared_pass_took_over() {
        let fx = two_installs();
        fx.enable_main();
        // In the middle of a merge the shared pass leaves the repo alone; the
        // ordinary auto-commit would try a partial commit there and fail.
        let head = g(&fx.main_dir, &["rev-parse", "HEAD"]);
        fs::write(fx.main_dir.join(".git/MERGE_HEAD"), format!("{}\n", head)).unwrap();
        write(&fx.main_dir, "docs/server-api/api.md", "# API\n");
        let home = tempfile::TempDir::new().unwrap();
        let res = crate::sync::run_project_sync_in(
            &fx.main_db,
            fx.project_id,
            Some(home.path().to_path_buf()),
        )
        .unwrap();
        assert!(
            !res.errors.iter().any(|i| i.code == Code::AutocommitFailed),
            "{:?}",
            res.errors
        );
        assert!(res.committed.is_empty());
        assert_eq!(g(&fx.main_dir, &["rev-parse", "HEAD"]), head);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn non_sync_commits_on_a_non_current_branch_block_plumbing_and_push() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.sync_helper(); // creates the local dev with our commit and pushes it
                          // The user commits code on the local `dev`, then goes back to its own branch.
        g(&fx.helper_dir, &["checkout", "-q", "dev"]);
        write(&fx.helper_dir, "src.rs", "fn main() {}\n");
        g(&fx.helper_dir, &["add", "src.rs"]);
        g(&fx.helper_dir, &["commit", "-q", "-m", "my code"]);
        g(&fx.helper_dir, &["checkout", "-q", "helper-dev"]);
        let local_dev = g(&fx.helper_dir, &["rev-parse", "dev"]);
        let remote_dev = fx.remote_rev("dev").unwrap();

        write(
            &fx.helper_dir,
            "docs/todo.md",
            &todo(&["T-500000", "T-500001"]),
        );
        let out = fx.sync_helper();
        assert_eq!(fx.remote_rev("dev").unwrap(), remote_dev, "never pushed");
        assert_eq!(
            g(&fx.helper_dir, &["rev-parse", "dev"]),
            local_dev,
            "never plumbed"
        );
        assert_eq!(fx.remote_file("dev", "src.rs"), None);
        assert_eq!(fx.helper_status().as_deref(), Some("needs_manual_merge"));
        let issue = out
            .issues
            .iter()
            .find(|i| i.code == Code::SharedNeedsManualMerge)
            .unwrap();
        let detail = issue.detail.as_deref().unwrap();
        assert!(
            detail.contains("dev") && detail.contains("git fetch"),
            "{}",
            detail
        );
        // The own branch still went through.
        assert!(fx
            .remote_file("helper-dev", "docs/todo.md")
            .unwrap()
            .contains("T-500001"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_diverged_branch_holding_only_our_commits_is_reset_with_a_backup() {
        let fx = two_installs();
        fx.make_participant();
        let hook = block_pushes_to(&fx, "refs/heads/dev");
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.sync_helper(); // local dev gets our commit; the push of dev is refused
        assert_eq!(fx.helper_status().as_deref(), Some("push_rejected"));
        fs::remove_file(&hook).unwrap();
        // Meanwhile the owner moves origin/dev.
        write(&fx.main_dir, "docs/server-api/api.md", "# API\n");
        fx.sync_main();
        assert!(fx.remote_file("dev", "docs/server-api/api.md").is_some());
        let old_local = g(&fx.helper_dir, &["rev-parse", "dev"]);

        let out = fx.sync_helper();
        assert_eq!(
            g(&fx.helper_dir, &["rev-parse", "refs/sdh-backup/dev"]),
            old_local
        );
        // Re-plumbed on top of origin's dev and pushed.
        assert!(fx
            .remote_file("dev", "docs/todo.md")
            .unwrap()
            .contains("T-500000"));
        assert!(fx.remote_file("dev", "docs/server-api/api.md").is_some());
        assert_eq!(fx.helper_status(), None, "{:?}", out.issues);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_newer_foreign_version_is_not_clobbered_by_plumbing() {
        let fx = two_installs();
        fx.make_participant();
        let req = "docs/server-requirements/x/REQ-001.md";
        write(&fx.helper_dir, req, "theirs\n");
        fx.sync_helper(); // helper-dev and dev now hold the helper's version
        write(&fx.main_dir, req, "mine\n");
        fx.sync_main();
        assert_eq!(
            fx.remote_file("helper-dev", req).as_deref(),
            Some("theirs\n"),
            "the helper's branch keeps a version this checkout never had"
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn managed_root_files_are_committed_on_the_current_branch_only() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.sync_helper(); // helper-dev exists on origin now
        fx.sync_main(); // main creates its local copy of helper-dev
        write(&fx.main_dir, ".gitignore", "target\n");
        fx.sync_main();
        assert_eq!(
            fx.remote_file("dev", ".gitignore").as_deref(),
            Some("target\n")
        );
        assert_eq!(fx.remote_file("helper-dev", ".gitignore"), None);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn owner_off_the_dev_branch_still_sees_and_prunes_join_requests() {
        let fx = two_installs();
        fx.enable_main();
        g(&fx.main_dir, &["checkout", "-q", "-b", "feature"]);
        fx.sync_helper(); // pending: the request lands in origin/dev
        fx.sync_main(); // fetch; the owner is on `feature`
        assert!(!fx.main_dir.join(".sdh/requests/helper.json").exists());
        let reqs = list_join_requests(&fx.main_db, Some(&gb()), fx.main_repo).unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].login, "helper");

        let s = approve_join_request(&fx.main_db, Some(&gb()), fx.main_repo, "helper").unwrap();
        assert_eq!(s.participants[0].band_start, 500_000);
        fx.sync_main();
        assert!(fx
            .remote_file("dev", ".sdh/project.json")
            .unwrap()
            .contains("\"helper\""));
        assert_eq!(fx.remote_file("dev", ".sdh/requests/helper.json"), None);
        assert_eq!(g(&fx.main_dir, &["branch", "--show-current"]), "feature");
    }

    // ----- project scope and probes ---------------------------------------------------

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn phase_a_only_reaches_the_shared_repos_of_the_synced_project() {
        let fx = two_installs();
        fx.enable_main();
        let other = fx
            .main_db
            .create_project("Other", None, "standard")
            .unwrap();
        let none = run_phase_a(&fx.main_db, Some(&gb()), Some(other.id));
        assert!(none.handled.is_empty());
        let some = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        assert!(some.handled.contains(&fx.main_repo));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_role_less_repo_is_probed_at_most_every_ten_minutes_across_restarts() {
        let fx = two_installs();
        fx.enable_main();
        let mut imp = ss::read_shared_import(&fx.helper_db, fx.helper_repo);
        imp.last_probe_unix = Some(unix_now());
        ss::write_shared_import(&fx.helper_db, fx.helper_repo, &imp).unwrap();
        fx.sync_helper();
        assert_eq!(
            fx.helper_db
                .get_shared_state(fx.helper_repo)
                .unwrap()
                .shared_role,
            None,
            "probed a minute ago: not looked at again"
        );
        assert_eq!(fx.remote_file("dev", ".sdh/requests/helper.json"), None);

        imp.last_probe_unix = Some(unix_now() - 700);
        ss::write_shared_import(&fx.helper_db, fx.helper_repo, &imp).unwrap();
        fx.sync_helper();
        assert_eq!(
            fx.helper_db
                .get_shared_state(fx.helper_repo)
                .unwrap()
                .shared_role
                .as_deref(),
            Some("pending")
        );
        assert!(fx.remote_file("dev", ".sdh/requests/helper.json").is_some());
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_manual_probe_ignores_the_ten_minute_throttle() {
        let fx = two_installs();
        fx.enable_main();
        let mut imp = ss::read_shared_import(&fx.helper_db, fx.helper_repo);
        imp.last_probe_unix = Some(unix_now());
        ss::write_shared_import(&fx.helper_db, fx.helper_repo, &imp).unwrap();
        let sync = probe_role_less(&fx.helper_db, Some(&gb()));
        assert!(
            sync.handled.contains(&fx.helper_repo),
            "probed a minute ago, but the user pressed the button"
        );
        assert_eq!(
            fx.helper_db
                .get_shared_state(fx.helper_repo)
                .unwrap()
                .shared_role
                .as_deref(),
            Some("pending")
        );
    }

    // ----- switching off, review round -------------------------------------------------

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn disable_before_any_sync_just_removes_the_files() {
        let fx = two_installs();
        enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).unwrap();
        assert!(fx.main_dir.join(".sdh/project.json").exists());
        let head = g(&fx.main_dir, &["rev-parse", "HEAD"]);
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(!res.pushed);
        assert!(!fx.main_dir.join(".sdh").exists());
        assert_eq!(g(&fx.main_dir, &["rev-parse", "HEAD"]), head);
        let st = fx.main_db.get_shared_state(fx.main_repo).unwrap();
        assert!(!st.shared_mode && st.shared_role.is_none());
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn only_the_owner_can_disable_and_nothing_flips_on_a_refusal() {
        let fx = two_installs();
        fx.make_participant();
        assert!(disable_shared_mode(&fx.helper_db, &gb(), fx.helper_repo).is_err());
        let st = fx.helper_db.get_shared_state(fx.helper_repo).unwrap();
        assert_eq!(st.shared_role.as_deref(), Some("contributor"));
        // the owner without the switch on is refused as well
        fx.main_db.set_shared_mode(fx.main_repo, false).unwrap();
        assert!(disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).is_err());
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_refused_push_of_the_removal_is_retried_by_the_next_sync() {
        let fx = two_installs();
        fx.enable_main();
        let hook = block_pushes_to(&fx, "refs/heads/dev");
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(!res.pushed, "{:?}", res);
        assert!(
            !fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .shared_mode
        );
        assert_eq!(fx.main_status().as_deref(), Some("push_failed"));
        assert_eq!(
            ss::read_shared_import(&fx.main_db, fx.main_repo)
                .disabling
                .as_deref(),
            Some("dev")
        );
        assert!(fx.remote_file("dev", ".sdh/project.json").is_some());

        fs::remove_file(&hook).unwrap();
        fx.sync_main();
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);
        assert_eq!(fx.main_status(), None);
        assert_eq!(
            ss::read_shared_import(&fx.main_db, fx.main_repo).disabling,
            None
        );
    }

    // ----- BUG-3: switching sharing off, branch clean-up and its failure paths ----------

    /// The helper is a contributor whose own branch `helper-dev` is on the remote
    /// and tracks `.sdh/project.json` (the owner plumbed it there), checked out in
    /// the helper's tree.
    fn helper_branch_with_sdh(fx: &TwoInstalls) {
        fx.make_participant();
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.sync_helper(); // helper-dev and dev get the task
        fx.sync_main(); // the owner plumbs the structure into helper-dev
        fx.sync_helper(); // and the helper fast-forwards onto it
        assert!(fx.remote_file("helper-dev", ".sdh/project.json").is_some());
        assert!(fx.helper_dir.join(".sdh/project.json").exists());
    }

    /// The owner's switch-off as the remote sees it: `.sdh/` gone from dev only.
    fn owner_unshares_dev_only(fx: &TwoInstalls) {
        g(&fx.main_dir, &["rm", "-q", "-r", ".sdh"]);
        g(&fx.main_dir, &["commit", "-q", "-m", "unshare"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
    }

    fn tracks(dir: &Path, rev: &str) -> bool {
        !g(dir, &["ls-tree", "-r", "--name-only", rev, "--", ".sdh"]).is_empty()
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_failed_cleanup_push_is_retried_until_the_branch_on_origin_is_clean() {
        let fx = two_installs();
        helper_branch_with_sdh(&fx);
        owner_unshares_dev_only(&fx);
        let hook = block_pushes_to(&fx, "refs/heads/helper-dev");

        fx.sync_helper();
        // The role went at once; the clean-up is committed but did not reach origin.
        assert_eq!(shared_role_of(&fx), None);
        assert!(!tracks(&fx.helper_dir, "helper-dev"));
        assert!(!fx.helper_dir.join(".sdh/project.json").exists());
        assert!(fx.remote_file("helper-dev", ".sdh/project.json").is_some());
        assert!(
            ss::read_shared_import(&fx.helper_db, fx.helper_repo)
                .pinned_dev_branch
                .is_some(),
            "the pin stays until the clean-up has reached origin"
        );
        // Still pending on the next sync while the push keeps failing.
        fx.sync_helper();
        assert!(fx.remote_file("helper-dev", ".sdh/project.json").is_some());

        fs::remove_file(&hook).unwrap();
        fx.sync_helper();
        assert_eq!(fx.remote_file("helper-dev", ".sdh/project.json"), None);
        assert_eq!(shared_role_of(&fx), None);
        assert_eq!(
            ss::read_shared_import(&fx.helper_db, fx.helper_repo).pinned_dev_branch,
            None,
            "done: the pin is dropped"
        );
        // And it stays done.
        fx.sync_helper();
        assert_eq!(shared_role_of(&fx), None);
        assert!(!tracks(&fx.helper_dir, "helper-dev"));
    }

    fn shared_role_of(fx: &TwoInstalls) -> Option<String> {
        fx.helper_db
            .get_shared_state(fx.helper_repo)
            .unwrap()
            .shared_role
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn without_the_pinned_dev_ref_nothing_is_deleted() {
        let fx = two_installs();
        helper_branch_with_sdh(&fx);
        owner_unshares_dev_only(&fx);
        // The helper cannot reach the remote and has no copy of origin/dev: the
        // switch-off is not established, so nothing is taken for stale.
        g(
            &fx.helper_dir,
            &["update-ref", "-d", "refs/remotes/origin/dev"],
        );
        g(
            &fx.helper_dir,
            &["remote", "set-url", "origin", "/nonexistent/sdh-remote.git"],
        );
        fx.sync_helper();
        assert_eq!(shared_role_of(&fx).as_deref(), Some("contributor"));
        assert!(tracks(&fx.helper_dir, "helper-dev"));
        assert!(fx.helper_dir.join(".sdh/project.json").exists());
        assert!(ss::read_shared_import(&fx.helper_db, fx.helper_repo)
            .pinned_dev_branch
            .is_some());
    }

    /// A commit on top of `parent` with the same tree, made without touching any
    /// checkout.
    fn foreign_commit_on(dir: &Path, branch_ref: &str, parent: &str) {
        let tree = g(dir, &["rev-parse", &format!("{}^{{tree}}", parent)]);
        let c = g(dir, &["commit-tree", &tree, "-p", parent, "-m", "foreign"]);
        g(dir, &["update-ref", branch_ref, &c]);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_diverged_participant_branch_is_skipped_with_a_note() {
        let fx = two_installs();
        helper_branch_with_sdh(&fx);
        // The owner's local copy of helper-dev holds a commit origin does not have,
        // while the helper pushes another one after the owner's last fetch.
        foreign_commit_on(&fx.main_dir, "refs/heads/helper-dev", "origin/helper-dev");
        write(
            &fx.helper_dir,
            "docs/todo.md",
            &todo(&["T-500000", "T-500001"]),
        );
        fx.sync_helper();
        fx.sync_main(); // dev is up to date; helper-dev is diverged and left alone

        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(res.pushed, "dev itself went through: {:?}", res);
        let note = res.note.unwrap_or_default();
        assert!(
            note.contains("helper-dev") && note.contains("diverged"),
            "{}",
            note
        );
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);
        assert!(
            fx.remote_file("helper-dev", ".sdh/project.json").is_some(),
            "the diverged branch is left alone"
        );
        // The foreign commit was not pushed.
        assert!(!g(&fx.origin, &["log", "--format=%s", "helper-dev"])
            .lines()
            .any(|s| s == "foreign"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_retried_removal_is_made_again_on_top_of_a_branch_origin_moved_past() {
        let fx = two_installs();
        helper_branch_with_sdh(&fx);
        let hook = block_pushes_to(&fx, "refs/heads/helper-dev");
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(!res.pushed, "{:?}", res);
        assert_eq!(fx.main_status().as_deref(), Some("push_failed"));
        let imp = ss::read_shared_import(&fx.main_db, fx.main_repo);
        assert_eq!(imp.disabling_branches, vec!["helper-dev".to_string()]);
        // dev already went through; helper-dev still tracks .sdh on origin.
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);
        assert!(fx.remote_file("helper-dev", ".sdh/project.json").is_some());

        // The contributor pushes first: origin's helper-dev moves past the
        // owner's local copy (which holds the removal commit).
        fs::remove_file(&hook).unwrap();
        write(&fx.helper_dir, "extra.txt", "x\n");
        g(&fx.helper_dir, &["add", "extra.txt"]);
        g(&fx.helper_dir, &["commit", "-q", "-m", "extra"]);
        g(&fx.helper_dir, &["push", "-q", "origin", "helper-dev"]);

        fx.sync_main();
        assert_eq!(fx.remote_file("helper-dev", ".sdh/project.json"), None);
        assert_eq!(
            fx.remote_file("helper-dev", "extra.txt").as_deref(),
            Some("x\n"),
            "the contributor's commit is kept"
        );
        assert_eq!(fx.main_status(), None);
        let imp = ss::read_shared_import(&fx.main_db, fx.main_repo);
        assert_eq!(imp.disabling, None);
        assert!(imp.disabling_branches.is_empty());
    }

    // ----- T-000221: import of what arrived by commit ------------------------------

    use crate::models::{ActivityEvent, FileBugNote, TimelineFilter};

    fn note(id: &str, desc: &str, status: &str, attempts: i32) -> FileBugNote {
        FileBugNote {
            id: id.to_string(),
            date: "2026-10-01".to_string(),
            description: desc.to_string(),
            severity: "major".to_string(),
            category: "logic".to_string(),
            status: status.to_string(),
            fix_attempts: attempts,
            comment: None,
        }
    }

    fn bugs_md(notes: &[FileBugNote]) -> String {
        crate::export::generate_bug_reports(notes)
    }

    fn events(db: &AppDb, author: Option<&str>) -> Vec<ActivityEvent> {
        db.read_timeline_filtered(
            &TimelineFilter {
                start_date: "1970-01-01".into(),
                end_date: "2100-01-01".into(),
                event_kinds: None,
                project_ids: None,
                repo_ids: None,
                search: None,
                author: author.filter(|a| *a != "me").map(String::from),
                author_is_me: author == Some("me"),
            },
            0,
            1000,
        )
        .unwrap()
    }

    fn bug_events_of(db: &AppDb, display_id: &str) -> Vec<ActivityEvent> {
        events(db, None)
            .into_iter()
            .filter(|e| e.kind == "bug_event" && e.bug_display_id.as_deref() == Some(display_id))
            .collect()
    }

    fn task_events_of(db: &AppDb, task_id: &str) -> Vec<ActivityEvent> {
        events(db, None)
            .into_iter()
            .filter(|e| e.kind == "task_event" && e.task_display_id.as_deref() == Some(task_id))
            .collect()
    }

    /// An owner DB bug, with its `created` event, and the MD regenerated.
    fn owner_bug(fx: &TwoInstalls, nid: i64, desc: &str, status: &str, attempts: i32) {
        let bug = fx
            .main_db
            .insert_bug(
                fx.main_repo,
                nid,
                "2026-10-01T00:00:00Z",
                desc,
                "major",
                "logic",
                status,
                attempts,
                None,
                (status == "confirmed").then_some("2026-10-02T00:00:00Z"),
            )
            .unwrap();
        fx.main_db
            .insert_bug_event(bug.id, "created", None, Some("created"), &bug.created_at)
            .unwrap();
        for _ in 0..attempts {
            fx.main_db
                .insert_bug_event(
                    bug.id,
                    "entered_testing",
                    Some("in-progress"),
                    Some("testing"),
                    "2026-10-01T10:00:00Z",
                )
                .unwrap();
        }
    }

    fn entered_testing(db: &AppDb, display_id: &str) -> usize {
        bug_events_of(db, display_id)
            .iter()
            .filter(|e| e.event_type == "entered_testing")
            .count()
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_bug_the_helper_committed_arrives_with_its_author_and_events() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        let mut b = note("B-500000", "found by helper", "testing", 2);
        b.comment = Some("see logs".into());
        write(&fx.helper_dir, "docs/bug-reports.md", &bugs_md(&[b]));
        fx.sync_helper();

        fx.sync_main();
        let got = fx
            .main_db
            .get_bug_by_display_id(fx.main_repo, "B-500000")
            .unwrap()
            .expect("the helper's bug is in the owner's database");
        assert_eq!(got.numeric_id, 500000);
        assert_eq!(got.description, "found by helper");
        assert_eq!(got.status, "testing");
        assert_eq!(got.fix_attempts, 2);
        assert_eq!(got.comment.as_deref(), Some("see logs"));
        assert!(got.created_at.starts_with("2026-10-01"));

        let evs = bug_events_of(&fx.main_db, "B-500000");
        assert!(evs.iter().any(|e| e.event_type == "created"));
        assert_eq!(
            entered_testing(&fx.main_db, "B-500000"),
            got.fix_attempts as usize,
            "COUNT(entered_testing) == fix_attempts"
        );
        // No trailer range on the first import: the band owner is the author.
        assert!(evs.iter().all(|e| e.author.as_deref() == Some("helper")));
        // The work tree still holds the row (the reconcile did not drop it).
        assert!(fs::read_to_string(fx.main_dir.join("docs/bug-reports.md"))
            .unwrap()
            .contains("B-500000"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_committed_protected_change_is_accepted_and_attributed_by_trailer() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        owner_bug(&fx, 1, "original text", "created", 0);
        crate::sync::bugs::regenerate_bugs_md(&fx.main_db, fx.main_repo).unwrap();
        fx.sync_main(); // committed + pushed: its own content, nothing to import
        let imp = ss::read_shared_import(&fx.main_db, fx.main_repo);
        assert!(
            imp.blobs.contains_key("docs/bug-reports.md"),
            "the own commit is recorded: {:?}",
            imp
        );
        fx.sync_helper(); // the helper receives the file

        let text = fs::read_to_string(fx.helper_dir.join("docs/bug-reports.md")).unwrap();
        assert!(text.contains("original text"));
        // The helper's UI changes a protected field and the status; the sync
        // commits the rendering.
        let hb = fx
            .helper_db
            .get_bug_by_display_id(fx.helper_repo, "B-000001")
            .unwrap()
            .expect("migrated from the file that arrived");
        fx.helper_db
            .update_bug_fields(hb.id, Some("helper rewrote it"), None, None, None)
            .unwrap();
        fx.helper_db
            .update_bug_status(hb.id, "in-progress", None, None)
            .unwrap();
        crate::sync::bugs::regenerate_bugs_md(&fx.helper_db, fx.helper_repo).unwrap();
        fx.sync_helper();

        fx.sync_main();
        let got = fx
            .main_db
            .get_bug_by_display_id(fx.main_repo, "B-000001")
            .unwrap()
            .unwrap();
        assert_eq!(got.description, "helper rewrote it");
        assert_eq!(got.status, "in-progress");
        let evs = bug_events_of(&fx.main_db, "B-000001");
        let taken = evs.iter().find(|e| e.event_type == "taken").unwrap();
        // B-000001 lies in main's band, so only the trailer can name the helper.
        assert_eq!(taken.author.as_deref(), Some("helper"));
        let created = evs.iter().find(|e| e.event_type == "created").unwrap();
        assert_eq!(created.author, None);
        // The rendering sticks.
        assert!(fs::read_to_string(fx.main_dir.join("docs/bug-reports.md"))
            .unwrap()
            .contains("helper rewrote it"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn an_uncommitted_local_edit_of_a_protected_field_is_restored() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        owner_bug(&fx, 1, "original text", "created", 0);
        crate::sync::bugs::regenerate_bugs_md(&fx.main_db, fx.main_repo).unwrap();
        fx.sync_main();

        let path = fx.main_dir.join("docs/bug-reports.md");
        let edited = fs::read_to_string(&path)
            .unwrap()
            .replace("original text", "local AI changed it");
        fs::write(&path, edited).unwrap();
        fx.sync_main();

        let got = fx
            .main_db
            .get_bug_by_display_id(fx.main_repo, "B-000001")
            .unwrap()
            .unwrap();
        assert_eq!(got.description, "original text");
        assert!(fs::read_to_string(&path).unwrap().contains("original text"));
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("local AI changed it"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_row_only_the_work_tree_has_is_not_a_bug() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        owner_bug(&fx, 1, "real one", "created", 0);
        crate::sync::bugs::regenerate_bugs_md(&fx.main_db, fx.main_repo).unwrap();
        fx.sync_main();
        let path = fx.main_dir.join("docs/bug-reports.md");
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("- B-000009 | 2026-10-05 | invented by AI | major | logic | created | 0 |\n");
        fs::write(&path, text).unwrap();
        fx.sync_main();
        assert!(fx
            .main_db
            .get_bug_by_display_id(fx.main_repo, "B-000009")
            .unwrap()
            .is_none());
        assert!(!fs::read_to_string(&path).unwrap().contains("B-000009"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_confirmed_bug_removed_by_commit_is_archived_a_live_one_is_kept() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        // helper-dev must exist on origin so that the owner plumbs into it.
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.sync_helper();
        fx.sync_main();
        owner_bug(&fx, 1, "verdict given", "confirmed", 1);
        owner_bug(&fx, 2, "still open", "testing", 1);
        crate::sync::bugs::regenerate_bugs_md(&fx.main_db, fx.main_repo).unwrap();
        fx.sync_main(); // committed to dev and plumbed into helper-dev
        fx.sync_helper(); // the helper receives the file

        // The helper's local AI deletes both rows. The live one is restored, the
        // confirmed one is acknowledged; the sync commits and plumbs that into
        // origin's dev, which still holds both rows from the owner.
        write(&fx.helper_dir, "docs/bug-reports.md", &bugs_md(&[]));
        fx.sync_helper();
        assert!(!fx
            .remote_file("dev", "docs/bug-reports.md")
            .unwrap()
            .contains("B-000001"));

        fx.sync_main();
        let confirmed = fx
            .main_db
            .get_bug_by_display_id(fx.main_repo, "B-000001")
            .unwrap()
            .unwrap();
        assert!(confirmed.archived_from_md_at.is_some(), "acknowledged");
        let live = fx
            .main_db
            .get_bug_by_display_id(fx.main_repo, "B-000002")
            .unwrap()
            .unwrap();
        assert_eq!(live.status, "testing");
        assert_eq!(live.archived_from_md_at, None);
        let rendered = fs::read_to_string(fx.main_dir.join("docs/bug-reports.md")).unwrap();
        assert!(rendered.contains("B-000002"), "re-rendered:\n{}", rendered);
        assert!(!rendered.contains("B-000001"));
        // The deletion stays deleted on both branches after the next rounds.
        fx.sync_main();
        fx.sync_helper();
        for branch in ["dev", "helper-dev"] {
            let t = fx.remote_file(branch, "docs/bug-reports.md").unwrap();
            assert!(!t.contains("B-000001"), "{} resurrected it:\n{}", branch, t);
            assert!(
                t.contains("B-000002"),
                "{} lost the live bug:\n{}",
                branch,
                t
            );
        }
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn task_events_carry_the_author_of_the_commit_and_local_edits_carry_none() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.main_dir, "docs/todo.md", &todo(&["T-000001"]));
        crate::sync::tasks::sync_tasks_for_repo(&fx.main_db, fx.main_repo).unwrap();
        fx.sync_main(); // committed
        fx.sync_helper();

        let started = fs::read_to_string(fx.helper_dir.join("docs/todo.md"))
            .unwrap()
            .replace("| open |", "| in-progress |");
        write(&fx.helper_dir, "docs/todo.md", &started);
        fx.sync_helper();

        fx.sync_main(); // arrives
        let evs = task_events_of(&fx.main_db, "T-000001");
        let taken = evs.iter().find(|e| e.event_type == "taken").unwrap();
        assert_eq!(taken.author.as_deref(), Some("helper"));
        let before = evs.len();

        // The ordinary disk run after the import sees nothing new.
        crate::sync::tasks::sync_tasks_for_repo(&fx.main_db, fx.main_repo).unwrap();
        assert_eq!(task_events_of(&fx.main_db, "T-000001").len(), before);

        // A local edit made afterwards is this install's own.
        let reviewed = fs::read_to_string(fx.main_dir.join("docs/todo.md"))
            .unwrap()
            .replace("| in-progress |", "| review |");
        write(&fx.main_dir, "docs/todo.md", &reviewed);
        crate::sync::tasks::sync_tasks_for_repo(&fx.main_db, fx.main_repo).unwrap();
        let evs = task_events_of(&fx.main_db, "T-000001");
        let review = evs.iter().find(|e| e.event_type == "review").unwrap();
        assert_eq!(review.author, None);
        // And the author filter finds exactly the helper's events.
        let mine = events(&fx.main_db, Some("helper"));
        assert!(!mine.is_empty());
        assert!(mine.iter().all(|e| e.author.as_deref() == Some("helper")));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn the_first_sync_is_a_baseline_and_applies_nothing_spurious() {
        let fx = two_installs();
        fx.make_participant();
        // The owner commits a bug and a task.
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        owner_bug(&fx, 1, "shared bug", "testing", 1);
        crate::sync::bugs::regenerate_bugs_md(&fx.main_db, fx.main_repo).unwrap();
        write(&fx.main_dir, "docs/todo.md", &todo(&["T-000001"]));
        fx.sync_main();

        // The helper already has the same bug and the same (older) task in its
        // own database, and has never imported anything.
        let hb = fx
            .helper_db
            .insert_bug(
                fx.helper_repo,
                1,
                "2026-10-01T00:00:00Z",
                "shared bug",
                "major",
                "logic",
                "testing",
                1,
                None,
                None,
            )
            .unwrap();
        fx.helper_db
            .insert_bug_event(hb.id, "created", None, Some("created"), &hb.created_at)
            .unwrap();
        fx.helper_db
            .insert_bug_event(
                hb.id,
                "entered_testing",
                Some("in-progress"),
                Some("testing"),
                "2026-10-01T10:00:00Z",
            )
            .unwrap();
        fx.helper_db
            .set_bugs_migrated_at(fx.helper_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        fx.helper_db
            .insert_task(
                fx.helper_repo,
                "T-000001",
                "T",
                "task T-000001",
                Some(1.0),
                Some("low"),
                Some("in-progress"),
                Some("v1.0.0"),
                "todo",
                "2026-10-01",
            )
            .unwrap();
        fx.helper_db
            .mark_tasks_migrated(fx.helper_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        let bugs_before = bug_events_of(&fx.helper_db, "B-000001").len();
        let tasks_before = task_events_of(&fx.helper_db, "T-000001").len();

        fx.sync_helper();
        assert_eq!(bug_events_of(&fx.helper_db, "B-000001").len(), bugs_before);
        assert_eq!(
            fx.helper_db
                .get_bug_by_display_id(fx.helper_repo, "B-000001")
                .unwrap()
                .unwrap()
                .fix_attempts,
            1
        );
        // Tasks: the baseline leaves the mirror to the disk run.
        assert_eq!(
            task_events_of(&fx.helper_db, "T-000001").len(),
            tasks_before
        );
        let imp = ss::read_shared_import(&fx.helper_db, fx.helper_repo);
        assert!(imp.blobs.contains_key("docs/todo.md"), "{:?}", imp);
        assert!(imp.blobs["docs/bug-reports.md"].contains('@'));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_file_that_does_not_change_is_not_imported_twice() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        write(
            &fx.helper_dir,
            "docs/bug-reports.md",
            &bugs_md(&[note("B-500000", "once", "testing", 1)]),
        );
        fx.sync_helper();
        fx.sync_main();
        let n = bug_events_of(&fx.main_db, "B-500000").len();
        assert!(n >= 2);
        fx.sync_main();
        fx.sync_helper();
        fx.sync_main();
        assert_eq!(bug_events_of(&fx.main_db, "B-500000").len(), n);
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn authors_follow_the_trailer_or_the_band() {
        let s = ProjectStructure {
            format: 1,
            project: ss::StructProject {
                name: "Acme".into(),
                project_type: "standard".into(),
            },
            main: ss::StructMain {
                login: "Owner".into(),
                dev_branch: "dev".into(),
            },
            repos: vec![],
            microservices: vec![],
            participants: vec![
                ss::Participant {
                    login: "helper".into(),
                    branch: "h".into(),
                    band_start: 500000,
                },
                ss::Participant {
                    login: "other".into(),
                    branch: "o".into(),
                    band_start: 600000,
                },
            ],
        };
        let a = Authors {
            me: "Owner".into(),
            structure: Some(s),
        };
        // one foreign login
        let r = a.rule(Some(vec!["helper".into(), "Helper".into()]));
        assert_eq!(a.resolve(&r, Some(1)).as_deref(), Some("helper"));
        // this install's own login does not count towards "several"
        let r = a.rule(Some(vec!["Owner".into(), "helper".into(), "owner".into()]));
        assert_eq!(a.resolve(&r, Some(42)).as_deref(), Some("helper"));
        // only this install / nobody
        let r = a.rule(Some(vec!["owner".into()]));
        assert_eq!(a.resolve(&r, Some(500001)), None);
        let r = a.rule(Some(vec![]));
        assert_eq!(a.resolve(&r, Some(500001)), None);
        // several logins: per id band; main's own band is this install
        let r = a.rule(Some(vec!["helper".into(), "other".into()]));
        assert_eq!(a.resolve(&r, Some(500010)).as_deref(), Some("helper"));
        assert_eq!(a.resolve(&r, Some(600010)).as_deref(), Some("other"));
        assert_eq!(a.resolve(&r, Some(42)), None);
        // unusable range: same as several
        let r = a.rule(None);
        assert_eq!(a.resolve(&r, Some(600010)).as_deref(), Some("other"));
        assert_eq!(id_number("T-500001"), Some(500001));
        assert_eq!(id_number("B-000042"), Some(42));
        assert_eq!(id_number("nonsense"), None);
    }

    // ----- T-000220 review minors -----------------------------------------------

    fn commit_on(dir: &Path, branch: &str, msg: &str) -> String {
        let tree = g(dir, &["rev-parse", &format!("{}^{{tree}}", branch)]);
        let new = g(dir, &["commit-tree", &tree, "-p", branch, "-m", msg]);
        g(
            dir,
            &["update-ref", &format!("refs/heads/{}", branch), &new],
        );
        new
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn reset_to_origin_uses_the_judged_tip_and_refuses_a_checked_out_branch() {
        let fx = two_installs();
        let dir = &fx.main_dir;
        g(dir, &["branch", "side"]);
        g(dir, &["push", "-q", "origin", "side"]);
        let judged = g(dir, &["rev-parse", "side"]);
        let origin_side = judged.clone();
        // the branch moves after it was judged
        let moved = commit_on(dir, "side", "moved after the judgement");
        assert!(!reset_to_origin(&gb(), dir, "side", &judged).unwrap());
        assert_eq!(g(dir, &["rev-parse", "side"]), moved, "left alone");
        // the judged tip is the current one: reset, with a backup
        assert!(reset_to_origin(&gb(), dir, "side", &moved).unwrap());
        assert_eq!(g(dir, &["rev-parse", "side"]), origin_side);
        assert_eq!(g(dir, &["rev-parse", "refs/sdh-backup/side"]), moved);
        // a checked-out branch is refused outright
        let tip = g(dir, &["rev-parse", "dev"]);
        assert!(reset_to_origin(&gb(), dir, "dev", &tip).is_err());
        assert_eq!(g(dir, &["rev-parse", "dev"]), tip);
        // the judgement itself reports the sha it judged
        assert_eq!(
            local_tip_if_ours(&gb(), dir, "side").unwrap(),
            Some(origin_side)
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_pending_removal_is_fetched_before_it_is_pushed() {
        let fx = two_installs();
        fx.enable_main();
        let hook = block_pushes_to(&fx, "refs/heads/dev");
        disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(fx.main_status().as_deref(), Some("push_failed"));
        fs::remove_file(&hook).unwrap();
        // The remote is unreachable now: the retry fails at the fetch.
        g(
            &fx.main_dir,
            &["remote", "set-url", "origin", "/nonexistent/origin.git"],
        );
        fx.sync_main();
        assert_eq!(fx.main_status().as_deref(), Some("fetch_failed"));
        assert_eq!(
            ss::read_shared_import(&fx.main_db, fx.main_repo)
                .disabling
                .as_deref(),
            Some("dev"),
            "still pending"
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn enabling_sharing_again_drops_a_leftover_removal_marker() {
        let fx = two_installs();
        fx.enable_main();
        block_pushes_to(&fx, "refs/heads/dev");
        disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(
            ss::read_shared_import(&fx.main_db, fx.main_repo)
                .disabling
                .as_deref(),
            Some("dev")
        );
        enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).unwrap();
        assert_eq!(
            ss::read_shared_import(&fx.main_db, fx.main_repo).disabling,
            None
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn last_common_blob_finds_the_shared_version_of_unrelated_lineages() {
        let dir = tempfile::TempDir::new().unwrap();
        let d = dir.path();
        g(d, &["init", "-q"]);
        g(d, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        configure(d);
        write(d, "f.md", "A\n");
        g(d, &["add", "."]);
        g(d, &["commit", "-q", "-m", "a"]);
        let a = g(d, &["rev-parse", "HEAD:f.md"]);
        write(d, "f.md", "C\n");
        g(d, &["commit", "-q", "-am", "c"]);
        // A second root that never shares a commit with `dev`, only the blob A.
        g(d, &["checkout", "-q", "--orphan", "other"]);
        g(d, &["rm", "-rfq", "."]);
        write(d, "f.md", "X\n");
        g(d, &["add", "."]);
        g(d, &["commit", "-q", "-m", "x"]);
        write(d, "f.md", "A\n");
        g(d, &["commit", "-q", "-am", "a again"]);
        write(d, "f.md", "B\n");
        g(d, &["commit", "-q", "-am", "b"]);
        let git = gb();
        assert_eq!(git_sync::merge_base(&git, d, "dev", "other").unwrap(), None);
        assert_eq!(
            last_common_blob(&git, d, "dev", "other", "f.md"),
            Some(a.clone())
        );
        assert_eq!(last_common_blob(&git, d, "other", "dev", "f.md"), Some(a));
        // Content recurs: the other side's tip equals an old version, but the
        // last state both held is the one in between.
        g(d, &["checkout", "-q", "dev"]);
        write(
            d, "r.md", "A
",
        );
        g(d, &["add", "r.md"]);
        g(d, &["commit", "-q", "-m", "r1"]);
        write(
            d, "r.md", "X
",
        );
        g(d, &["commit", "-q", "-am", "r2"]);
        let x = g(d, &["rev-parse", "HEAD:r.md"]);
        write(
            d, "r.md", "Z
",
        );
        g(d, &["commit", "-q", "-am", "r3"]);
        g(d, &["checkout", "-q", "other"]);
        write(
            d, "r.md", "A
",
        );
        g(d, &["add", "r.md"]);
        g(d, &["commit", "-q", "-m", "o1"]);
        write(
            d, "r.md", "X
",
        );
        g(d, &["commit", "-q", "-am", "o2"]);
        write(
            d, "r.md", "A
",
        );
        g(d, &["commit", "-q", "-am", "o3"]);
        assert_eq!(last_common_blob(&git, d, "dev", "other", "r.md"), Some(x));
        // nothing in common, and a file neither has
        write(d, "g.md", "only other\n");
        g(d, &["add", "g.md"]);
        g(d, &["commit", "-q", "-m", "g"]);
        assert_eq!(last_common_blob(&git, d, "dev", "other", "g.md"), None);
        assert_eq!(last_common_blob(&git, d, "dev", "other", "nope.md"), None);
    }

    /// Commit each version of `docs/todo.md` in turn on `branch`; an `orphan`
    /// branch shares no commit with the others, only blobs.
    fn commit_lineage(d: &Path, branch: &str, orphan: bool, versions: &[&str]) {
        if orphan {
            g(d, &["checkout", "-q", "--orphan", branch]);
            g(d, &["rm", "-rfq", "--ignore-unmatch", "."]);
        } else {
            g(d, &["checkout", "-q", "-B", branch]);
        }
        for (i, v) in versions.iter().enumerate() {
            if *v == "<binary>" {
                // A version that is not text.
                fs::create_dir_all(d.join("docs")).unwrap();
                fs::write(d.join("docs/todo.md"), [0xffu8, 0xfe, 0x00, 0x80]).unwrap();
            } else {
                write(d, "docs/todo.md", v);
            }
            g(d, &["add", "docs/todo.md"]);
            g(d, &["commit", "-q", "-m", &format!("{} {}", branch, i)]);
        }
    }

    /// `ours` is HEAD's history (the work tree is its tip), `theirs` the
    /// plumbed branch's; returns the todo.md the branch holds afterwards.
    fn plumb_lineages(ours: &[&str], theirs: &[&str]) -> String {
        let dir = tempfile::TempDir::new().unwrap();
        let d = dir.path();
        g(d, &["init", "-q"]);
        g(d, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
        configure(d);
        commit_lineage(d, "other", true, theirs);
        commit_lineage(d, "dev", true, ours);
        let spec = vec!["docs/todo.md".to_string()];
        plumb_into_branch(&gb(), d, "other", &spec, "me", &[]).unwrap();
        g(d, &["show", "other:docs/todo.md"])
    }

    #[test]
    fn a_repeating_history_does_not_resurrect_a_row_we_deleted() {
        // S0 = both rows, K = T-000001 deleted. Our history ends one deletion
        // after the branch's tip: the branch is a prefix of ours, so it simply
        // missed the last deletion.
        let s0 = todo(&["T-000001", "T-000002"]);
        let k = todo(&["T-000002"]);
        let got = plumb_lineages(&[&s0, &k, &s0, &k], &[&s0, &k, &s0]);
        assert!(!got.contains("T-000001"), "resurrected:\n{}", got);
        assert!(got.contains("T-000002"));
    }

    #[test]
    fn one_unreadable_old_version_does_not_spoil_the_per_row_base() {
        let s0 = todo(&["T-000001", "T-000002"]);
        let k = todo(&["T-000002"]);
        let got = plumb_lineages(&["<binary>", &s0, &k, &s0, &k], &[&s0, &k, &s0]);
        assert!(!got.contains("T-000001"), "resurrected:\n{}", got);
        assert!(got.contains("T-000002"));
    }

    #[test]
    fn a_repeating_history_keeps_a_byte_identical_row_we_re_added() {
        // Our history is one re-add past the branch's tip (S0 again, same bytes
        // as the first time): the row is ours to add back, not a deletion.
        let s0 = todo(&["T-000001", "T-000002"]);
        let k = todo(&["T-000002"]);
        let got = plumb_lineages(&[&k, &s0, &k, &s0], &[&k, &s0, &k]);
        assert!(got.contains("T-000001"), "re-added row lost:\n{}", got);
        assert!(got.contains("T-000002"));
    }

    fn done_md(ids: &[&str]) -> String {
        let mut s = String::from("## 2026-10-20\n\n");
        for id in ids {
            s.push_str(&format!("- {} | task {} | v1.0.0\n", id, id));
        }
        s
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_task_moved_to_done_stays_moved_and_nothing_resurrects() {
        let fx = two_installs();
        fx.make_participant();
        // The helper's task reaches origin's dev and helper-dev.
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.sync_helper();
        fx.sync_main(); // dev fast-forwards; main gets its copy of helper-dev
                        // The owner adds a task; it is plumbed into helper-dev.
        write(
            &fx.main_dir,
            "docs/todo.md",
            &todo(&["T-000001", "T-500000"]),
        );
        fx.sync_main();
        assert!(fx
            .remote_file("helper-dev", "docs/todo.md")
            .unwrap()
            .contains("T-000001"));
        fx.sync_helper(); // fast-forwards onto it

        // The owner finishes T-000001: it leaves todo and enters done.
        write(&fx.main_dir, "docs/todo.md", &todo(&["T-500000"]));
        write(&fx.main_dir, "docs/done.md", &done_md(&["T-000001"]));
        fx.sync_main();
        for branch in ["dev", "helper-dev"] {
            let t = fx.remote_file(branch, "docs/todo.md").unwrap();
            assert!(
                !t.contains("T-000001"),
                "{} todo resurrected it:\n{}",
                branch,
                t
            );
            assert!(
                fx.remote_file(branch, "docs/done.md")
                    .unwrap()
                    .contains("T-000001"),
                "{} lacks it in done",
                branch
            );
        }

        // The helper still has the stale todo and adds a row of its own; its
        // sync must neither bring T-000001 back to dev nor lose the new row.
        let stale = fs::read_to_string(fx.helper_dir.join("docs/todo.md")).unwrap();
        assert!(stale.contains("T-000001"));
        write(
            &fx.helper_dir,
            "docs/todo.md",
            &format!("{}{}", stale, task("T-500001")),
        );
        fx.sync_helper();
        let dev_todo = fx.remote_file("dev", "docs/todo.md").unwrap();
        assert!(
            dev_todo.contains("T-500001"),
            "the new row arrives:\n{}",
            dev_todo
        );
        assert!(
            !dev_todo.contains("T-000001"),
            "no resurrection:\n{}",
            dev_todo
        );

        // And the owner's next sync does not bounce it back either.
        fx.sync_main();
        for branch in ["dev", "helper-dev"] {
            let t = fx.remote_file(branch, "docs/todo.md").unwrap();
            assert!(
                !t.contains("T-000001"),
                "{} after the owner's sync:\n{}",
                branch,
                t
            );
            assert!(
                t.contains("T-500001"),
                "{} lost the new row:\n{}",
                branch,
                t
            );
            assert!(fx
                .remote_file(branch, "docs/done.md")
                .unwrap()
                .contains("T-000001"));
        }
        let mine = fs::read_to_string(fx.main_dir.join("docs/todo.md")).unwrap();
        assert!(!mine.contains("T-000001") && mine.contains("T-500001"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_first_sync_with_a_dirty_bugs_file_keeps_the_local_work() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        owner_bug(&fx, 1, "original", "created", 0);
        crate::sync::bugs::regenerate_bugs_md(&fx.main_db, fx.main_repo).unwrap();
        fx.sync_main();
        // The helper gets HEAD by hand, so its first shared sync meets a
        // migrated repo whose HEAD has the file.
        g(&fx.helper_dir, &["fetch", "-q", "origin"]);
        g(
            &fx.helper_dir,
            &["merge", "-q", "--ff-only", "origin/helper-dev"],
        );
        fx.helper_db
            .set_bugs_migrated_at(fx.helper_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        // Uncommitted UI work: an edit of the owner's bug and a new bug.
        let hb = fx
            .helper_db
            .insert_bug(
                fx.helper_repo,
                1,
                "2026-10-01T00:00:00Z",
                "ui edited",
                "major",
                "logic",
                "created",
                0,
                None,
                None,
            )
            .unwrap();
        fx.helper_db
            .insert_bug_event(hb.id, "created", None, Some("created"), &hb.created_at)
            .unwrap();
        let new = fx
            .helper_db
            .insert_bug(
                fx.helper_repo,
                500000,
                "2026-10-02T00:00:00Z",
                "ui created",
                "minor",
                "other",
                "created",
                0,
                None,
                None,
            )
            .unwrap();
        fx.helper_db
            .insert_bug_event(new.id, "created", None, Some("created"), &new.created_at)
            .unwrap();
        crate::sync::bugs::regenerate_bugs_md(&fx.helper_db, fx.helper_repo).unwrap();

        fx.sync_helper();
        let got = |id: &str| {
            fx.helper_db
                .get_bug_by_display_id(fx.helper_repo, id)
                .unwrap()
                .unwrap()
        };
        assert_eq!(got("B-000001").description, "ui edited");
        assert_eq!(got("B-500000").description, "ui created");
        let imp = ss::read_shared_import(&fx.helper_db, fx.helper_repo);
        assert!(
            imp.blobs.contains_key("docs/bug-reports.md"),
            "HEAD is recorded as the baseline"
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn refused_rows_are_reported_and_hold_the_reconcile_back_for_one_pass() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        owner_bug(&fx, 1, "fine", "created", 0);
        crate::sync::bugs::regenerate_bugs_md(&fx.main_db, fx.main_repo).unwrap();
        fx.sync_main();

        g(&fx.helper_dir, &["fetch", "-q", "origin"]);
        let wt = fx._tmp.path().join("wt2");
        g(
            &fx.helper_dir,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "tmpdev2",
                wt.to_str().unwrap(),
                "origin/dev",
            ],
        );
        let text = fs::read_to_string(wt.join("docs/bug-reports.md")).unwrap()
            + "- B-500001 | 2026-10-03 | good | major | logic | created | 0 |\n"
            + "- B-500002 | 2026-10-03 | bad | huge | logic | created | 0 |\n";
        write(&wt, "docs/bug-reports.md", &text);
        g(&wt, &["add", "docs/bug-reports.md"]);
        g(
            &wt,
            &[
                "commit",
                "-q",
                "-m",
                "chore(sdh-sync): sync shared files\n\nSDH-Participant: helper",
            ],
        );
        g(&wt, &["push", "-q", "origin", "tmpdev2:dev"]);

        let out = fx.sync_main();
        assert!(
            codes(&out).contains(&Code::ManagedFileFailed),
            "reported: {:?}",
            out.issues
        );
        let detail = out
            .issues
            .iter()
            .find_map(|i| i.detail.clone())
            .unwrap_or_default();
        assert!(detail.contains("B-500002"), "{}", detail);
        let main = |id: &str| fx.main_db.get_bug_by_display_id(fx.main_repo, id).unwrap();
        assert!(main("B-500001").is_some(), "the good row is imported");
        assert!(main("B-500002").is_none());
        // The reconcile was held back: the incoming file is not rewritten yet.
        let file = fs::read_to_string(fx.main_dir.join("docs/bug-reports.md")).unwrap();
        assert!(file.contains("B-500002"));
        // The same problem is not reported again, and the next pass reconciles.
        let again = fx.sync_main();
        assert!(!codes(&again).contains(&Code::ManagedFileFailed));
        let file = fs::read_to_string(fx.main_dir.join("docs/bug-reports.md")).unwrap();
        assert!(!file.contains("B-500002"));
    }
}
