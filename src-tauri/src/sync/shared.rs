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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::db::AppDb;
use crate::git_ops::{
    commit_paths, commit_paths_index_only, current_branch, is_git_repo, list_markers_in_rev,
    spawn_cmd, CommitOutcome, GitBinary,
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

/// T-000243 review (sixth pass): per-repo shared-lifecycle GENERATION.
/// In-memory on purpose: a pass never outlives its sync call, so the
/// counter needs no persistence (and no schema change). Keyed by
/// (repo_id, root) like the net/probe slots — test fixtures share repo ids.
/// Every lifecycle writer bumps it while holding `specs_policy_guard`; a
/// pass seals the generation that was current when its phase-A decisions
/// STARTED (before the shared-state read at the top of the repo loop), and
/// `finish` treats a mismatch as stale. A generation — not a state
/// snapshot — because lifecycle round trips (disable→re-enable) restore
/// identical state, and because the pass's decisions span phase A while
/// writers do not wait for the seal: a disable that completes between the
/// pass's discovery and its seal still bumps the counter the pass sealed.
fn lifecycle_generations() -> &'static Mutex<HashMap<(i64, String), u64>> {
    static G: OnceLock<Mutex<HashMap<(i64, String), u64>>> = OnceLock::new();
    G.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lifecycle_generation(repo_id: i64, root: &Path) -> u64 {
    lifecycle_generations()
        .lock()
        .unwrap()
        .get(&(repo_id, root.to_string_lossy().into_owned()))
        .copied()
        .unwrap_or(0)
}

/// Bump the lifecycle generation of `repo_id` at `root`. Callers hold
/// `specs_policy_guard(repo_id)` — the bump must be atomic with `finish`'s
/// guarded freshness check, or a writer could land between the check and
/// the settlement.
fn bump_lifecycle_generation(repo_id: i64, root: &Path) {
    let key = (repo_id, root.to_string_lossy().into_owned());
    *lifecycle_generations()
        .lock()
        .unwrap()
        .entry(key)
        .or_insert(0) += 1;
}

/// T-000243 review (eighth pass): the freshness gate every phase-A
/// lifecycle mutation passes UNDER the guard before writing. `expected`
/// is the pass's capture (`gen0`) plus its own bumps so far — equality
/// means no foreign lifecycle landed after the observation that
/// justified this write; anything else means the write is obsolete and
/// is skipped entirely (the pass seals stale anyway).
fn lifecycle_is_current(repo_id: i64, root: &Path, expected: u64) -> bool {
    lifecycle_generation(repo_id, root) == expected
}

/// A pass's OWN lifecycle transition: guarded, freshness-checked, and —
/// on a real change — generation-bumped and counted as the pass's own
/// bump, which `seal_lifecycle_gen` subtracts again. `expected` is
/// `gen0 + own_bumps` at the call site.
fn pass_role_transition(pass: &mut RepoPass, db: &AppDb, role: Option<&str>, expected: u64) {
    let guard = specs_policy_guard(pass.repo.id);
    let _g = guard.lock().unwrap();
    if !lifecycle_is_current(pass.repo.id, &pass.root, expected) {
        return;
    }
    let cur = db
        .get_shared_state(pass.repo.id)
        .ok()
        .and_then(|s| s.shared_role);
    if cur.as_deref() == role {
        return;
    }
    let _ = db.set_shared_role(pass.repo.id, role);
    bump_lifecycle_generation(pass.repo.id, &pass.root);
    pass.own_bumps += 1;
}

/// The role+status cleanup that belongs to NO pass (the loop's stale-cache
/// cleanup, the `NotFound` cleanup): ONE guarded freshness decision for
/// both writes — an obsolete observation cannot erase a newer lifecycle.
/// Returns how many generation bumps it applied, so the caller can carry
/// the resulting expected generation forward (T-000243 review, ninth
/// pass — the same pass's pin cleanup must not invalidate itself).
fn foreign_state_cleanup(db: &AppDb, repo_id: i64, root: &Path, expected: u64) -> u64 {
    let guard = specs_policy_guard(repo_id);
    let _g = guard.lock().unwrap();
    if !lifecycle_is_current(repo_id, root, expected) {
        return 0;
    }
    let st = db.get_shared_state(repo_id).ok();
    let Some(st) = st else { return 0 };
    let mut bumps = 0;
    if st.shared_role.is_some() {
        let _ = db.set_shared_role(repo_id, None);
        bump_lifecycle_generation(repo_id, root);
        bumps += 1;
    }
    if st.shared_status.is_some() {
        let _ = db.set_shared_status(repo_id, None);
        bump_lifecycle_generation(repo_id, root);
        bumps += 1;
    }
    bumps
}

/// Rebind `local_path` atomically vs pass generation capture: the guard
/// covers read-old → write → bump-both, so a pass initializing
/// concurrently sees either the old binding with its pre-bump generation
/// (then goes stale on the bumps) or the new binding with the post-bump
/// one — never a new path under an old generation (T-000243 review,
/// eighth pass).
pub fn rebind_local_path(
    db: &AppDb,
    repo_id: i64,
    new_path: Option<&str>,
) -> Result<Repository, String> {
    let guard = specs_policy_guard(repo_id);
    let _g = guard.lock().unwrap();
    let before = db.get_repository(repo_id).ok().and_then(|r| r.local_path);
    let repo = db
        .set_repo_local_path(repo_id, new_path)
        .map_err(|e| e.to_string())?;
    for p in [before.as_deref(), repo.local_path.as_deref()]
        .into_iter()
        .flatten()
    {
        bump_lifecycle_generation(repo_id, Path::new(p));
    }
    Ok(repo)
}

// ---------------------------------------------------------------------------
// T-000243: the per-repo `docs/superpowers/` policy
// ---------------------------------------------------------------------------

/// This install's own stored `share_specs` policy for `repo_id` (the owner's
/// checkbox; the default is share). What `build_structure` publishes and what
/// the pre-enable checks read — before a role exists, the owner's own stored
/// choice is the only policy there is.
fn own_share_specs(db: &AppDb, repo_id: i64) -> bool {
    db.get_shared_state(repo_id)
        .map(|s| s.share_specs)
        .unwrap_or(true)
}

/// T-000243 review: the per-repo guard that serializes a `share_specs` change
/// with the policy-sensitive work of the sync for the SAME repo. The setter
/// (`set_repo_share_specs`, an async Tauri command on another thread) takes it
/// while it reads the role, diagnoses and writes the flag; phase B holds it for
/// its whole policy section (the structure publish, the pathspecs and every
/// commit and push of that repo); the managed `.gitignore` render sites hold it
/// while they resolve the policy AND write the file; and every other writer of
/// the policy-bearing structure — `enable_shared_mode`, `approve_join_request`
/// and `disable_shared_mode` (it commits the deletion) — holds it across its
/// read→build→write sequence. Nothing inside a guard awaits or takes another
/// repo's guard, so the order cannot deadlock. Phase A is deliberately NOT
/// guarded: its `pass.share_specs` only steers the fast-forward / divergence
/// detection, and phase B resolves the policy again under the guard anyway.
pub(crate) fn specs_policy_guard(repo_id: i64) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<i64, Arc<Mutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    // The registry lock is released before anybody waits on a repo guard.
    let mut m = map.lock().unwrap();
    m.entry(repo_id)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// The validated structure of the repo's work tree, if any: the same trusted
/// read (`read_structure_file`) that settles a helper's role.
/// `Ok(None)` = ABSENT (a repo that never carried a structure, or a legacy
/// one): the default policy applies. `Err` = PRESENT BUT UNUSABLE — malformed
/// JSON, a wrong field type, or a source this install does not trust.
/// T-000243 review: that case is never folded into a policy (and least of all
/// into the default `on`, which would publish specs on a guess) — the callers
/// surface it and block the policy-sensitive operation instead.
fn validated_structure_of(
    db: &AppDb,
    repo: &Repository,
) -> Result<Option<ProjectStructure>, String> {
    let (Some(path), Some(gh)) = (repo.local_path.as_deref(), repo.github_name.as_deref()) else {
        return Ok(None);
    };
    let admins = ss::AdminVerified::load(db);
    ss::read_structure_file(Path::new(path), gh, &admins)
}

/// The effective `docs/superpowers/` policy of one repo of this install,
/// resolved the way the spec fixes it: this install being the repo's owner
/// ("main") publishes its own stored setting; a contributor / pending helper
/// accepts the owner's flag for THIS repo from the same validated structure
/// that supplied its role — never from folder existence, `.gitignore`, or a
/// sibling repo of the project. `structure` is a validated structure the
/// caller already holds (the sync pass); `None` reads the work-tree file the
/// same validated way. `Ok(true)` is the answer only when the policy really
/// resolves to share: absent structure (or no GitHub name) is the legacy
/// default, while a structure that IS present but cannot be used is `Err` —
/// the caller must block the render / commit on it, not guess. Meaningful
/// only while the repo is shared.
pub fn effective_share_specs(
    db: &AppDb,
    repo: &Repository,
    structure: Option<&ProjectStructure>,
) -> Result<bool, String> {
    let state = db.get_shared_state(repo.id).ok();
    if state
        .as_ref()
        .is_some_and(|s| s.shared_mode || s.shared_role.as_deref() == Some("main"))
    {
        // This install is the repo's owner: its own setting is the policy.
        return Ok(state.is_some_and(|s| s.share_specs));
    }
    let structure = match structure {
        Some(s) => Some(s.to_owned()),
        None => validated_structure_of(db, repo)?,
    };
    match (structure, repo.github_name.as_deref()) {
        (Some(s), Some(gh)) => Ok(ss::share_specs_for(&s, gh)),
        _ => Ok(true),
    }
}

/// T-000243 review (second pass): the authoritative structure a STANDALONE
/// render (the init-docs button, `init_docs_for_repo_in`) resolves a helper's
/// policy from — the same `discover_structure` the sync pass runs, so a
/// malformed / untrusted AUTHORITATIVE copy refuses the render instead of the
/// work-tree file (a possibly stale policy) answering. `Ok(None)`: this
/// install is the repo's owner (its policy is its own database row, which
/// `effective_share_specs` reads anyway), or no structure was found anywhere
/// (the legacy default). Reads the remote-tracking refs the last fetch left
/// (the button does not fetch); a refused or failed discovery is `Err` — the
/// render fails closed, it never guesses.
pub fn render_structure_of(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
) -> Result<Option<ProjectStructure>, String> {
    let state = db.get_shared_state(repo.id).map_err(|e| e.to_string())?;
    if state.shared_mode || state.shared_role.as_deref() == Some("main") {
        return Ok(None);
    }
    let (Some(path), Some(gh)) = (repo.local_path.as_deref(), repo.github_name.as_deref()) else {
        return Ok(None);
    };
    let root = Path::new(path);
    if !is_git_repo(root) {
        return Ok(None);
    }
    let pinned = ss::read_shared_import(db, repo.id).pinned_dev_branch;
    match ss::discover_structure(
        git,
        root,
        gh,
        pinned.as_deref(),
        &ss::AdminVerified::load(db),
    ) {
        Ok(DiscoverOutcome::Found(s, _)) => Ok(Some(s)),
        Ok(DiscoverOutcome::NotFound) => Ok(None),
        Ok(DiscoverOutcome::Untrusted(reason)) | Err(reason) => Err(reason),
    }
}

/// Whether this install's shared policy for `repo` is its OWN database row —
/// the owner side (`shared_mode` on, or the cached `main` role). The only
/// case a policy may resolve without git: a helper's policy lives in the
/// owner's published structure and must be discovered, never guessed.
pub fn policy_is_local(db: &AppDb, repo: &Repository) -> bool {
    db.get_shared_state(repo.id)
        .map(|s| s.shared_mode || s.shared_role.as_deref() == Some("main"))
        .unwrap_or(false)
}

/// `render_structure_of` with an OPTIONAL git binary. Without git no
/// authoritative discovery is possible: the owner's policy needs none
/// (`Ok(None)` — its database row answers), while a HELPER's render is
/// REFUSED (`Err`) — unavailable discovery is not evidence of absence, and
/// the work-tree copy may carry a stale ON the fetched ref has replaced.
pub fn render_structure_opt(
    db: &AppDb,
    git: Option<&GitBinary>,
    repo: &Repository,
) -> Result<Option<ProjectStructure>, String> {
    match git {
        Some(git) => render_structure_of(db, git, repo),
        None => {
            if policy_is_local(db, repo) {
                Ok(None)
            } else {
                Err("the shared specs policy cannot be resolved: git is \
                     unavailable, so the owner's published structure cannot \
                     be discovered"
                    .to_string())
            }
        }
    }
}

/// `.gitignore` template lines dropped for a shared repo: the files that travel
/// through git (spec R10), plus `docs/sdh_skills/` (the owner commits them) and
/// `docs/superpowers/` (T-000243 — but ONLY while the per-repo `share_specs`
/// policy shares it; every consumer of this list applies that flag to the
/// superpowers line, see `gitignore_template_for` / `probe_paths`).
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
    "docs/superpowers/",
];

/// T-000243: the specs folder of the shared set, as the `.gitignore` line
/// spells it.
const SPECS_IGNORE_LINE: &str = "docs/superpowers/";

/// `.gitattributes` lines added for a shared repo.
pub const SHARED_ATTRIBUTE_LINES: &[&str] = &[
    "docs/todo.md merge=sdh",
    "docs/done.md merge=sdh",
    "docs/bug-reports.md merge=sdh",
];

/// The `.gitignore` template to render for a repo: unchanged when it is not
/// shared, minus the shared paths when it is. Identical for every role.
/// T-000243: `docs/superpowers/` leaves the render only while the effective
/// `share_specs` policy shares it; opted out, the managed render MUST keep the
/// folder ignored even when the custom (or empty) template lacks the line —
/// off is "never committed", whatever the template says.
pub fn gitignore_template_for(template: &str, shared: bool, share_specs: bool) -> String {
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
    if !share_specs && !out.iter().any(|l| l.trim() == SPECS_IGNORE_LINE) {
        out.push(SPECS_IGNORE_LINE);
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
/// T-000243: the specs folder of the shared set, as a pathspec spells it.
const SPECS_DIR: &str = "docs/superpowers";
const SKILLS_EXCLUDES: [&str; 2] = [
    ":(exclude)docs/sdh_skills/*.approved",
    ":(exclude)docs/sdh_skills/*.conflict",
];

fn request_rel_path(login: &str) -> String {
    format!("{}/{}.json", REQUESTS_REL_DIR, login)
}

/// Files and folders a role commits (positive pathspecs only; see
/// `exclude_specs` for the exclusions). T-000243: `docs/superpowers/` travels
/// for BOTH the owner and a contributor — but only while `share_specs` is on;
/// a pending helper never sends specs, whatever the flag says. `docs/sdh_skills/`
/// stays main-only.
pub fn shared_pathspec(role: Role, login: &str, share_specs: bool) -> Vec<String> {
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
    if share_specs {
        v.push(SPECS_DIR.to_string());
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
    // T-000244: rule hand-back markers this branch still tracks leave it as
    // deletions. The content exclusions above keep a marker out of every other
    // plan; here the branch's own markers are enumerated directly, because a
    // contributor's / pending helper's pathspec does not name `docs/sdh_skills/`
    // at all. This is the narrow exception shared by every role, and it only
    // ever deletes — a marker is never written into a branch by this pass.
    for path in ls_tree(git, root, &refname, &[SKILLS_DIR.to_string()])?
        .keys()
        .filter(|p| is_excluded_file(p))
    {
        changes.push(FileChange {
            rel_path: PathBuf::from(path),
            content: None,
        });
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
/// Caller holds `specs_policy_guard(repo_id)` — `after_fetch` runs the import
/// inside the same hold as its freshness-gated branch section, so a repath
/// cannot slip between the generation check and the import's database and
/// work-tree writes (T-000243 review, fourteenth pass).
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
        // Whole-JSON read-modify-write under the caller-held policy guard:
        // the fresh read and the write stay atomic vs lifecycle writers
        // (disable's marker, pin, disabling) — a stale snapshot must not
        // erase them (T-000243 review, eighth pass). Blob fields are not
        // lifecycle: no bump.
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
    /// T-000243 review (second pass): why `structure` is `None` when discovery
    /// REFUSED a copy (malformed / untrusted authoritative structure): the
    /// reason must survive to the render, which may not quietly re-read the
    /// work tree — its (possibly stale) policy is exactly what the refusal
    /// rejected. `None` here + `None` in `structure` = no structure anywhere
    /// (the legacy default-on).
    structure_refused: Option<String>,
    /// T-000243: the effective `docs/superpowers/` policy this pass settled on
    /// (phase A). Phase B resolves it again: a toggle between the phases must
    /// not leave the old `on` in the deferred half of the pass.
    share_specs: bool,
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
    /// T-000243 review (third pass): phase B found the repo no longer in the
    /// state phase A captured (sharing switched off, or the cached role moved,
    /// between the phases) and cancelled the policy-sensitive half of the
    /// pass. `finish` then settles no status from it either — the writer that
    /// won the guard owns the repo's state now (e.g. a disable that left a
    /// `push_failed` status with its `disabling` marker must not have it
    /// cleared by a cancelled pass's empty notes).
    cancelled: bool,
    /// T-000243 review (sixth pass): the lifecycle generation sealed when
    /// this pass's phase-A decisions started (see `lifecycle_generation`).
    /// `None` only for passes built outside `run_phase_a` (tests): they
    /// settle unconditionally, as before.
    lifecycle_gen: Option<u64>,
    /// T-000243 review (seventh pass): lifecycle bumps THIS pass caused
    /// itself (role adoption, pin, disable-retry cleanup). `seal_lifecycle_gen`
    /// subtracts them: only a FOREIGN bump between the capture and the seal
    /// makes the pass stale.
    own_bumps: u32,
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

    /// Seal under the guard: adopt the current generation only when every
    /// bump since `g0` is this pass's own lifecycle write (`own_bumps`) —
    /// a foreign bump (another sync's transition, a disable, an approval
    /// side effect) makes the pass stale. `g0` can never match again
    /// because generations are monotonic, so a rejected seal stays stale.
    fn seal_lifecycle_gen(&mut self, g0: u64) {
        let guard = specs_policy_guard(self.repo.id);
        let _g = guard.lock().unwrap();
        let cur = lifecycle_generation(self.repo.id, &self.root);
        self.lifecycle_gen = Some(if cur == g0 + u64::from(self.own_bumps) {
            cur
        } else {
            g0
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

    /// T-000243 review (second pass): phase A's structure outcome for
    /// `repo_id` as the TRI-STATE the policy needs. `Ok(None)` = no structure
    /// anywhere (or no pass for the repo — discovery was not refused, it just
    /// did not run): the legacy default applies, and `effective_share_specs`
    /// re-reads the work tree the same validated way. `Ok(Some(_))` = a
    /// structure phase A accepted. `Err(reason)` = discovery REFUSED a copy
    /// (malformed / untrusted authoritative structure): the caller must block
    /// the policy-sensitive operation and report the reason — re-reading the
    /// work tree would hand the policy to a stale copy the refusal just
    /// rejected. (The owner's own work-tree read error in `phase_a_main` is
    /// deliberately NOT a refusal: the owner's policy is its own database
    /// row, which stays readable, and its phase B never runs with a broken
    /// file.)
    ///
    /// T-000243 review (third pass): for a repo WITHOUT a pass the `Ok(None)`
    /// arm no longer means "authoritatively absent" — phase A is
    /// project-scoped and never ran for the repo. A RENDER (the only consumer
    /// that must answer for a repo it did not discover) resolves its policy
    /// through `render_state_of` instead, which performs the discovery
    /// itself.
    pub fn structure_state_of(&self, repo_id: i64) -> Result<Option<&ProjectStructure>, String> {
        match self.passes.iter().find(|p| p.repo.id == repo_id) {
            Some(p) => match &p.structure_refused {
                Some(reason) => Err(reason.clone()),
                None => Ok(p.structure.as_ref()),
            },
            None => Ok(None),
        }
    }

    /// Did phase A produce a pass for `repo_id` in this SharedSync? `false`
    /// means discovery never ran for the repo HERE — phase A is
    /// project-scoped, and a KNOWN shared repo of another project (the server
    /// repo of a connected microservice, which this project's sync renders)
    /// gets no pass — NOT that no structure exists authoritatively.
    pub fn has_pass(&self, repo_id: i64) -> bool {
        self.passes.iter().any(|p| p.repo.id == repo_id)
    }

    /// T-000243 review (third pass): the structure state a RENDER (the managed
    /// `.gitignore` of the project sync) resolves a shared repo's policy from,
    /// distinguishing "phase A discovered this repo" from "phase A never ran
    /// for it". T-000243 review (fourth pass): a HELPER's answer is ALWAYS
    /// fresh authoritative discovery (`render_structure_opt`) — a phase-A
    /// snapshot may already be superseded by another SharedSync of this
    /// install that fetched and accepted a newer structure between the
    /// phases, and the work tree alone may hold the stale copy. Only the
    /// OWNER side still answers from its pass (its policy is its own
    /// database row anyway, so the structure matters little) or `None`.
    pub fn render_state_of(
        &self,
        db: &AppDb,
        git: Option<&GitBinary>,
        repo: &Repository,
    ) -> Result<Option<ProjectStructure>, String> {
        if policy_is_local(db, repo) {
            return match self.has_pass(repo.id) {
                // A pass of THIS sync answers with its phase-A outcome, so a
                // refusal stays a refusal.
                true => self.structure_state_of(repo.id).map(|s| s.cloned()),
                false => Ok(None),
            };
        }
        render_structure_opt(db, git, repo)
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
    // Whole-JSON read-modify-write: hold the guard so a concurrent lifecycle
    // writer (pin, disabling marker) cannot be clobbered by this snapshot.
    let guard = specs_policy_guard(repo_id);
    let _g = guard.lock().unwrap();
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
    // T-000243: this is the HISTORICAL allow-list of SDH paths, not the set of
    // files the current flag sends: a sync commit made while specs were shared
    // is still ours after the owner opts out (off stops future commits only),
    // so the specs path is always allowed here.
    let specs = shared_pathspec(Role::Main, "", true);
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
    for mut repo in repos {
        let Some(gh) = repo.github_name.clone() else {
            continue;
        };
        let Some(lp) = repo.local_path.clone() else {
            continue;
        };
        let mut root = PathBuf::from(&lp);
        if !is_git_repo(&root) {
            continue;
        }
        // T-000243 review (sixth pass): the lifecycle generation is read
        // BEFORE this repo's shared-state read and every pass decision —
        // any lifecycle writer that bumps it afterwards (under the per-repo
        // guard, whenever it completes) makes this pass stale at `finish`,
        // no matter when or where its seal happens.
        // T-000243 review (eighth pass): the binding is RE-READ under the
        // same guard hold as the capture — `set_repo_local_path` serializes
        // against this, so a pass never captures a post-repath generation
        // for a pre-repath path (or the reverse).
        let guard = specs_policy_guard(repo.id);
        let mut lifecycle_gen = {
            let _g = guard.lock().unwrap();
            match db.get_repository(repo.id).ok().and_then(|r| r.local_path) {
                // Unconditional: `Path` equality normalizes separators and
                // `.` components while the generation key is the LITERAL
                // string — an "equivalent" spelling would capture the wrong
                // key's generation (T-000243 review, ninth pass).
                Some(p) => {
                    root = PathBuf::from(&p);
                    repo.local_path = Some(p);
                }
                None => {
                    // The binding was cleared meanwhile: nothing to sync.
                    continue;
                }
            }
            lifecycle_generation(repo.id, &root)
        };
        drop(guard);
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
                pass.seal_lifecycle_gen(lifecycle_gen);
                sync.handled.insert(repo.id);
                sync.passes.push(pass);
                continue;
            };
            sync.handled.insert(repo.id);
            let mut pass = phase_a_main(db, git, &repo, &root, login, lifecycle_gen);
            pass.seal_lifecycle_gen(lifecycle_gen);
            sync.passes.push(pass);
            continue;
        }
        if ss::read_shared_import(db, repo.id).disabling.is_some() {
            // Switched off, but the deletion of `.sdh/` did not reach the remote.
            if let Some(mut pass) = retry_disabling(db, git, &repo, &root, lifecycle_gen) {
                pass.seal_lifecycle_gen(lifecycle_gen);
                sync.passes.push(pass);
            }
            continue;
        }
        if state.shared_role.as_deref() == Some("main") {
            // Stale cache: the owner switched sharing off. ONE freshness-
            // checked transition — an obsolete observation cannot erase a
            // lifecycle that landed after this repo's generation capture.
            foreign_state_cleanup(db, repo.id, &root, lifecycle_gen);
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
            // The cleanup's own pin-clear bumps the generation; the pass
            // below must capture the RESULT, not the input — otherwise it
            // seals itself stale for a transition it did not make
            // (T-000243 review, sixteenth pass).
            lifecycle_gen +=
                drop_stale_structure(db, git, &repo, &root, pass.net_ok, lifecycle_gen);
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
        if let Some(mut pass) =
            phase_a_foreign(db, git, &repo, &root, &gh, login, mode, lifecycle_gen)
        {
            sync.handled.insert(repo.id);
            for rule in &pass.canon_changed {
                if !sync.canon_changed.contains(rule) {
                    sync.canon_changed.push(rule.clone());
                }
            }
            pass.seal_lifecycle_gen(lifecycle_gen);
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
        structure_refused: None,
        share_specs: true,
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
        cancelled: false,
        lifecycle_gen: None,
        own_bumps: 0,
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
    gen0: u64,
) -> RepoPass {
    let mut pass = new_pass(repo, root, login);
    let gh = repo.github_name.clone().unwrap_or_default();
    pass.role = Some(Role::Main);
    pass_role_transition(&mut pass, db, Some("main"), gen0);
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
    // T-000243: the owner's own stored setting is the policy (its structure
    // file may still hold the previous value until phase B rewrites it). An
    // `Err` cannot happen here (the owner answers from its own database row),
    // and phase A only steers the fast-forward detection anyway — phase B
    // resolves the policy again under the per-repo guard.
    pass.share_specs = effective_share_specs(db, repo, pass.structure.as_ref()).unwrap_or(true);
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
    after_fetch(db, git, &mut pass, gen0);
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
    gen0: u64,
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
                        .detail(e.clone()),
                );
                // T-000243 review (second pass): discovery failed outright —
                // the render must not fall back to the work-tree copy either.
                pass.structure_refused = Some(e);
                return Some(pass);
            }
            return None;
        }
    };
    match found {
        DiscoverOutcome::NotFound => {
            let st = db.get_shared_state(repo.id).ok();
            let mut expected = gen0 + u64::from(pass.own_bumps);
            if st.is_some_and(|s| s.shared_role.is_some() || s.shared_status.is_some()) {
                // Repo is ordinary again (spec R15). The clear belongs to no
                // pass — but it IS a lifecycle transition gated on THIS
                // pass's generation: if a newer lifecycle landed after the
                // NotFound observation (a republish + accept), the cleanup
                // is obsolete and skipped. Its bumps carry forward so the
                // pin cleanup below is gated on the RESULT, not the input.
                expected += foreign_state_cleanup(db, repo.id, root, expected);
            }
            // Facts decide whether anything is left to clean up (nothing, for a repo
            // that never carried a structure). No pass survives this branch,
            // so the cleanup's bumps need no carrying.
            let _ = drop_stale_structure(db, git, repo, root, pass.net_ok, expected);
            None
        }
        DiscoverOutcome::Untrusted(reason) => {
            // Refused for lack of a verification? Ask about every non-owner
            // main the files name.
            for l in ss::candidate_admin_logins(git, root, gh, pinned.as_deref()) {
                pass.ask_admin_check(gh, &l);
            }
            pass.note_untrusted(reason.clone());
            // T-000243 review (second pass): the refusal survives to the
            // render (see `structure_state_of`): the `.gitignore` render is
            // refused with this reason instead of re-reading the work tree,
            // whose stale policy is what the refusal just rejected.
            pass.structure_refused = Some(reason);
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
            // T-000243 review (fifteenth pass): applying the discovered
            // structure mutates project assignments — a stale pass must not
            // write them. Guarded and freshness-checked like every other
            // lifecycle mutation; a skipped apply simply lets the newer
            // lifecycle's own pass do it.
            {
                let guard = specs_policy_guard(repo.id);
                let _g = guard.lock().unwrap();
                if lifecycle_is_current(repo.id, root, gen0 + u64::from(pass.own_bumps)) {
                    if let Err(e) = ss::apply_structure_locally(db, repo.id, &s) {
                        pass.issue(
                            repo_issue(Code::ManagedFileFailed, repo)
                                .target(STRUCTURE_REL_PATH)
                                .detail(e),
                        );
                    }
                }
            }
            if pinned.is_none() {
                // Pinning is this pass's own lifecycle transition: guarded,
                // freshness-checked against THIS pass's generation, bumped,
                // and counted so the seal forgives it.
                let guard = specs_policy_guard(repo.id);
                let _g = guard.lock().unwrap();
                if lifecycle_is_current(repo.id, root, gen0 + u64::from(pass.own_bumps))
                    && ss::pin_after_accept(db, repo.id, &s).is_ok()
                {
                    bump_lifecycle_generation(repo.id, root);
                    pass.own_bumps += 1;
                }
            }
            let role = ss::derive_role(Some(&s), Some(login));
            let Some(role) = role else {
                return Some(pass);
            };
            let expected = gen0 + u64::from(pass.own_bumps);
            pass_role_transition(&mut pass, db, Some(role.as_str()), expected);
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
                remove_own_stale_request(git, &pass, gen0 + u64::from(pass.own_bumps));
            }
            // Р16: this install does not render rule copies here; copies an
            // earlier version (or the time before the join) left untracked
            // would stop the owner's from being fast-forwarded in.
            if role != Role::Main {
                drop_untracked_rule_copies(git, repo.id, root, gen0 + u64::from(pass.own_bumps));
            }
            // T-000243: the helper takes the specs policy from the very
            // structure that gave it the role, before any path is computed.
            // Phase A only steers the fast-forward detection with it; phase B
            // resolves the policy again under the per-repo guard.
            pass.share_specs = effective_share_specs(db, repo, Some(&s)).unwrap_or(true);
            pass.structure = Some(s);
            after_fetch(db, git, &mut pass, gen0);
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
fn drop_stale_structure(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
    root: &Path,
    net_ok: bool,
    expected_gen: u64,
) -> u64 {
    // T-000243 review (tenth pass): the WHOLE cleanup — the switch-off
    // evidence check, the deletion commits, the pushes and the pin
    // settlement — runs under the policy guard. Every lifecycle writer
    // needs the same guard, so the generation cannot move while it is
    // held: the freshness check below stays valid for every step, and a
    // republished structure can never be deleted by an obsolete pass.
    // Returns how many lifecycle bumps it performed: the caller's own
    // pass must count them or it seals itself stale (T-000243 review,
    // sixteenth pass).
    let guard = specs_policy_guard(repo.id);
    let _g = guard.lock().unwrap();
    if !lifecycle_is_current(repo.id, root, expected_gen) {
        return 0;
    }
    let mut imp = ss::read_shared_import(db, repo.id);
    let Some(pin) = imp.pinned_dev_branch.clone() else {
        return 0;
    };
    if !repo_ready(git, root) {
        return 0;
    }
    // The switch-off must be established by origin's pinned dev branch itself.
    if tracks_structure(git, root, &remote_ref(&pin)) != Some(false) {
        return 0;
    }
    // The guard is already held — `finish` must NOT re-acquire it.
    let finish = |db: &AppDb, _imp: &mut ss::SharedImport| -> u64 {
        if !root.join(STRUCTURE_REL_PATH).exists() {
            // Clearing the pin is a lifecycle transition gated on the pin
            // still being the one the evidence was gathered for — a newer
            // accept may have pinned a different dev branch meanwhile.
            let mut fresh = ss::read_shared_import(db, repo.id);
            if fresh.pinned_dev_branch.as_deref() != Some(pin.as_str()) {
                return 0;
            }
            fresh.pinned_dev_branch = None;
            if ss::write_shared_import(db, repo.id, &fresh).is_ok() {
                bump_lifecycle_generation(repo.id, root);
                return 1;
            }
        }
        0
    };
    let Some(own) = own_branch_of(db, repo.id) else {
        return finish(db, &mut imp);
    };
    let sdh = vec![STRUCTURE_REL_PATH.to_string(), REQUESTS_REL_DIR.to_string()];
    let login = db.github_login().unwrap_or_default();
    let remote = remote_ref(&own);
    let cur = current_branch(git, root).ok().flatten();
    let is_current = cur.as_deref().is_some_and(|c| same_branch(c, &own));
    if tracks_structure(git, root, &local_ref(&own)) != Some(true)
        && tracks_structure(git, root, &remote) != Some(true)
    {
        return finish(db, &mut imp);
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
            return 0;
        }
    }
    if net_ok && remote_sha.is_some() {
        let _ = git_sync::push_branches(git, root, REMOTE, &[own.as_str()]);
    }
    if tracks_structure(git, root, &local_ref(&own)) != Some(true)
        && tracks_structure(git, root, &remote) != Some(true)
    {
        return finish(db, &mut imp);
    }
    0
}

/// The owner's deletion of `.sdh/` was committed but not pushed: push it now.
/// The repo is not shared any more; this is the only thing left to do for it.
fn retry_disabling(
    db: &AppDb,
    git: &GitBinary,
    repo: &Repository,
    root: &Path,
    gen0: u64,
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
    #[cfg(test)]
    retry_post_fetch_probe(repo.id, root);
    // T-000243 review (eleventh pass): from here on the retry MUTATES —
    // branch fast-forwards/resets, fresh deletion commits, the push. All of
    // it runs under the policy guard, gated on the generation the retry was
    // created under: a re-enable (or any newer lifecycle) that landed during
    // the fetch makes this retry obsolete BEFORE the first mutation, not
    // just before the database cleanup.
    let guard = specs_policy_guard(repo.id);
    let _g = guard.lock().unwrap();
    if lifecycle_generation(repo.id, root) != gen0 {
        return None;
    }
    let imp_now = ss::read_shared_import(db, repo.id);
    if imp_now.disabling.as_deref() != Some(dev.as_str()) {
        return None;
    }
    // The dev branch and the participant branches whose deletion is pending.
    let mut branches: Vec<String> = vec![dev.clone()];
    branches.extend(imp_now.disabling_branches.iter().cloned());
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
            // The clear bumps the generation; the pass is dropped either
            // way, so there is no settlement left to account for.
            let _ = clear_disabling_locked(db, repo.id, root);
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
            let _ = clear_disabling_locked(db, repo.id, root);
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

/// Caller holds `specs_policy_guard(repo_id)` — the clear and the
/// generation bump are already atomic with `finish`'s freshness check.
fn clear_disabling_locked(db: &AppDb, repo_id: i64, root: &Path) -> Result<(), String> {
    let mut imp = ss::read_shared_import(db, repo_id);
    imp.disabling = None;
    imp.disabling_branches.clear();
    ss::write_shared_import(db, repo_id, &imp)?;
    db.set_shared_status(repo_id, None)
        .map_err(|e| e.to_string())?;
    bump_lifecycle_generation(repo_id, root);
    Ok(())
}

/// `gen0` is the lifecycle generation the retry pass was created under:
/// only THAT operation may clear the marker. A newer disable / re-enable /
/// another retry's cleanup bumped the generation meanwhile — clearing then
/// would erase work this retry does not own (T-000243 review, seventh pass).
/// Returns `Ok(true)` when the marker was actually cleared.
/// Production clears go through `retry_disabling`'s guard-held section;
/// this wrapper remains for the generation-gate regression test.
#[cfg(test)]
fn clear_disabling(db: &AppDb, repo_id: i64, root: &Path, gen0: u64) -> Result<bool, String> {
    let guard = specs_policy_guard(repo_id);
    let _guard = guard.lock().unwrap();
    if lifecycle_generation(repo_id, root) != gen0 {
        return Ok(false);
    }
    clear_disabling_locked(db, repo_id, root)?;
    Ok(true)
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
/// contributor; remove it when git does not track it. The tracked check and
/// the delete run under the policy guard: a concurrent pass's fast-forward
/// may have made the file tracked since this pass last looked, and deleting
/// a tracked file would publish its removal (T-000243 review, fifteenth
/// pass — same check-then-delete race as `drop_untracked_rule_copies`).
fn remove_own_stale_request(git: &GitBinary, pass: &RepoPass, expected: u64) {
    let rel = request_rel_path(&pass.login);
    let abs = pass.root.join(&rel);
    if !abs.is_file() {
        return;
    }
    let guard = specs_policy_guard(pass.repo.id);
    let _g = guard.lock().unwrap();
    // T-000243 review (sixteenth pass): serializing is not enough — a repath
    // that landed since the pass captured its generation makes this delete
    // obsolete even though the mutex is free. Skip it on the detached
    // checkout; the pass seals stale at `finish` regardless.
    if !lifecycle_is_current(pass.repo.id, &pass.root, expected) {
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
/// The tracked set and the deletes run under the policy guard: a concurrent
/// pass's fast-forward may have made a copy tracked since this pass last
/// looked, and deleting a tracked file would publish its removal on the next
/// commit (T-000243 review, fifteenth pass).
fn drop_untracked_rule_copies(git: &GitBinary, repo_id: i64, root: &Path, expected: u64) {
    let guard = specs_policy_guard(repo_id);
    let _g = guard.lock().unwrap();
    // T-000243 review (sixteenth pass): same freshness gate as
    // `remove_own_stale_request` — a repathed pass must not delete files on
    // the checkout it no longer owns.
    if !lifecycle_is_current(repo_id, root, expected) {
        return;
    }
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
/// Nothing is touched unless the repo is `repo_ready`. `gen0` is the
/// generation the pass captured: everything below mutates — the branch
/// section (`reset_to_origin` can roll a branch back over a disable's
/// unpushed deletion commit), the incoming import and the canon adoption —
/// so it ALL runs under the policy guard, gated on the pass's freshness
/// BEFORE the first mutation (T-000243 review, twelfth and fourteenth
/// passes). The import resolves the repo's CURRENT binding during
/// reconcile: a repath that landed during the fetch must not let this
/// pass write the old checkout's rows into the new checkout's files.
fn after_fetch(db: &AppDb, git: &GitBinary, pass: &mut RepoPass, gen0: u64) {
    let Some(role) = pass.role else { return };
    if !repo_ready(git, &pass.root) {
        return;
    }
    let cur = current_branch(git, &pass.root).ok().flatten();
    let pathspec = shared_pathspec(role, &pass.login, pass.share_specs);
    let recipients = pass.recipients.clone();
    #[cfg(test)]
    after_fetch_probe(pass.repo.id, &pass.root);
    {
        let guard = specs_policy_guard(pass.repo.id);
        let _g = guard.lock().unwrap();
        // A lifecycle that landed during the fetch (disable, re-enable,
        // approval, repath) makes every side effect below obsolete — the
        // branch resets AND the import/canon tail. The pass seals stale at
        // `finish` regardless (T-000243 review, fourteenth pass).
        if lifecycle_is_current(pass.repo.id, &pass.root, gen0 + u64::from(pass.own_bumps)) {
            for b in &recipients {
                let remote = remote_ref(b);
                let has_remote =
                    matches!(git_sync::rev_parse(git, &pass.root, &remote), Ok(Some(_)));
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
                            let reset = tip.is_some_and(|t| {
                                reset_to_origin(git, &pass.root, b, &t).unwrap_or(false)
                            });
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
            // T-000243 review (fourteenth pass): the import and the canon
            // adoption are side effects of the same observation — they run
            // inside this hold, gated by the same freshness check. A repath
            // during the fetch can no longer leave this pass importing the
            // old checkout's rows into the new binding's files.
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
            // T-000243 review (fourth pass): ONE guard hold covers phase B
            // AND the status settlement — a writer that flipped the repo's
            // shared state between the phases (disable / approve / enable /
            // the specs setter) cannot land between the policy work and the
            // status write.
            let guard = specs_policy_guard(pass.repo.id);
            let _guard = guard.lock().unwrap();
            // T-000243 review (sixth pass): settle only when the lifecycle
            // generation this pass's decisions started under is still
            // current. A disable that completed — even one that finished
            // BETWEEN the pass's discovery and its seal — owns the repo's
            // status (including a `push_failed` it is still retrying), and
            // so does an enable or a status-resolution action; a stale pass
            // must not overwrite it. Passes phase B never revalidates (no
            // git, or a pass refused back in phase A) get the same
            // protection here. This is NOT a shared-membership check: a
            // first-discovery `Untrusted` pass or a `retry_disabling` pass
            // legitimately settles onto a repo that is not shared — only a
            // lifecycle that MOVED after the pass's decisions began makes
            // it stale, and the generation sees round trips a state
            // snapshot cannot.
            // T-000243 review (thirteenth pass): the same gate covers PHASE
            // B — a pass whose lifecycle moved (repath, disable, re-enable,
            // approval) between its decisions and now must not commit or
            // push from a checkout it no longer owns. The check and phase
            // B share this one guard hold, so nothing can interleave.
            let stale = pass
                .lifecycle_gen
                .is_some_and(|gen| gen != lifecycle_generation(pass.repo.id, &pass.root));
            if !stale {
                if let (Some(git), Some(_)) = (git, pass.role) {
                    phase_b_repo(db, git, &mut pass);
                }
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
            // T-000243 review (third pass): a pass cancelled by phase B's
            // revalidation settles no status — the writer that changed the
            // repo's state under the guard owns it (see `RepoPass::cancelled`).
            if !pass.cancelled && !stale {
                settle_status(db, &pass, &mut out);
            }
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
fn plumb_pathspec(role: Role, login: &str, share_specs: bool) -> Vec<String> {
    shared_pathspec(role, login, share_specs)
        .into_iter()
        .filter(|p| !MANAGED_ROOT_FILES.contains(&p.as_str()))
        .collect()
}

// T-000243 review: a test-only rendezvous inside phase B's policy section —
// fired after the per-repo guard is taken and before the policy is resolved.
// Production never installs one; the concurrency tests use it to interleave
// `set_repo_share_specs` (another thread) with an in-flight phase B without
// relying on timing. The probe is matched by repo id AND repo root (the temp
// dir of the test that installed it), so parallel tests of the same process
// never fire somebody else's rendezvous, and it is consumed by the first pass
// that sees it (later passes of the same test run normally).
//
// T-000243 review (third pass): the registry is a MAP keyed by
// (repo id, repo root), not a single slot — a single `Option` let one test's
// install drop another test's sender wholesale, and a test running in
// parallel (`--test-threads=2`, the default is more) then failed with "the
// probe never fired: Disconnected". An entry whose test has ended is inert:
// its key contains a temp dir that no later pass ever matches.
#[cfg(test)]
struct PhaseBPolicyProbe {
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
}

#[cfg(test)]
static PHASE_B_POLICY_PROBES: OnceLock<Mutex<HashMap<(i64, PathBuf), PhaseBPolicyProbe>>> =
    OnceLock::new();

/// Install the rendezvous for the repo at `root`: its phase B sends the repo
/// id on `entered` and then waits for one message on `release` while it holds
/// the per-repo guard. Installing for a key replaces only that key's probe;
/// other tests' probes are untouched.
#[cfg(test)]
fn install_phase_b_policy_probe(
    repo: i64,
    root: &Path,
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
) {
    let cell = PHASE_B_POLICY_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    cell.lock().unwrap().insert(
        (repo, root.to_path_buf()),
        PhaseBPolicyProbe { entered, release },
    );
}

#[cfg(test)]
fn phase_b_policy_probe(repo_id: i64, root: &Path) {
    let cell = PHASE_B_POLICY_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    // The registry lock is never held while the pass waits for the release.
    let probe = {
        let mut g = cell.lock().unwrap();
        g.remove(&(repo_id, root.to_path_buf()))
    };
    if let Some(p) = probe {
        let _ = p.entered.send(repo_id);
        // The pass holds the per-repo guard while it waits, so whatever the
        // test interleaves here cannot sneak into this section.
        let _ = p.release.recv();
    }
}

// T-000243 review (second pass): the same rendezvous mechanism for the
// managed `.gitignore` RENDER sites — fired right AFTER the file write, while
// the per-repo policy guard is still held (the guard must outlive the write,
// not just the policy read: a render computed with the old policy may not
// land after an OFF save or overwrite a newer OFF render). The concurrency
// tests use it to prove a specs save / phase B cannot interleave between the
// read and the write. Matched by repo id AND repo root, like the phase B
// probe; production never installs one. T-000243 review (third pass): the
// registry is a MAP keyed by (repo id, repo root) for the same
// parallel-tests reason as `PHASE_B_POLICY_PROBES` above.
#[cfg(test)]
struct RenderWriteProbe {
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
}

#[cfg(test)]
static RENDER_WRITE_PROBES: OnceLock<Mutex<HashMap<(i64, PathBuf), RenderWriteProbe>>> =
    OnceLock::new();

/// Install the render rendezvous for the repo at `root`: the render sends the
/// repo id on `entered` after it wrote `.gitignore` (still holding the guard)
/// and waits for one message on `release`. Installing for a key replaces only
/// that key's probe; other tests' probes are untouched.
#[cfg(test)]
pub(crate) fn install_render_write_probe(
    repo: i64,
    root: &Path,
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
) {
    let cell = RENDER_WRITE_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    cell.lock().unwrap().insert(
        (repo, root.to_path_buf()),
        RenderWriteProbe { entered, release },
    );
}

/// Fire the installed render probe, if any. Callers call this INSIDE the
/// held policy guard, after the `.gitignore` write.
#[cfg(test)]
pub(crate) fn render_write_probe(repo_id: i64, root: &Path) {
    let cell = RENDER_WRITE_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    let probe = {
        let mut g = cell.lock().unwrap();
        g.remove(&(repo_id, root.to_path_buf()))
    };
    if let Some(p) = probe {
        let _ = p.entered.send(repo_id);
        // The render still holds the per-repo guard while it waits.
        let _ = p.release.recv();
    }
}

// T-000243 review (eleventh pass): a rendezvous for `retry_disabling`'s
// post-fetch window. The fetch is the retry's last read-only step —
// everything after it mutates (branch refresh, deletion commits, push).
// The probe fires AFTER the fetch and BEFORE the policy guard is taken,
// so a test can land a re-enable exactly in the window the review found:
// the retry must then refuse before its FIRST mutation, not merely before
// the database cleanup. Matched by (repo id, repo root); fires once.
#[cfg(test)]
struct RetryPostFetchProbe {
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
}

#[cfg(test)]
static RETRY_POST_FETCH_PROBES: OnceLock<Mutex<HashMap<(i64, PathBuf), RetryPostFetchProbe>>> =
    OnceLock::new();

#[cfg(test)]
fn install_retry_post_fetch_probe(
    repo: i64,
    root: &Path,
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
) {
    let cell = RETRY_POST_FETCH_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    cell.lock().unwrap().insert(
        (repo, root.to_path_buf()),
        RetryPostFetchProbe { entered, release },
    );
}

#[cfg(test)]
fn retry_post_fetch_probe(repo_id: i64, root: &Path) {
    let cell = RETRY_POST_FETCH_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    let probe = {
        let mut g = cell.lock().unwrap();
        g.remove(&(repo_id, root.to_path_buf()))
    };
    if let Some(p) = probe {
        let _ = p.entered.send(repo_id);
        // The retry has NOT taken the policy guard yet — whatever the test
        // interleaves here completes before the retry's freshness check.
        let _ = p.release.recv();
    }
}

// T-000243 review (twelfth pass): a rendezvous for `after_fetch`'s branch
// section — the fast-forwards and `reset_to_origin` there MUTATE refs, so
// they run under the policy guard gated on the pass's generation. The
// probe fires AFTER the fetch, BEFORE the guard is taken: a test can land
// a whole disable + republish in exactly the window the review found.
// Matched by (repo id, repo root); fires once.
#[cfg(test)]
struct AfterFetchProbe {
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
}

/// Sends the rendezvous release on drop: a panicking interleave inside
/// `thread::scope` must still free the parked worker, or the scope's join
/// hangs forever (T-000243 review, fifteenth pass).
#[cfg(test)]
struct ReleaseOnDrop(std::sync::mpsc::Sender<i64>);

#[cfg(test)]
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        let _ = self.0.send(1);
    }
}

#[cfg(test)]
static AFTER_FETCH_PROBES: OnceLock<Mutex<HashMap<(i64, PathBuf), AfterFetchProbe>>> =
    OnceLock::new();

#[cfg(test)]
fn install_after_fetch_probe(
    repo: i64,
    root: &Path,
    entered: std::sync::mpsc::Sender<i64>,
    release: std::sync::mpsc::Receiver<i64>,
) {
    let cell = AFTER_FETCH_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    cell.lock().unwrap().insert(
        (repo, root.to_path_buf()),
        AfterFetchProbe { entered, release },
    );
}

#[cfg(test)]
fn after_fetch_probe(repo_id: i64, root: &Path) {
    let cell = AFTER_FETCH_PROBES.get_or_init(|| Mutex::new(HashMap::new()));
    let probe = {
        let mut g = cell.lock().unwrap();
        g.remove(&(repo_id, root.to_path_buf()))
    };
    if let Some(p) = probe {
        let _ = p.entered.send(repo_id);
        // The pass has NOT taken the policy guard yet — whatever the test
        // interleaves here completes before the freshness check.
        let _ = p.release.recv();
    }
}

/// T-000243 review (third pass): revalidate a phase-A pass under the phase-B
/// guard. The pass was captured before the skeleton writes; a writer of the
/// policy-bearing structure (`approve_join_request`, `disable_shared_mode`,
/// `enable_shared_mode`, the specs setter) may have completed between the
/// phases — serialized by the same guard, but only for its own run. The
/// deferred half therefore re-reads the CURRENT state instead of trusting the
/// snapshot:
///
/// * the shared state from the database: a repo that stopped sharing (the
///   switch-off landed in between) or whose cached role moved CANCELS the
///   whole policy section — no structure publish, no commit, no push of the
///   stale state — and marks the pass so `finish` settles no status from it
///   either (the writer that won owns the repo's state now);
/// * the owner's structure from the WORK TREE: an approval that landed in
///   between wrote its participant there, and `write_main_structure` rebuilds
///   from `existing` — rebuilding from the phase-A snapshot would silently
///   drop the participant and re-publish the stale list. A file that is
///   present but unusable fails closed exactly like phase A (`untrusted`,
///   no policy work this pass). The recipients follow the refreshed
///   structure, so a participant approved in between is plumbed and pushed
///   in this very pass, whenever its branch already exists locally (a fresh
///   participant's branch is still created by the next phase A).
///
/// * a HELPER's structure is re-DISCOVERED, not re-read from the work tree:
///   another SharedSync of this install may have fetched and accepted a
///   NEWER authoritative copy between the phases (its fetch already moved
///   the remote-tracking refs), so the phase-A snapshot may carry a stale
///   ON — or miss a newer refusal — exactly like the owner's work-tree file
///   could. Discovery runs the same `discover_structure` phase A ran,
///   against the pinned / matching origin ref. A structure that derives a
///   DIFFERENT role for this install (approved elsewhere, removed) or is
///   gone entirely CANCELS the pass — the next phase A re-derives
///   everything; a refusal fails closed with the untrusted status, as in
///   phase A.
///
/// Returns `true` when the pass may proceed.
fn revalidate_pass_under_guard(db: &AppDb, git: &GitBinary, pass: &mut RepoPass) -> bool {
    let Some(role) = pass.role else { return false };
    let state = match db.get_shared_state(pass.repo.id) {
        Ok(s) => s,
        Err(_) => {
            pass.cancelled = true;
            return false;
        }
    };
    let role_still = match role {
        Role::Main => state.shared_mode,
        Role::Contributor => {
            !state.shared_mode && state.shared_role.as_deref() == Some("contributor")
        }
        Role::Pending => !state.shared_mode && state.shared_role.as_deref() == Some("pending"),
    };
    if !role_still {
        pass.cancelled = true;
        return false;
    }
    match role {
        Role::Main => {
            let Some(gh) = pass.repo.github_name.clone() else {
                pass.cancelled = true;
                return false;
            };
            match ss::read_structure_file(&pass.root, &gh, &ss::AdminVerified::load(db)) {
                Ok(s) => {
                    pass.structure = s;
                    if let Some(own) = pass.own_branch.clone() {
                        let mut rec = vec![own];
                        if let Some(s) = &pass.structure {
                            rec.extend(s.participants.iter().map(|p| p.branch.clone()));
                        }
                        pass.recipients = clean_recipients(rec);
                    }
                }
                Err(reason) => {
                    pass.note_untrusted(reason);
                    pass.role = None;
                    return false;
                }
            }
        }
        Role::Contributor | Role::Pending => {
            let (Some(gh), Some(_)) = (pass.repo.github_name.clone(), pass.repo.local_path.clone())
            else {
                // Phase A could not have discovered without these either;
                // the snapshot is all there is.
                return true;
            };
            let pinned = ss::read_shared_import(db, pass.repo.id).pinned_dev_branch;
            let admins = ss::AdminVerified::load(db);
            match ss::discover_structure(git, &pass.root, &gh, pinned.as_deref(), &admins) {
                Ok(DiscoverOutcome::Found(s, _)) => {
                    if ss::derive_role(Some(&s), Some(pass.login.as_str())) != Some(role) {
                        pass.cancelled = true;
                        return false;
                    }
                    let main_dev = s.main.dev_branch.clone();
                    pass.structure = Some(s);
                    if let Some(own) = pass.own_branch.clone() {
                        pass.recipients = match role {
                            Role::Contributor => clean_recipients(vec![own, main_dev]),
                            _ => clean_recipients(vec![main_dev]),
                        };
                    }
                }
                Ok(DiscoverOutcome::NotFound) => {
                    pass.cancelled = true;
                    return false;
                }
                Ok(DiscoverOutcome::Untrusted(reason)) | Err(reason) => {
                    pass.note_untrusted(reason);
                    return false;
                }
            }
        }
    }
    true
}

fn phase_b_repo(db: &AppDb, git: &GitBinary, pass: &mut RepoPass) {
    let Some(role) = pass.role else { return };
    // T-000243 review: one coherent policy snapshot for the whole
    // policy-sensitive half of the pass. The CALLER (`SharedSync::finish`)
    // holds the per-repo guard from here to the status settlement — the
    // structure publish (which carries the flag), the `.gitignore`-driven
    // enumeration, the pathspecs and every commit and push of THIS repo —
    // so `set_repo_share_specs` (another thread) is never acknowledged in
    // the middle of it: either this whole section completes under the old
    // flag and the setter applies after it, or the setter lands first and
    // this section reads the new flag. The managed `.gitignore` render of
    // `project_sync` takes the same guard, so a render cannot read a flag
    // that is being written either. The residual window is a render that
    // finished with the old policy just before the setter landed: the file
    // on disk is then stale until the next render, but nothing is committed
    // with it (phase B resolves the policy again under the guard), so it
    // self-heals without ever publishing the wrong files.
    #[cfg(test)]
    phase_b_policy_probe(pass.repo.id, &pass.root);
    // Same guard as phase A, checked again: the repo may have changed since.
    if !repo_ready(git, &pass.root) {
        return;
    }
    // T-000243 review (third pass): REVALIDATE the pass under the guard — a
    // writer of the policy-bearing structure (`approve_join_request`,
    // `disable_shared_mode`, `enable_shared_mode`) may have completed BETWEEN
    // the phases; each of them takes this same guard, but only for its own
    // run, so the phase-A snapshot may not be trusted by the deferred half.
    if !revalidate_pass_under_guard(db, git, pass) {
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
    // T-000243: resolved HERE, not carried from phase A — a toggle between the
    // phases must not leave the old `on` in this deferred half of the pass
    // (the owner's setting is read fresh from the database). The per-repo
    // guard taken above makes this read and everything derived from it one
    // coherent snapshot against the setter.
    let share_specs = match effective_share_specs(db, &pass.repo, pass.structure.as_ref()) {
        Ok(v) => v,
        Err(e) => {
            // T-000243 review: a structure that is present but unusable
            // (malformed / untrusted) must not become a guessed `on` — no
            // shared file of this repo is committed this pass and the issue
            // says why. The repo is reported, never silently rendered.
            pass.issue(
                repo_issue(Code::SharedUntrusted, &pass.repo)
                    .target(STRUCTURE_REL_PATH)
                    .detail(e),
            );
            return;
        }
    };
    diagnose_specs_blocking(git, pass, role, share_specs);
    let pathspec = shared_pathspec(role, &login, share_specs);
    let plumb_spec = plumb_pathspec(role, &login, share_specs);
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
            // T-000244: markers this branch still tracks leave it as deletions
            // — even with nothing else to commit and with `docs/sdh_skills/`
            // gone from disk (the list is read from HEAD). The same sync
            // subject and identity as every other commit of this pass, so the
            // branch still reads as holding nothing but our own sync commits.
            let markers = match list_markers_in_rev(git, &root, "HEAD") {
                Ok(m) => m,
                Err(e) => {
                    let issue = repo_issue(Code::AutocommitFailed, &pass.repo).detail(e);
                    pass.issue(issue);
                    continue;
                }
            };
            if active.is_empty() && markers.is_empty() {
                continue;
            }
            let paths: Vec<PathBuf> = active.iter().map(PathBuf::from).collect();
            let (subject, body) = commit_message(&login);
            let pre_head = git_sync::rev_parse(git, &root, "HEAD").ok().flatten();
            let outcome = if markers.is_empty() {
                let mut pathspec: Vec<PathBuf> = paths.clone();
                pathspec.extend(exclude_specs(&active).into_iter().map(PathBuf::from));
                commit_paths(
                    git,
                    &root,
                    b,
                    &pathspec,
                    &subject,
                    Some(&body),
                    SDH_COMMIT_NAME,
                    SDH_COMMIT_EMAIL,
                )
            } else {
                // Index-only: the commit is built from HEAD plus the allowed
                // shared paths, minus every marker it holds. A live marker can
                // therefore be deleted but never added or modified.
                commit_paths_index_only(
                    git,
                    &root,
                    b,
                    &paths,
                    &subject,
                    Some(&body),
                    SDH_COMMIT_NAME,
                    SDH_COMMIT_EMAIL,
                )
            };
            match outcome {
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
/// T-000243: with the specs policy off, `docs/superpowers/` is not a shared
/// path — a user rule ignoring it is the user's own business and is neither
/// probed nor offered for removal.
fn probe_paths(share_specs: bool) -> Vec<String> {
    let mut v: Vec<String> = SHARED_UNIGNORED_LINES
        .iter()
        .filter(|l| share_specs || **l != SPECS_IGNORE_LINE)
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

/// User-section lines of the repo's own `.gitignore` that still ignore one of
/// `probes` (a file path, probed with `git check-ignore -v`). Lines of the
/// managed block are not reported (the sync rewrites it).
fn ignoring_lines(
    git: &GitBinary,
    root: &Path,
    probes: &[String],
) -> Result<Vec<IgnoreLine>, String> {
    let mut input = Vec::new();
    for p in probes {
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

/// User-section lines of the repo's own `.gitignore` that still ignore a shared
/// path. T-000243: `share_specs` filters the specs folder out of the probe set
/// when the policy is off (see `probe_paths`).
pub fn blocking_ignore_lines(
    git: &GitBinary,
    root: &Path,
    share_specs: bool,
) -> Result<Vec<IgnoreLine>, String> {
    ignoring_lines(git, root, &probe_paths(share_specs))
}

/// T-000243 review: user-section lines that still ignore `docs/superpowers/`
/// while the effective policy shares it. The shared enumeration honours
/// `.gitignore` (`ls-files --others --exclude-standard`), so such a rule makes
/// the folder silently never travel — the recurring pass diagnoses it here,
/// while the removal itself stays behind the explicit-confirmation preflight
/// flow of the shared switch. Covers a repo that was enabled with the flag off
/// and switched on later, and one that upgraded to this feature already shared.
fn specs_blocking_lines(git: &GitBinary, root: &Path) -> Result<Vec<IgnoreLine>, String> {
    ignoring_lines(git, root, &[format!("{}sdh-probe.md", SPECS_IGNORE_LINE)])
}

/// The recurring diagnosis of the ON policy: user rules that keep the specs
/// folder ignored are reported once per change (`fresh_issues`), never removed.
fn diagnose_specs_blocking(git: &GitBinary, pass: &mut RepoPass, role: Role, share_specs: bool) {
    if !share_specs || role == Role::Pending {
        return;
    }
    let blocking = match specs_blocking_lines(git, &pass.root) {
        Ok(b) => b,
        Err(e) => {
            pass.issue(
                repo_issue(Code::ManagedFileFailed, &pass.repo)
                    .target(".gitignore")
                    .detail(format!(
                        "could not check whether docs/superpowers/ is ignored: {}",
                        e
                    )),
            );
            return;
        }
    };
    if blocking.is_empty() {
        return;
    }
    let rules: Vec<&str> = blocking.iter().map(|l| l.text.as_str()).collect();
    pass.issue(
        repo_issue(Code::ManagedFileFailed, &pass.repo)
            .target(".gitignore")
            .detail(format!(
                "user rule(s) [{}] still ignore docs/superpowers/: the specs policy is on but the \
             folder is ignored, so specs never travel. Remove the rule(s) in the preflight of \
             the shared switch (or switch the specs off); they are never removed automatically",
                rules.join(", ")
            )),
    );
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
    // T-000243: the owner's own stored policy — preflight also runs before a
    // role exists, and with specs opted out the folder is not a shared path,
    // so a user rule ignoring it is not a blocker to offer for removal.
    let share_specs = own_share_specs(db, repo_id);
    Ok(SharedPreflight {
        blocking_lines: blocking_ignore_lines(git, &root, share_specs)?,
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
    let login = db.github_login().ok_or_else(|| {
        "the GitHub login of this install is unknown - save the token again".to_string()
    })?;
    let gh = repo
        .github_name
        .clone()
        .ok_or_else(|| "shared mode needs a GitHub repository".to_string())?;
    // T-000243 review (second pass): the whole read→build→write sequence of
    // the POLICY-BEARING structure runs under the per-repo guard (see
    // `specs_policy_guard`). Without it an enable could interleave with a
    // specs save / a sync's policy section: the structure built from the old
    // flag would be written after the sync already published the new one, and
    // the next pass would commit the stale policy. The DB flips sit inside
    // the guard as well, so the mode and the published structure are one
    // coherent snapshot.
    let guard = specs_policy_guard(repo_id);
    let _guard = guard.lock().unwrap();
    // T-000243 review (ninth pass): re-read the binding UNDER the guard —
    // a repath that completed while this command queued must not leave the
    // structure work (or the generation bump) on the obsolete checkout.
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    let existing = ss::read_structure_file(&root, &gh, &ss::AdminVerified::load(db))?;
    // Validates: owner or administrator, project, dev branch.
    let built = ss::build_structure(db, repo_id, &login, existing.as_ref())?;
    // T-000243: the blocking check follows the policy being published — with
    // specs opted out the folder stays ignored and its rules are no blocker.
    let blocking = blocking_ignore_lines(git, &root, own_share_specs(db, repo_id))?;
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
    // T-000243 review (sixth pass): the lifecycle moved — passes sealed
    // under the pre-enable generation settle no status.
    bump_lifecycle_generation(repo_id, &root);
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

/// T-000243: store the owner's `docs/superpowers/` choice for `repo_id`.
/// Refused unless this install knows its GitHub login and may act as the
/// repo's main — the owner, or an administrator verified by the frontend (the
/// same trust rule as switching shared mode on) — so a contributor, a pending
/// helper or an unknown login cannot flip the flag, not even through a direct
/// IPC call. The ROLE is refused before the capability: a verified
/// administrator of an organization this install participates in as a
/// contributor / pending helper is still not that repo's main here.
/// Settable before sharing is enabled; the structure publish, the
/// `.gitignore` render and the pathspecs follow on the next sync. The write
/// and its diagnosis run under the per-repo policy guard, so the answer is
/// never given while a sync's policy section could still commit the old flag.
/// Returns a warning note when the flag was switched ON for a repo that
/// already shares and user rules still keep the folder ignored.
pub fn set_repo_share_specs(db: &AppDb, repo_id: i64, on: bool) -> Result<Option<String>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let gh = repo
        .github_name
        .clone()
        .ok_or_else(|| "the specs switch needs a repository linked to GitHub".to_string())?;
    let login = db.github_login().ok_or_else(|| {
        "the GitHub login of this install is unknown - save the token again".to_string()
    })?;
    // T-000243 review: serialize with the sync's policy section of this repo
    // (see `specs_policy_guard`) — one coherent snapshot, either order. The
    // guard is taken before the role read and the capability check, so the
    // whole check+write pair stays atomic against a sync in flight.
    let guard = specs_policy_guard(repo_id);
    let _guard = guard.lock().unwrap();
    // Re-read the binding under the guard: the checkout check below must
    // run against the CURRENT local path (T-000243 review, ninth pass).
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    // The ROLE is refused before the capability: a verified administrator of
    // an organization this install participates in as a contributor / pending
    // helper is still not that repo's main here. Read under the guard, so the
    // role cannot be settled mid-check.
    let state = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    if matches!(
        state.shared_role.as_deref(),
        Some("contributor") | Some("pending")
    ) {
        return Err(format!(
            "the specs switch belongs to the repo's main: this install participates in {} as a {}",
            gh,
            state.shared_role.unwrap_or_default()
        ));
    }
    if !ss::can_enable_shared(&gh, &login, &ss::AdminVerified::load(db)) {
        return Err(ss::refusal_not_owner_or_admin(&login, &gh));
    }
    // T-000243 review: switching a repo that already shares ON while user
    // rules still ignore the folder would silently keep the specs local (the
    // shared enumeration honours .gitignore) — the next sync diagnoses it as
    // an issue, and the setter says it right away. A repo without a usable
    // checkout (preconfiguration before the first clone) has nothing to check.
    let note = if on && is_shared_state(&state) {
        crate::git_ops::cached_git()
            .and_then(|git| repo_root(&repo).ok().map(|root| (git, root)))
            .and_then(|(git, root)| specs_blocking_lines(&git, &root).ok())
            .filter(|lines| !lines.is_empty())
            .map(|lines| {
                let rules: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
                format!(
                    "docs/superpowers/ is still ignored by user .gitignore rule(s) [{}]: the \
                     specs will not travel until they are removed in the preflight of the \
                     shared switch",
                    rules.join(", ")
                )
            })
    } else {
        None
    };
    db.set_repo_share_specs(repo_id, on)
        .map_err(|e| e.to_string())?;
    Ok(note)
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
    let state = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    if !state.shared_mode || state.shared_role.as_deref().is_some_and(|r| r != "main") {
        return Err("shared mode can only be switched off by the owner of the repo".into());
    }
    // T-000243 review (second pass): the switch-off DELETES the policy-bearing
    // structure (a commit and a push of the deletion), so it is serialized
    // with everything else that reads or writes it — phase B's policy section
    // (the guard spans its commit + push too), the specs setter, enable and
    // the approval. Without the guard a phase B publishing the structure
    // could interleave with the deletion commit and resurrect `.sdh/`.
    let guard = specs_policy_guard(repo_id);
    let _guard = guard.lock().unwrap();
    // T-000243 review (ninth pass): same rebind rule as enable — the
    // checkout this deletion commits to is the CURRENT binding, read
    // under the guard, and the generation bump keys off it.
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    // T-000243 review (third pass): the operation-validity check is repeated
    // UNDER the guard — a first disable (or the approval) that queued on it
    // before must not be followed by a second run acting on the stale state
    // read above.
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
    // T-000243 review (sixth pass): the lifecycle moved — passes sealed
    // under the pre-disable generation settle no status (this covers the
    // `disabling` marker write below as well: same transition).
    bump_lifecycle_generation(repo_id, &root);

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
    let gh = repo
        .github_name
        .clone()
        .ok_or("the repository has no GitHub name")?;
    let me = db
        .github_login()
        .ok_or_else(|| "the GitHub login of this install is unknown".to_string())?;
    // T-000243 review (second pass): the approval rebuilds and writes the
    // POLICY-BEARING structure, so it takes the same per-repo guard as phase B
    // and the specs setter. Unguarded, an approval built with the old flag
    // could land after a sync's policy section (or after a specs save) and
    // publish / commit a stale policy — or clobber the sync's structure write
    // mid-section. All the request reading, the build and the write sit
    // inside the guard.
    let guard = specs_policy_guard(repo_id);
    let _guard = guard.lock().unwrap();
    // T-000243 review (ninth pass): re-read the binding under the guard —
    // the structure write below must land on the CURRENT checkout.
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    // T-000243 review (third pass): the sharing check is repeated UNDER the
    // guard — a disable queued on it between the read above and here must not
    // be followed by an approval that writes the structure back over the
    // deletion it committed. Everything below reads the state the guard
    // protects.
    let state = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    if !state.shared_mode {
        return Err("only the owner of a shared repo can approve join requests".into());
    }
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
    // T-000243 review (sixth pass): accepting a new main branch clears the
    // pin AND the status — a lifecycle-resolution action. It ran unguarded,
    // so an older pass could settle afterwards and restore the rejection
    // the user just accepted. Serialize the mutation with `finish`'s
    // freshness check and bump the generation. The generation key uses the
    // recorded local path (not `repo_root`): this command is DB-only and
    // must not start failing on a repo without a git checkout. The path is
    // read UNDER the guard so a concurrent repath cannot split the key
    // from the bump (T-000243 review, eighth pass).
    let guard = specs_policy_guard(repo_id);
    let _guard = guard.lock().unwrap();
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let key_root = PathBuf::from(repo.local_path.clone().unwrap_or_default());
    // T-000243 review (seventh pass): the eligibility check above ran
    // BEFORE the guard — a sync may have settled a different status in
    // between. Re-verify under the guard, or this accept would clear a
    // status that is no longer the refused dev-branch one.
    let st = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    let still_changed = st.shared_status.as_deref() == Some(SharedStatus::Untrusted.as_str())
        && st
            .shared_status_detail
            .as_deref()
            .is_some_and(|d| d.starts_with(ss::DEV_BRANCH_CHANGED_PREFIX));
    if !still_changed {
        return Err("the owner's dev branch has not changed: nothing to accept".into());
    }
    let mut v = ss::read_shared_import(db, repo_id);
    v.pinned_dev_branch = None;
    ss::write_shared_import(db, repo_id, &v)?;
    db.set_shared_status(repo_id, None)
        .map_err(|e| e.to_string())?;
    bump_lifecycle_generation(repo_id, &key_root);
    Ok(())
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
    // T-000243 review (tenth pass): the guard spans binding read, merge
    // driver registration, the merge itself AND the status settlement —
    // a repath that landed mid-merge must not let the OLD checkout's
    // result clear the NEW binding's status or bump its generation.
    let guard = specs_policy_guard(repo_id);
    let _guard = guard.lock().unwrap();
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
            // T-000243 review (seventh pass): a successful merge invalidates
            // every pre-merge observation — bump unconditionally, whether or
            // not `needs_manual_merge` was persisted. `root` IS the guarded
            // binding, so the clear and the bump apply to the checkout that
            // was actually merged.
            let st = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
            if st.shared_status.as_deref() == Some(SharedStatus::NeedsManualMerge.as_str()) {
                let _ = db.set_shared_status(repo_id, None);
            }
            bump_lifecycle_generation(repo_id, &root);
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
    use std::process::Command;

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
        // A non-shared repo keeps the template, whatever the policy would be.
        assert_eq!(
            gitignore_template_for(GITIGNORE_TMPL, false, true),
            GITIGNORE_TMPL
        );
        assert_eq!(
            gitignore_template_for(GITIGNORE_TMPL, false, false),
            GITIGNORE_TMPL
        );
        let shared = gitignore_template_for(GITIGNORE_TMPL, true, true);
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
        ] {
            assert!(
                shared.lines().any(|l| l.trim() == kept),
                "{} must stay",
                kept
            );
        }
        // T-000243: opted out of specs, the shared render keeps ignoring the
        // folder (the template line survives the shared drop).
        let off = gitignore_template_for(GITIGNORE_TMPL, true, false);
        assert!(off.lines().any(|l| l.trim() == "docs/superpowers/"));
        assert!(!off.lines().any(|l| l.trim() == "docs/sdh_skills/"));
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
        let tpl = gitignore_template_for(GITIGNORE_TMPL, true, true);
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

    // ----- T-000243: the specs policy in the managed render -------------------

    /// Off must guarantee the ignore even when the custom (or empty) template
    /// lacks the line; on with an empty template must not leave a stale managed
    /// ignore rule behind. The user's own block is preserved throughout.
    #[test]
    fn off_render_ensures_the_specs_ignore_with_custom_and_empty_templates() {
        // A custom template without the line: the off render carries it anyway.
        let custom = gitignore_template_for("target\nnode_modules\n", true, false);
        assert!(custom.lines().any(|l| l.trim() == "docs/superpowers/"));
        // An empty template: the managed render is exactly the ensured rule.
        assert_eq!(gitignore_template_for("", true, false), "docs/superpowers/");
        // On with an empty template renders nothing at all.
        assert_eq!(gitignore_template_for("", true, true), "");
        // A non-shared repo is untouched, whatever the policy.
        assert_eq!(gitignore_template_for("", false, false), "");

        // The whole on/off/on cycle over a user .gitignore: the managed state
        // follows the flag every time and the user's lines survive.
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join(".gitignore"), "# mine\n*.log\n").unwrap();
        let on = gitignore_template_for(GITIGNORE_TMPL, true, true);
        let off = gitignore_template_for(GITIGNORE_TMPL, true, false);
        assert!(crate::sync::sync_gitignore_section(&on, tmp.path()).unwrap());
        let text_on = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(!text_on.contains("docs/superpowers/"));
        assert!(text_on.contains("# mine") && text_on.contains("*.log"));

        assert!(crate::sync::sync_gitignore_section(&off, tmp.path()).unwrap());
        let text_off = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert_eq!(
            text_off.matches("docs/superpowers/").count(),
            1,
            "off keeps exactly one ignore of the folder:\n{}",
            text_off
        );
        assert!(text_off.contains("# mine") && text_off.contains("*.log"));
        // Re-rendering the same policy is idempotent.
        assert!(!crate::sync::sync_gitignore_section(&off, tmp.path()).unwrap());

        assert!(crate::sync::sync_gitignore_section(&on, tmp.path()).unwrap());
        let back_on = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(!back_on.contains("docs/superpowers/"));
        assert!(back_on.contains("# mine") && back_on.contains("*.log"));
    }

    /// The empty-template corner of the same contract: a managed block written
    /// by an off render is removed when the policy turns on and the custom
    /// template renders nothing — off → on must not keep the stale rule.
    #[test]
    fn an_empty_template_does_not_keep_a_stale_managed_specs_rule() {
        let tmp = tempfile::TempDir::new().unwrap();
        // off: the ensured rule lands in the managed block
        assert!(crate::sync::sync_gitignore_section(
            &gitignore_template_for("", true, false),
            tmp.path()
        )
        .unwrap());
        assert_eq!(
            fs::read_to_string(tmp.path().join(".gitignore")).unwrap(),
            "# --- solo-dev-hub:begin ---\ndocs/superpowers/\n# --- solo-dev-hub:end ---\n"
        );
        // on with the same empty template: the block is gone, not kept
        assert!(crate::sync::sync_gitignore_section(
            &gitignore_template_for("", true, true),
            tmp.path()
        )
        .unwrap());
        assert_eq!(
            fs::read_to_string(tmp.path().join(".gitignore")).unwrap(),
            ""
        );
        // and an absent file stays absent
        let fresh = tempfile::TempDir::new().unwrap();
        assert!(!crate::sync::sync_gitignore_section(
            &gitignore_template_for("", true, true),
            fresh.path()
        )
        .unwrap());
        assert!(!fresh.path().join(".gitignore").exists());
    }

    /// The setter's authority: the owner and a verified administrator may flip
    /// the flag; a contributor, a pending helper and an unknown login are
    /// refused — the same trust rule as enabling sharing. T-000243 review: the
    /// ROLE is refused before the CAPABILITY, so even a verified administrator
    /// of the organization cannot flip the policy of a repo this install
    /// participates in as a contributor / pending helper.
    #[test]
    fn the_specs_switch_is_stored_for_the_owner_or_a_verified_admin_only() {
        // The owner of the repo.
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.set_setting("github_login", "Owner").unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        set_repo_share_specs(&db, repo.id, false).unwrap();
        assert!(!db.get_shared_state(repo.id).unwrap().share_specs);
        set_repo_share_specs(&db, repo.id, true).unwrap();
        assert!(db.get_shared_state(repo.id).unwrap().share_specs);

        // A verified administrator of an organization's repo may too.
        let (org_db, org_repo) = org_db("boss");
        let err = set_repo_share_specs(&org_db, org_repo.id, false).unwrap_err();
        assert!(err.contains("repo owner or an administrator"), "{}", err);
        ss::record_admin_verification(&org_db, "Acme-Org/web", "boss", true).unwrap();
        set_repo_share_specs(&org_db, org_repo.id, false).unwrap();
        assert!(!org_db.get_shared_state(org_repo.id).unwrap().share_specs);

        // T-000243 review: the same verified administrator is refused while
        // this install's ROLE in that repo is contributor or pending — the
        // capability alone is not the contract, and the role is checked first.
        // (The stored value is false by now, so a refused `true` proves that
        // nothing was written.)
        for role in ["contributor", "pending"] {
            org_db.set_shared_role(org_repo.id, Some(role)).unwrap();
            let err = set_repo_share_specs(&org_db, org_repo.id, true).unwrap_err();
            assert!(err.contains("participates in Acme-Org/web as a"), "{}", err);
            assert!(!org_db.get_shared_state(org_repo.id).unwrap().share_specs);
        }
        // Back on the owner's side of the role, the same admin may set it.
        org_db.set_shared_role(org_repo.id, Some("main")).unwrap();
        set_repo_share_specs(&org_db, org_repo.id, false).unwrap();
        assert!(!org_db.get_shared_state(org_repo.id).unwrap().share_specs);

        // A contributor (a helper on somebody else's repo) and a pending
        // helper are refused; nothing is stored either way.
        let helper = AppDb::new(PathBuf::from(":memory:")).unwrap();
        helper.set_setting("github_login", "helper").unwrap();
        helper
            .upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let hrepo = helper.get_repository_by_name("Owner/api").unwrap();
        helper
            .set_shared_role(hrepo.id, Some("contributor"))
            .unwrap();
        assert!(set_repo_share_specs(&helper, hrepo.id, false).is_err());
        helper.set_shared_role(hrepo.id, Some("pending")).unwrap();
        assert!(set_repo_share_specs(&helper, hrepo.id, true).is_err());
        assert!(helper.get_shared_state(hrepo.id).unwrap().share_specs);

        // An unknown login is refused before anything is checked.
        let no_login = AppDb::new(PathBuf::from(":memory:")).unwrap();
        no_login
            .upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let nrepo = no_login.get_repository_by_name("Owner/api").unwrap();
        let err = set_repo_share_specs(&no_login, nrepo.id, false).unwrap_err();
        assert!(err.contains("unknown"), "{}", err);
        // A repo without a GitHub name has no specs policy at all.
        no_login.set_setting("github_login", "Owner").unwrap();
        no_login
            .upsert_repository("local-only", None, None, None, None, None)
            .unwrap();
        let local = no_login.get_repository_by_name("local-only").unwrap();
        assert!(set_repo_share_specs(&no_login, local.id, false).is_err());
    }

    /// `effective_share_specs`: the owner's own stored setting wins on the
    /// owner's install; a helper's install takes the owner's published flag
    /// for THIS repo from the validated structure, defaulting to on. A
    /// structure that IS present but unusable is an error (see the test below),
    /// never a silent `on`.
    #[test]
    fn effective_specs_follow_the_owner_db_on_main_and_the_structure_on_helpers() {
        // Owner: its own DB row, even when the (stale) structure says else.
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.set_setting("github_login", "Owner").unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        db.set_shared_mode(repo.id, true).unwrap();
        db.set_shared_role(repo.id, Some("main")).unwrap();
        let mut st = sample_structure();
        st.repos[0].share_specs = Some(true);
        assert!(effective_share_specs(&db, &repo, Some(&st)).unwrap());
        db.set_repo_share_specs(repo.id, false).unwrap();
        assert!(!effective_share_specs(&db, &repo, Some(&st)).unwrap());

        // Helper: the flag comes from the structure, per repo; a repo the
        // structure does not list means the default on.
        let helper = AppDb::new(PathBuf::from(":memory:")).unwrap();
        helper.set_setting("github_login", "helper").unwrap();
        helper
            .upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let hrepo = helper.get_repository_by_name("Owner/api").unwrap();
        helper
            .set_shared_role(hrepo.id, Some("contributor"))
            .unwrap();
        // The helper's own DB is irrelevant — even an explicit local off.
        helper.set_repo_share_specs(hrepo.id, false).unwrap();
        let mut on_struct = sample_structure();
        on_struct.repos[0].share_specs = Some(true);
        assert!(effective_share_specs(&helper, &hrepo, Some(&on_struct)).unwrap());
        let mut off_struct = sample_structure();
        off_struct.repos[0].share_specs = Some(false);
        assert!(!effective_share_specs(&helper, &hrepo, Some(&off_struct)).unwrap());
        let mut legacy = sample_structure();
        legacy.repos[0].share_specs = None; // legacy structure: default on
        assert!(effective_share_specs(&helper, &hrepo, Some(&legacy)).unwrap());
        // No structure at all: the default on.
        assert!(effective_share_specs(&helper, &hrepo, None).unwrap());
    }

    /// T-000243 review: a work-tree structure that IS present but cannot be
    /// used (malformed JSON, a wrong field type) is an error — the callers
    /// block the render / commit on it instead of guessing `on`, which would
    /// publish specs nobody opted in to. An absent file keeps the default on.
    #[test]
    fn an_unreadable_worktree_structure_is_an_error_not_a_silent_on() {
        let helper = AppDb::new(PathBuf::from(":memory:")).unwrap();
        helper.set_setting("github_login", "helper").unwrap();
        helper
            .upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let hrepo = helper.get_repository_by_name("Owner/api").unwrap();
        helper
            .set_shared_role(hrepo.id, Some("contributor"))
            .unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        helper
            .set_repo_local_path(hrepo.id, Some(dir.path().to_str().unwrap()))
            .unwrap();
        // The row is re-read: the policy resolver looks at the repo's own
        // `local_path`, which was empty when the row above was fetched.
        let hrepo = helper.get_repository_by_name("Owner/api").unwrap();
        // No file at all: the legacy default on.
        assert!(effective_share_specs(&helper, &hrepo, None).unwrap());
        // Malformed JSON in the trusted place: an error.
        write(dir.path(), ".sdh/project.json", "{ not json");
        let err = effective_share_specs(&helper, &hrepo, None).unwrap_err();
        assert!(err.contains("invalid JSON"), "{}", err);
        // A wrong field type is the same refusal, not a fallback (the spec:
        // «неверный тип — ошибка, не fallback on»).
        write(
            dir.path(),
            ".sdh/project.json",
            "{\"format\":1,\"project\":{\"name\":\"Acme\",\"type\":\"standard\"},\
             \"main\":{\"login\":\"Owner\",\"dev_branch\":\"dev\"},\
             \"repos\":[{\"github\":\"Owner/api\",\"role\":\"server\",\
             \"share_specs\":\"yes\"}],\"microservices\":[],\"participants\":[]}",
        );
        let err = effective_share_specs(&helper, &hrepo, None).unwrap_err();
        assert!(err.contains("invalid JSON"), "{}", err);
        // T-000243 review (second pass): an explicit `null` is a wrong type,
        // not "absent" — the default-on is granted only to a MISSING field,
        // so this must be the same refusal, never a silent on.
        write(
            dir.path(),
            ".sdh/project.json",
            "{\"format\":1,\"project\":{\"name\":\"Acme\",\"type\":\"standard\"},\
             \"main\":{\"login\":\"Owner\",\"dev_branch\":\"dev\"},\
             \"repos\":[{\"github\":\"Owner/api\",\"role\":\"server\",\
             \"share_specs\":null}],\"microservices\":[],\"participants\":[]}",
        );
        let err = effective_share_specs(&helper, &hrepo, None).unwrap_err();
        assert!(err.contains("share_specs"), "{}", err);
        assert!(err.contains("null"), "{}", err);
        // And an untrusted source (a main this install cannot verify) is not a
        // policy either: still an error, never `on`.
        write(
            dir.path(),
            ".sdh/project.json",
            "{\"format\":1,\"project\":{\"name\":\"Acme\",\"type\":\"standard\"},\
             \"main\":{\"login\":\"stranger\",\"dev_branch\":\"dev\"},\
             \"repos\":[{\"github\":\"Owner/api\",\"role\":\"server\",\"share_specs\":true}],\
             \"microservices\":[],\"participants\":[]}",
        );
        let err = effective_share_specs(&helper, &hrepo, None).unwrap_err();
        assert!(err.contains("stranger"), "{}", err);
    }

    // ----- T-000243 review: the setter cannot interleave with phase B ---------

    /// The per-repo policy guard really excludes: while a sync's policy section
    /// holds it, `set_repo_share_specs` (a Tauri command on another thread) is
    /// not answered, and it completes as soon as the section is over. This is
    /// the lock the two interleavings below rely on.
    #[test]
    fn the_specs_setter_waits_for_the_syncs_policy_section() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.set_setting("github_login", "Owner").unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        db.set_shared_mode(repo.id, true).unwrap();
        let guard = specs_policy_guard(repo.id);
        let held = guard.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|s| {
            s.spawn(|| {
                let _ = tx.send(set_repo_share_specs(&db, repo.id, false));
            });
            assert!(
                rx.recv_timeout(Duration::from_millis(500)).is_err(),
                "the setter must not be acknowledged while the policy section runs"
            );
            drop(held);
            assert!(rx.recv_timeout(Duration::from_secs(30)).is_ok());
        });
        assert!(!db.get_shared_state(repo.id).unwrap().share_specs);
    }

    /// Interleaving (a) — the setter arrives while phase B is already inside
    /// its policy section: the guard forces one of the two orders, so the pass
    /// that resolved `on` commits the WHOLE policy coherently (the spec travels,
    /// the structure publishes on) and only then is the off acknowledged. No
    /// half-applied state is ever observable.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_specs_toggle_mid_phase_b_commits_the_old_policy_then_persists_off() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# x\n");

        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<i64>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<i64>();
        install_phase_b_policy_probe(fx.main_repo, &fx.main_dir, entered_tx, release_rx);

        let out = std::thread::scope(|s| {
            // Phase B of the sync runs here; inside its policy section it hands
            // the repo id over and waits for the release.
            let sync = s.spawn(|| fx.sync_main());
            let in_section = entered_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("phase B never entered its policy section (the probe did not fire)");
            assert_eq!(in_section, fx.main_repo);
            // The OFF setter starts while phase B is inside the section: the
            // guard is held, so it cannot be acknowledged until the section
            // (structure publish, pathspec commits) is over.
            let setter = s.spawn(|| set_repo_share_specs(&fx.main_db, fx.main_repo, false));
            release_tx.send(1).unwrap();
            let out = sync.join().unwrap();
            assert!(
                setter.join().unwrap().is_ok(),
                "the setter must finish right after the policy section"
            );
            out
        });
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        // The whole ON policy of that pass landed ...
        assert!(fx
            .remote_file("dev", "docs/superpowers/specs/x.md")
            .is_some());
        assert!(fx
            .remote_file("dev", ".sdh/project.json")
            .unwrap()
            .contains("\"share_specs\": true"));
        // ... and the off is persisted after it: one coherent order, no
        // half-on pass and no half-off commit.
        assert!(
            !fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .share_specs
        );
        // The next pass follows the new flag alone.
        fx.sync_main();
        write(&fx.main_dir, "docs/superpowers/specs/after.md", "# after\n");
        fx.sync_main();
        assert_eq!(
            fx.remote_file("dev", "docs/superpowers/specs/after.md"),
            None,
            "after the off, a new spec must not travel"
        );
        assert!(fx
            .remote_file("dev", "docs/superpowers/specs/x.md")
            .is_some());
    }

    /// Interleaving (b) — the setter completes first: every policy-sensitive
    /// step of the sync that follows sees the new flag, so a spec that exists
    /// already stays local and nothing is published with the old `on`.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_specs_toggle_before_phase_b_is_seen_by_the_whole_pass() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# x\n");
        // No warning note: this repo is already shared, but its `.gitignore`
        // carries no user rule that hides the folder.
        assert!(set_repo_share_specs(&fx.main_db, fx.main_repo, false)
            .unwrap()
            .is_none());
        let out = fx.sync_main();
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert_eq!(fx.remote_file("dev", "docs/superpowers/specs/x.md"), None);
        // The published structure carries the off the helper will accept.
        assert!(fx
            .remote_file("dev", ".sdh/project.json")
            .unwrap()
            .contains("\"share_specs\": false"));
    }

    /// T-000243 review (second pass): `approve_join_request` and
    /// `enable_shared_mode` also write the policy-bearing structure, so they
    /// take the same per-repo guard as phase B and the specs setter. The
    /// deterministic order (the probe parks phase B inside its policy
    /// section): phase B (still ON, without the new participant) → the
    /// approval (adds `helper2`) → the OFF setter → one more sync. Whatever
    /// order the queued writers wake in, nothing COMMITTED after the OFF save
    /// says on again, and the approval is never clobbered by phase B's stale
    /// structure snapshot. Without the guard the approval ran to completion
    /// DURING the parked section and phase B's rebuild (from the phase-A
    /// snapshot) silently dropped the new participant — the blocking
    /// assertions below fail without it.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn an_approval_and_a_specs_toggle_never_publish_a_stale_on_structure() {
        let fx = two_installs();
        fx.make_participant();
        // A second helper asks to join; the request waits in the owner's tree.
        write(
            &fx.main_dir,
            ".sdh/requests/helper2.json",
            "{\"login\":\"helper2\",\"branch\":\"helper2-dev\"}\n",
        );

        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<i64>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<i64>();
        install_phase_b_policy_probe(fx.main_repo, &fx.main_dir, entered_tx, release_rx);
        let (ap_tx, ap_rx) = std::sync::mpsc::channel();
        let (en_tx, en_rx) = std::sync::mpsc::channel();

        std::thread::scope(|s| {
            let sync = s.spawn(|| fx.sync_main());
            let in_section = entered_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("phase B never entered its policy section (the probe did not fire)");
            assert_eq!(in_section, fx.main_repo);
            // Both structure writers queue on the guard phase B holds.
            s.spawn(|| {
                let _ = ap_tx.send(approve_join_request(
                    &fx.main_db,
                    Some(&gb()),
                    fx.main_repo,
                    "helper2",
                ));
            });
            s.spawn(|| {
                let _ = en_tx.send(enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false));
            });
            assert!(
                ap_rx.recv_timeout(Duration::from_millis(500)).is_err(),
                "the approval must wait for the per-repo policy guard"
            );
            assert!(
                en_rx.recv_timeout(Duration::from_millis(500)).is_err(),
                "enable must wait for the per-repo policy guard"
            );
            release_tx.send(1).unwrap();
            let out = sync.join().unwrap();
            assert!(out.issues.is_empty(), "{:?}", out.issues);
            assert!(
                ap_rx.recv_timeout(Duration::from_secs(60)).unwrap().is_ok(),
                "the approval must finish right after the policy section"
            );
            assert!(
                en_rx.recv_timeout(Duration::from_secs(60)).unwrap().is_ok(),
                "enable must finish right after the policy section"
            );
        });
        // The OFF save happens after the guarded writers: whatever they wrote,
        // the next pass publishes the new policy coherently — a stale ON
        // structure is never committed after the save.
        assert!(set_repo_share_specs(&fx.main_db, fx.main_repo, false)
            .unwrap()
            .is_none());
        let out = fx.sync_main();
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        let published = fx.remote_file("dev", ".sdh/project.json").unwrap();
        assert!(
            published.contains("\"share_specs\": false"),
            "the structure committed after the OFF save must be off:\n{}",
            published
        );
        assert!(
            published.contains("helper2"),
            "the approval must survive the interleaving:\n{}",
            published
        );
    }

    /// T-000243 review (third pass), ordering (a): phase A captures the pass;
    /// the approval — a writer of the policy-bearing structure — completes
    /// BETWEEN the phases (it takes the same per-repo guard, but only for its
    /// own run; phase B is driven directly here, `run_phase_a` then `finish`).
    /// Phase B used to rebuild the structure from the phase-A snapshot and
    /// re-publish it, silently dropping the participant the approval added;
    /// under the revalidation it re-reads the work-tree file the approval
    /// wrote and publishes the approval instead.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn an_approval_between_phase_a_and_phase_b_survives_phase_b() {
        let fx = two_installs();
        fx.make_participant();
        // A second helper asks to join; the request waits in the owner's tree.
        write(
            &fx.main_dir,
            ".sdh/requests/helper2.json",
            "{\"login\":\"helper2\",\"branch\":\"helper2-dev\"}\n",
        );

        // Phase A runs to completion and captures the OLD participant list.
        let shared = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        // The approval lands between the phases.
        approve_join_request(&fx.main_db, Some(&gb()), fx.main_repo, "helper2").unwrap();
        let approved =
            fs::read_to_string(fx.main_dir.join(".sdh/project.json")).unwrap_or_default();
        assert!(approved.contains("helper2"), "{}", approved);

        // The deferred phase B publishes the approval, never clobbers it.
        let out = shared.finish(&fx.main_db, Some(&gb()));
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        let published = fx.remote_file("dev", ".sdh/project.json").unwrap();
        assert!(
            published.contains("helper2"),
            "the approval must survive phase B:\n{}",
            published
        );
        // and the request file of the approved participant stays gone
        assert!(!fx.main_dir.join(".sdh/requests/helper2.json").exists());
    }

    /// T-000243 review (third pass), ordering (b): `disable_shared_mode`
    /// completes between the phases — the deletion of `.sdh/` is committed
    /// and pushed and the database flipped. The deferred phase-B pass of the
    /// run that captured the repo while sharing was on must NOT re-publish
    /// `.sdh/project.json` (the stale Main pass used to rebuild and commit
    /// the structure right over the deletion) — the revalidation cancels its
    /// whole policy section.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_disable_between_phase_a_and_phase_b_cancels_the_deferred_pass() {
        let fx = two_installs();
        fx.make_participant();

        let shared = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        // Sharing is switched off between the phases.
        disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(!fx.main_dir.join(".sdh/project.json").exists());
        assert_eq!(fx.remote_file("dev", ".sdh/project.json"), None);

        // The deferred phase B of the pass captured while sharing was on.
        let out = shared.finish(&fx.main_db, Some(&gb()));
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert!(
            !fx.main_dir.join(".sdh/project.json").exists(),
            "phase B must not resurrect the structure after the switch-off"
        );
        assert_eq!(
            fx.remote_file("dev", ".sdh/project.json"),
            None,
            "the deletion must stand on origin/dev"
        );
        assert!(
            !fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .shared_mode
        );
        // and the repo settled no status from the cancelled pass
        assert_eq!(fx.main_status(), None);
    }

    /// T-000243 review (fourth pass): two SharedSyncs of the SAME helper
    /// install race. Pass A captures the ON structure in phase A; the owner
    /// then publishes OFF and pass B accepts it whole; A resumes — its
    /// phase-B revalidation must discover the NEWER authoritative structure
    /// instead of committing with the stale ON snapshot, and the render must
    /// resolve the same fresh policy.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_stale_helper_pass_follows_the_newer_authoritative_policy() {
        let fx = two_installs();
        fx.make_participant();
        let git = gb();
        // A spec the helper could publish if its stale ON snapshot were used.
        write(&fx.helper_dir, "docs/superpowers/specs/x.md", "# x\n");
        // Pass A: phase A only — the ON structure is captured.
        let project = fx
            .helper_db
            .list_all_repos()
            .unwrap()
            .into_iter()
            .find_map(|r| r.project_id);
        let shared_a = run_phase_a(&fx.helper_db, Some(&git), project);
        // The owner turns specs OFF and publishes it; pass B accepts it.
        fx.main_db
            .set_repo_share_specs(fx.main_repo, false)
            .unwrap();
        fx.sync_main();
        fx.sync_helper();
        // And THIS sync's render — the one holding the stale ON snapshot —
        // resolves the same fresh policy: a helper's answer is always
        // authoritative discovery, never the phase-A structure.
        let repo = fx.helper_db.get_repository(fx.helper_repo).unwrap();
        let state = shared_a
            .render_state_of(&fx.helper_db, Some(&git), &repo)
            .unwrap();
        assert!(
            !effective_share_specs(&fx.helper_db, &repo, state.as_ref()).unwrap(),
            "the render must resolve the newer OFF, not the phase-A ON"
        );
        // Pass A resumes: phase B must see OFF — no specs commit of its own.
        let out_a = shared_a.finish(&fx.helper_db, Some(&git));
        assert!(out_a.issues.is_empty(), "{:?}", out_a.issues);
        assert_eq!(
            fx.remote_file("helper-dev", "docs/superpowers/specs/x.md"),
            None,
            "the stale ON pass must not publish the helper's spec"
        );
        assert_eq!(fx.remote_file("dev", "docs/superpowers/specs/x.md"), None);
    }

    /// T-000243 review (fourth pass): without a git binary a HELPER's specs
    /// policy cannot be discovered — `render_structure_opt` refuses instead
    /// of letting a possibly stale work-tree copy answer. The owner's own
    /// policy needs no discovery and still resolves.
    #[test]
    fn render_structure_opt_refuses_a_helper_without_git() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.set_setting("github_login", "helper").unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        db.set_shared_role(repo.id, Some("contributor")).unwrap();
        let err = render_structure_opt(&db, None, &repo).unwrap_err();
        assert!(err.contains("git is unavailable"), "{}", err);
        // The owner side answers from its own row — no git needed.
        db.set_shared_role(repo.id, Some("main")).unwrap();
        db.set_shared_mode(repo.id, true).unwrap();
        assert!(render_structure_opt(&db, None, &repo).unwrap().is_none());
    }

    /// T-000243 review (fourth pass): a pass phase A REFUSED never entered
    /// phase B (`role` is `None`), so `finish` used to settle its stale
    /// untrusted status even after a disable that completed between the
    /// phases had already owned the repo's state. The settlement now happens
    /// under the same guard and only while the lifecycle the pass was sealed
    /// under is still current — the disable moved it, so the pass is stale.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_disable_between_the_phases_keeps_the_disabled_status() {
        let fx = two_installs();
        fx.make_participant();
        // The owner's structure turns unusable: phase A refuses the pass.
        write(&fx.main_dir, ".sdh/project.json", "{ not json");
        let shared = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        // Sharing is switched off between the phases.
        disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(
            !fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .shared_mode
        );
        let _out = shared.finish(&fx.main_db, Some(&gb()));
        assert_eq!(
            fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .shared_status,
            None,
            "the disabled repo's status belongs to the disable, not the stale pass"
        );
    }

    /// T-000243 review (sixth pass): a FAILED disable followed by a
    /// re-enable is a lifecycle ROUND TRIP — the state fields end where
    /// they started, which a state snapshot cannot see. The generation
    /// can: the pass sealed before the disable must not settle over the
    /// `push_failed` the disable left behind.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_disable_reenable_round_trip_still_makes_the_pass_stale() {
        let fx = two_installs();
        fx.enable_main();
        let shared = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        // The disable cannot push: it flips the state AND leaves push_failed.
        let hook = block_pushes_to(&fx, "refs/heads/dev");
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(!res.pushed, "{:?}", res);
        assert_eq!(fx.main_status().as_deref(), Some("push_failed"));
        // Sharing is switched back on before the stale pass finishes: mode
        // and role return to exactly what the pass was created under.
        enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).unwrap();
        fs::remove_file(&hook).unwrap();
        let _out = shared.finish(&fx.main_db, Some(&gb()));
        assert_eq!(
            fx.main_status().as_deref(),
            Some("push_failed"),
            "the round-tripped lifecycle must still count as moved"
        );
    }

    /// T-000243 review (sixth pass): a successful disable retry clears the
    /// `disabling` marker and the status; an OLDER retry pass that failed
    /// before the clear must not settle its obsolete `push_failed` back.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_successful_retry_supersedes_an_older_retry_failure() {
        let fx = two_installs();
        fx.enable_main();
        let hook = block_pushes_to(&fx, "refs/heads/dev");
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(!res.pushed, "{:?}", res);
        // Pass A: phase A only — its retry of the pending deletion fails
        // again while the hook still blocks pushes.
        let shared_a = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        // A second sync's retry succeeds before pass A finishes.
        fs::remove_file(&hook).unwrap();
        fx.sync_main();
        assert_eq!(fx.main_status(), None);
        // Pass A resumes: its PushFailed note is obsolete — the deletion
        // already reached origin and the marker is gone.
        let _out = shared_a.finish(&fx.main_db, Some(&gb()));
        assert_eq!(
            fx.main_status(),
            None,
            "the superseded retry failure must not be restored"
        );
    }

    /// T-000243 review (eleventh pass): the retry's fetch is its last
    /// read-only step — a re-enable landing while it runs must stop the
    /// retry BEFORE the first mutation (no branch refresh, no deletion
    /// commit, no push), not merely before the marker cleanup. The probe
    /// parks the retry in exactly that window.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_reenable_during_the_retry_fetch_stops_it_before_any_mutation() {
        let fx = two_installs();
        fx.enable_main();
        // The disable cannot push: the `.sdh/` deletion stays pending.
        let hook = block_pushes_to(&fx, "refs/heads/dev");
        let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(!res.pushed, "{:?}", res);
        assert!(ss::read_shared_import(&fx.main_db, fx.main_repo)
            .disabling
            .is_some());
        let head_before = g(&fx.main_dir, &["rev-parse", "HEAD"]);

        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<i64>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<i64>();
        install_retry_post_fetch_probe(fx.main_repo, &fx.main_dir, entered_tx, release_rx);

        let out = std::thread::scope(|s| {
            // Phase A reaches the pending-disable retry; inside its
            // post-fetch window it hands the repo id over and waits.
            let sync = s.spawn(|| run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id)));
            let in_window = entered_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("the retry never reached its post-fetch window");
            assert_eq!(in_window, fx.main_repo);
            // The re-enable lands while the retry is between fetch and the
            // guard: it bumps the lifecycle generation and clears the
            // pending-disable marker.
            enable_shared_mode(&fx.main_db, &gb(), fx.main_repo, false).unwrap();
            release_tx.send(1).unwrap();
            sync.join().unwrap()
        });
        fs::remove_file(&hook).unwrap();
        assert!(
            out.passes.is_empty(),
            "the obsolete retry must not even produce a pass, got {:?}",
            out.passes.len()
        );

        // The stale retry mutated NOTHING: no local deletion commit, the
        // structure is still tracked locally and still on origin.
        assert_eq!(
            g(&fx.main_dir, &["rev-parse", "HEAD"]),
            head_before,
            "the obsolete retry must not commit the `.sdh/` deletion"
        );
        assert!(fx.main_dir.join(".sdh/project.json").is_file());
        assert!(fx.remote_file("dev", ".sdh/project.json").is_some());
        // And it settled no status of its own: the repo is shared again.
        assert!(
            fx.main_db
                .get_shared_state(fx.main_repo)
                .unwrap()
                .shared_mode
        );
    }

    /// T-000243 review (twelfth pass): phase A's branch section may
    /// `reset_to_origin` a non-current branch — rolling it back over a
    /// disable's unpushed deletion commit if the disable lands between the
    /// pass's fetch and its ref updates. The section now runs under the
    /// policy guard gated on the pass's generation: a lifecycle that moved
    /// meanwhile makes every mutation obsolete BEFORE the first ref update.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_phase_a_reset_cannot_roll_back_an_in_flight_disable() {
        let fx = two_installs();
        fx.enable_main();
        // A feature branch is checked out: `dev` stays a non-current ref —
        // exactly the branch the phase-A reset path may move.
        g(&fx.main_dir, &["checkout", "-q", "-b", "feature"]);
        // The disable's push is refused, so its `.sdh/` deletion on `dev`
        // stays pending behind the marker.
        let hook = block_pushes_to(&fx, "refs/heads/dev");

        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<i64>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<i64>();
        install_after_fetch_probe(fx.main_repo, &fx.main_dir, entered_tx, release_rx);

        let mut deletion = String::new();
        std::thread::scope(|s| {
            // Phase A runs to `after_fetch` and parks before its guarded
            // branch section. `_release` frees the worker even when an
            // assertion below panics — `thread::scope` would otherwise
            // wait forever on a still-parked worker (T-000243 review,
            // fifteenth pass).
            let _release = ReleaseOnDrop(release_tx);
            let sync = s.spawn(|| run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id)));
            let in_window = entered_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("phase A never reached its branch section");
            assert_eq!(in_window, fx.main_repo);
            // The disable lands inside the window: deletion commit D on
            // `dev`, sharing off, generation bumped, push still blocked.
            let res = disable_shared_mode(&fx.main_db, &gb(), fx.main_repo).unwrap();
            assert!(!res.pushed, "{:?}", res);
            deletion = g(&fx.main_dir, &["rev-parse", "dev"]);
            assert!(ss::read_shared_import(&fx.main_db, fx.main_repo)
                .disabling
                .is_some());
            // A participant's commit C — a child of the pre-disable tip,
            // still carrying `.sdh/` — lands on origin/dev, and the local
            // remote-tracking ref sees it.
            fs::remove_file(&hook).unwrap();
            let p = g(&fx.main_dir, &["rev-parse", "origin/dev"]);
            let tree = g(&fx.main_dir, &["rev-parse", &format!("{}^{{tree}}", p)]);
            let c = g(
                &fx.main_dir,
                &["commit-tree", &tree, "-p", &p, "-m", "participant work"],
            );
            g(
                &fx.main_dir,
                &["push", "-q", "origin", &format!("{}:dev", c)],
            );
            g(&fx.main_dir, &["fetch", "-q", "origin"]);
            drop(_release);
            sync.join().unwrap();
        });

        // The stale pass must not have reset `dev` over the disable's
        // deletion: the branch still points at D, the marker still waits
        // for the retry, and no backup ref was created by a reset.
        assert_eq!(
            g(&fx.main_dir, &["rev-parse", "dev"]),
            deletion,
            "the obsolete pass must not roll dev back over the disable's commit"
        );
        assert!(ss::read_shared_import(&fx.main_db, fx.main_repo)
            .disabling
            .is_some());
        let backup = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", "refs/sdh-backup/dev"])
            .current_dir(&fx.main_dir)
            .output()
            .unwrap();
        assert!(
            !backup.status.success(),
            "reset_to_origin must not have run for the stale pass"
        );
    }

    /// T-000243 review (thirteenth pass): the generation gate covers PHASE
    /// B too — a pass captured before a repath must not commit or push
    /// from the checkout it no longer owns. Before the fix the deferred
    /// half still ran (only the status settlement was suppressed), so the
    /// old checkout's files, refs and remote all stayed untouched is the
    /// assertion that matters.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_repathed_repo_makes_its_phase_a_pass_skip_phase_b() {
        let fx = two_installs();
        fx.enable_main();
        // A spec waits for phase B to carry it to origin.
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# x\n");
        let shared = run_phase_a(&fx.main_db, Some(&gb()), Some(fx.project_id));
        let dev_before = g(&fx.main_dir, &["rev-parse", "dev"]);
        // The checkout is rebound while the pass waits for phase B.
        rebind_local_path(
            &fx.main_db,
            fx.main_repo,
            Some(fx.helper_dir.to_str().unwrap()),
        )
        .unwrap();
        let _out = shared.finish(&fx.main_db, Some(&gb()));
        assert_eq!(
            g(&fx.main_dir, &["rev-parse", "dev"]),
            dev_before,
            "the stale pass must not commit from the old checkout"
        );
        assert_eq!(
            fx.remote_file("dev", "docs/superpowers/specs/x.md"),
            None,
            "the stale pass must not push from the old checkout"
        );
    }

    /// T-000243 review (fourteenth pass): `after_fetch` released the policy
    /// guard after the branch section and THEN ran `import_incoming` and the
    /// canon adoption — a repath landing in that window let an obsolete pass
    /// import the old checkout's rows and reconcile them into the NEW
    /// binding's `docs/bug-reports.md`. The whole side-effecting tail now
    /// sits inside the same guarded, freshness-gated hold.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_repath_during_after_fetch_stops_the_import_from_the_old_checkout() {
        let fx = two_installs();
        fx.make_participant();
        fx.main_db
            .set_bugs_migrated_at(fx.main_repo, "2026-10-01T00:00:00Z")
            .unwrap();
        fx.sync_main();
        let imp_before = ss::read_shared_import(&fx.main_db, fx.main_repo);
        let helper_bugs_before =
            fs::read_to_string(fx.helper_dir.join("docs/bug-reports.md")).unwrap_or_default();

        // A bug row the pass's checkout holds in HEAD but the import has not
        // seen yet — exactly what `import_incoming` would pick up.
        write(
            &fx.main_dir,
            "docs/bug-reports.md",
            &bugs_md(&[note(
                "B-500001",
                "committed in the stale checkout",
                "created",
                0,
            )]),
        );
        g(&fx.main_dir, &["add", "docs/bug-reports.md"]);
        g(&fx.main_dir, &["commit", "-q", "-m", "bug row"]);

        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<i64>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<i64>();
        install_after_fetch_probe(fx.main_repo, &fx.main_dir, entered_tx, release_rx);
        let project = fx
            .main_db
            .list_all_repos()
            .unwrap()
            .into_iter()
            .find_map(|r| r.project_id);
        std::thread::scope(|s| {
            let _release = ReleaseOnDrop(release_tx);
            let sync = s.spawn(|| run_phase_a(&fx.main_db, Some(&gb()), project));
            let in_window = entered_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("phase A never reached after_fetch");
            assert_eq!(in_window, fx.main_repo);
            // The checkout is rebound while the pass waits before its
            // guarded section: the generation moves under both spellings.
            rebind_local_path(
                &fx.main_db,
                fx.main_repo,
                Some(fx.helper_dir.to_str().unwrap()),
            )
            .unwrap();
            drop(_release);
            sync.join().unwrap();
        });

        // The stale pass imported NOTHING: no row in the database, no blob
        // bookkeeping, and the new binding's bug file was not regenerated
        // with the old checkout's rows.
        assert!(
            fx.main_db
                .get_bug_by_display_id(fx.main_repo, "B-500001")
                .unwrap()
                .is_none(),
            "the obsolete pass must not import the old checkout's bug row"
        );
        let imp_after = ss::read_shared_import(&fx.main_db, fx.main_repo);
        assert_eq!(
            imp_after.blobs.get("docs/bug-reports.md"),
            imp_before.blobs.get("docs/bug-reports.md"),
            "the import bookkeeping must not move for a stale pass"
        );
        assert_eq!(
            fs::read_to_string(fx.helper_dir.join("docs/bug-reports.md")).unwrap_or_default(),
            helper_bugs_before,
            "reconcile must not write the old checkout's rows into the new binding"
        );
    }

    /// T-000243 review (sixth pass): `accept_new_main_branch` resolves the
    /// status between phase A and `finish` — the generation bump makes the
    /// older refused pass stale, so it cannot restore the rejection the
    /// user just accepted.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn an_accepted_main_branch_is_not_restored_by_a_stale_pass() {
        let fx = two_installs();
        fx.make_participant();
        // The owner moves the dev branch: the helper's pin now refuses it.
        let structure = fs::read_to_string(fx.main_dir.join(".sdh/project.json")).unwrap();
        let moved = structure.replace("\"dev_branch\": \"dev\"", "\"dev_branch\": \"next\"");
        assert_ne!(moved, structure, "the fixture structure must carry the pin");
        write(&fx.main_dir, ".sdh/project.json", &moved);
        g(&fx.main_dir, &["add", ".sdh"]);
        g(&fx.main_dir, &["commit", "-q", "-m", "move dev branch"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
        // The first sync settles the untrusted status; a second phase A
        // captures the same refusal in a fresh pass.
        fx.sync_helper();
        assert_eq!(fx.helper_status().as_deref(), Some("shared_untrusted"));
        let project = fx
            .helper_db
            .list_all_repos()
            .unwrap()
            .into_iter()
            .find_map(|r| r.project_id);
        let shared = run_phase_a(&fx.helper_db, Some(&gb()), project);
        // The user accepts the new branch: pin and status are cleared.
        accept_new_main_branch(&fx.helper_db, fx.helper_repo).unwrap();
        assert_eq!(fx.helper_status(), None);
        // The stale pass must not settle the refusal back.
        let _out = shared.finish(&fx.helper_db, Some(&gb()));
        assert_eq!(
            fx.helper_status(),
            None,
            "the accepted branch must not be refused again by a stale pass"
        );
    }

    /// T-000243 review (seventh pass): an obsolete disable retry must not
    /// clear a NEWER pending-disable marker. `clear_disabling` verifies the
    /// lifecycle generation it was started under — a re-enable plus a fresh
    /// disable bumped it meanwhile, so the old retry's cleanup is refused.
    #[test]
    fn an_obsolete_retry_cannot_clear_a_newer_disable_marker() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let mut imp = ss::SharedImport::default();
        imp.disabling = Some("dev".into());
        ss::write_shared_import(&db, repo.id, &imp).unwrap();
        // The retry pass starts under the current generation…
        let gen0 = lifecycle_generation(repo.id, root);
        // …then a newer lifecycle lands: re-enable + a fresh disable.
        bump_lifecycle_generation(repo.id, root);
        bump_lifecycle_generation(repo.id, root);
        assert_eq!(
            clear_disabling(&db, repo.id, root, gen0).unwrap(),
            false,
            "the obsolete retry must leave the newer marker alone"
        );
        assert_eq!(
            ss::read_shared_import(&db, repo.id).disabling.as_deref(),
            Some("dev")
        );
        // The retry of the CURRENT operation may clear it.
        let now = lifecycle_generation(repo.id, root);
        assert_eq!(clear_disabling(&db, repo.id, root, now).unwrap(), true);
        assert!(ss::read_shared_import(&db, repo.id).disabling.is_none());
    }

    /// T-000243 review (seventh pass): the generation key carries the local
    /// path — rebinding the repo must stale passes keyed by the old
    /// spelling, on both sides of the change. Eighth pass: the rebind is
    /// one guarded operation, so the invalidation cannot be split from
    /// the binding write.
    #[test]
    fn a_repathed_repo_invalidates_passes_keyed_by_the_old_path() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        db.set_repo_local_path(repo.id, Some("/old/root")).unwrap();
        let old = Path::new("/old/root");
        let new = Path::new("/new/root");
        let g_old = lifecycle_generation(repo.id, old);
        let g_new = lifecycle_generation(repo.id, new);
        rebind_local_path(&db, repo.id, Some("/new/root")).unwrap();
        assert_ne!(
            lifecycle_generation(repo.id, old),
            g_old,
            "a pass captured under the old path must go stale"
        );
        assert_ne!(lifecycle_generation(repo.id, new), g_new);
        // Ninth pass: a spelling that differs only by a trailing separator
        // compares EQUAL under `Path` but is a DIFFERENT literal generation
        // key — both spellings must be bumped, never normalized away.
        let g_trailing = lifecycle_generation(repo.id, Path::new("/new/root/"));
        rebind_local_path(&db, repo.id, Some("/new/root/")).unwrap();
        assert_ne!(
            lifecycle_generation(repo.id, Path::new("/new/root/")),
            g_trailing,
            "the new spelling's own key must be bumped too"
        );
    }

    /// T-000243 review (eighth pass): an OBSOLETE phase-A cleanup — one
    /// whose `NotFound` observation predates a newer accepted structure —
    /// must not erase the role the newer lifecycle established. The
    /// freshness gate compares the cleanup's expected generation with the
    /// current one under the guard.
    #[test]
    fn an_obsolete_cleanup_cannot_erase_a_newer_role() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        db.set_shared_role(repo.id, Some("contributor")).unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        // Sync A observed `NotFound` under generation g0 and paused…
        let g0 = lifecycle_generation(repo.id, root);
        // …then the owner republished and sync B accepted: a NEW lifecycle.
        db.set_shared_role(repo.id, Some("pending")).unwrap();
        bump_lifecycle_generation(repo.id, root);
        // A resumes: its cleanup is obsolete — the gate refuses the write.
        assert_eq!(foreign_state_cleanup(&db, repo.id, root, g0), 0);
        assert_eq!(
            db.get_shared_state(repo.id).unwrap().shared_role.as_deref(),
            Some("pending"),
            "the newer accepted role must survive the obsolete cleanup"
        );
        // The same cleanup under the CURRENT generation does clear — and
        // reports its bump count so a same-pass pin cleanup stays gated
        // on the RESULT (ninth pass).
        let now = lifecycle_generation(repo.id, root);
        assert_eq!(foreign_state_cleanup(&db, repo.id, root, now), 1);
        assert_eq!(db.get_shared_state(repo.id).unwrap().shared_role, None);
        assert_eq!(
            lifecycle_generation(repo.id, root),
            now + 1,
            "the reported bump count matches the generation move"
        );
    }

    /// T-000243 review (eighth pass): the pass's own transition is gated
    /// the same way — a stale `expected` skips the write entirely.
    #[test]
    fn a_stale_expected_generation_skips_the_passs_own_transition() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let g0 = lifecycle_generation(repo.id, root);
        let mut pass = new_pass(&repo, root, "me");
        // A foreign lifecycle lands before the pass's own write.
        bump_lifecycle_generation(repo.id, root);
        pass_role_transition(&mut pass, &db, Some("contributor"), g0);
        assert_eq!(
            db.get_shared_state(repo.id).unwrap().shared_role,
            None,
            "the obsolete transition must not write"
        );
        assert_eq!(pass.own_bumps, 0, "and must not count a bump it skipped");
    }

    /// T-000243 review (seventh pass): a newer phase-A `NotFound` cleanup
    /// (belongs to no pass) clears the cached role — an OLDER pass refused
    /// as untrusted must not settle `shared_untrusted` back over it.
    #[test]
    fn a_newer_not_found_cleanup_stales_an_older_refused_pass() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        db.set_shared_role(repo.id, Some("contributor")).unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let mut pass = new_pass(&repo, root, "me");
        pass.note_untrusted("structure refused".into());
        let g0 = lifecycle_generation(repo.id, root);
        pass.seal_lifecycle_gen(g0);
        // The newer sync's phase A finds no structure at all: the cleanup
        // is a lifecycle transition even though it produces no pass. Its
        // own generation is still current, so the gate lets it through.
        foreign_state_cleanup(&db, repo.id, root, g0);
        let mut sync = SharedSync::empty();
        sync.passes.push(pass);
        let _out = sync.finish(&db, None);
        assert_eq!(
            db.get_shared_state(repo.id).unwrap().shared_status,
            None,
            "the stale refusal must not be restored over the cleanup"
        );
    }

    /// T-000243 review (seventh pass): the counterpart — a pass's OWN
    /// lifecycle transition (role adoption) bumps the generation too, but
    /// `seal_lifecycle_gen` forgives exactly its own bumps, so the pass
    /// still settles its status.
    #[test]
    fn a_passs_own_role_adoption_does_not_stale_it() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let g0 = lifecycle_generation(repo.id, root);
        let mut pass = new_pass(&repo, root, "me");
        pass_role_transition(&mut pass, &db, Some("contributor"), g0);
        pass.note(SharedStatus::NoOwnBranch, "");
        pass.seal_lifecycle_gen(g0);
        let mut sync = SharedSync::empty();
        sync.passes.push(pass);
        let _out = sync.finish(&db, None);
        assert_eq!(
            db.get_shared_state(repo.id)
                .unwrap()
                .shared_status
                .as_deref(),
            Some("no_own_branch"),
            "the pass's own transition must not stale it"
        );
    }

    /// T-000243 review (second pass): every writer of the policy-bearing
    /// structure waits for the per-repo guard — `enable_shared_mode`,
    /// `approve_join_request` and `disable_shared_mode` (it commits the
    /// deletion of `.sdh/`) are not answered while a policy section holds it.
    /// The end-to-end interleaving with phase B and the OFF setter is
    /// `an_approval_and_a_specs_toggle_never_publish_a_stale_on_structure`.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn structure_writers_wait_for_the_policy_guard() {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.set_setting("github_login", "Owner").unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        let repo = db.get_repository_by_name("Owner/api").unwrap();
        // A bare folder with a `.git` entry passes `repo_root`; every writer
        // then reaches the guard without spawning git (they fail fast on the
        // missing project / dev branch / checkout only after the release).
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        db.set_repo_local_path(repo.id, Some(dir.path().to_str().unwrap()))
            .unwrap();
        db.set_shared_mode(repo.id, true).unwrap();
        db.set_shared_role(repo.id, Some("main")).unwrap();

        let guard = specs_policy_guard(repo.id);
        let (en_tx, en_rx) = std::sync::mpsc::channel();
        let (ap_tx, ap_rx) = std::sync::mpsc::channel();
        let (di_tx, di_rx) = std::sync::mpsc::channel();
        std::thread::scope(|s| {
            let held = guard.lock().unwrap();
            s.spawn(|| {
                let _ = en_tx.send(enable_shared_mode(&db, &gb(), repo.id, false));
            });
            s.spawn(|| {
                let _ = ap_tx.send(approve_join_request(&db, Some(&gb()), repo.id, "someone"));
            });
            s.spawn(|| {
                let _ = di_tx.send(disable_shared_mode(&db, &gb(), repo.id));
            });
            assert!(
                en_rx.recv_timeout(Duration::from_millis(300)).is_err(),
                "enable must wait for the policy guard"
            );
            assert!(
                ap_rx.recv_timeout(Duration::from_millis(300)).is_err(),
                "the approval must wait for the policy guard"
            );
            assert!(
                di_rx.recv_timeout(Duration::from_millis(300)).is_err(),
                "disable must wait for the policy guard"
            );
            // Release before the scope joins the writers.
            drop(held);
        });
    }

    /// T-000243 review (already-shared repos): a user rule that keeps the
    /// folder ignored while the effective policy is ON must not silently kill
    /// the transport — the recurring pass diagnoses it (once per change) and
    /// never removes the rule by itself.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_user_rule_ignoring_specs_is_diagnosed_while_the_policy_is_on() {
        let fx = two_installs();
        fx.make_participant();
        // This repo's own case: a user rule that hides the folder without
        // naming it, kept by the owner on purpose (never auto-removed).
        write(&fx.main_dir, ".gitignore", "superpowers/\n");
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# x\n");

        // Enabled with OFF: the folder is not a shared path, its rules are the
        // user's own business — no diagnostic, no note, nothing travels.
        assert!(set_repo_share_specs(&fx.main_db, fx.main_repo, false)
            .unwrap()
            .is_none());
        let out = fx.sync_main();
        assert!(
            !out.issues
                .iter()
                .any(|i| i.target.as_deref() == Some(".gitignore")),
            "{:?}",
            out.issues
        );
        assert_eq!(fx.remote_file("dev", "docs/superpowers/specs/x.md"), None);

        // Flipped ON later, after the repo is already shared: the setter says
        // right away that user rules still hide the folder ...
        let note = set_repo_share_specs(&fx.main_db, fx.main_repo, true)
            .unwrap()
            .expect("the ON transition of an already-shared repo must warn");
        assert!(note.contains("superpowers/"), "{}", note);
        // ... and the recurring pass diagnoses it instead of silently sending
        // nothing.
        let out = fx.sync_main();
        let diag = out
            .issues
            .iter()
            .find(|i| {
                i.code == Code::ManagedFileFailed && i.target.as_deref() == Some(".gitignore")
            })
            .expect("the silently blocked specs must be diagnosed");
        assert!(
            diag.detail
                .as_deref()
                .unwrap_or_default()
                .contains("superpowers/"),
            "{:?}",
            diag
        );
        // The rule survived: removal stays behind the preflight confirmation.
        assert_eq!(
            fs::read_to_string(fx.main_dir.join(".gitignore")).unwrap(),
            "superpowers/\n"
        );
        // And nothing was silently committed: the folder is ignored, so no
        // specs pathspec had anything to send.
        assert_eq!(fx.remote_file("dev", "docs/superpowers/specs/x.md"), None);
        assert_eq!(
            g(&fx.main_dir, &["ls-files", "--", "docs/superpowers/"]),
            ""
        );

        // Opting out stops the diagnosis: the folder is the user's own
        // business again, no issue, no noise.
        set_repo_share_specs(&fx.main_db, fx.main_repo, false).unwrap();
        let out = fx.sync_main();
        assert!(
            !out.issues
                .iter()
                .any(|i| i.target.as_deref() == Some(".gitignore")),
            "{:?}",
            out.issues
        );
    }

    fn sample_structure() -> ProjectStructure {
        ProjectStructure {
            format: 1,
            project: ss::StructProject {
                name: "Acme".into(),
                project_type: "standard".into(),
            },
            main: ss::StructMain {
                login: "Owner".into(),
                dev_branch: "dev".into(),
            },
            repos: vec![ss::StructRepo {
                github: "Owner/api".into(),
                role: "server".into(),
                share_specs: None,
            }],
            microservices: vec![],
            participants: vec![],
        }
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
            share_specs: true,
        };
        assert!(!is_shared_state(&st(false, None)));
        assert!(is_shared_state(&st(true, None)));
        assert!(is_shared_state(&st(false, Some("contributor"))));
        assert!(is_shared_state(&st(false, Some("pending"))));
    }

    // ----- pathspec per role ------------------------------------------------

    #[test]
    fn pathspec_per_role() {
        let main = shared_pathspec(Role::Main, "Owner", true);
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
            "docs/superpowers",
        ] {
            assert!(main.iter().any(|x| x == p), "main lacks {}", p);
        }
        assert_eq!(exclude_specs(&main).len(), 2);

        // T-000243: the specs folder leaves the pathspec of BOTH roles when the
        // policy is off; `docs/sdh_skills/` stays main-only either way.
        let main_off = shared_pathspec(Role::Main, "Owner", false);
        assert!(!main_off.iter().any(|x| x == "docs/superpowers"));
        assert!(main_off.iter().any(|x| x == "docs/sdh_skills"));

        let contributor = shared_pathspec(Role::Contributor, "helper", true);
        assert!(contributor.iter().any(|x| x == "docs/todo.md"));
        assert!(contributor.iter().any(|x| x == "docs/superpowers"));
        assert!(!contributor.iter().any(|x| x.starts_with(".sdh")));
        assert!(!contributor.iter().any(|x| x == "docs/sdh_skills"));
        assert!(exclude_specs(&contributor).is_empty());

        let contributor_off = shared_pathspec(Role::Contributor, "helper", false);
        assert!(!contributor_off.iter().any(|x| x == "docs/superpowers"));

        // A pending helper sends only its request — specs never travel for it,
        // whatever the flag says.
        for flag in [true, false] {
            assert_eq!(
                shared_pathspec(Role::Pending, "helper", flag),
                vec![".sdh/requests/helper.json".to_string()]
            );
        }
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
        // T-000243: a spec file of a pending helper never travels either,
        // whatever the specs flag says.
        write(&fx.helper_dir, "docs/superpowers/specs/p.md", "# p\n");
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

    // ----- T-000243: docs/superpowers/ in the shared set -----------------------

    /// Preflight treats the rules that hide the specs folder exactly per the
    /// specs flag: while specs are shared they are blockers a user must confirm
    /// to remove; opted out, the folder is not a shared path and its rules are
    /// the user's own business. A rule of the managed block is never reported.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn preflight_treats_superpowers_rules_per_the_specs_flag() {
        let fx = two_installs();
        // A user rule like this repo's own `superpowers/`: it hides
        // docs/superpowers/ without naming it.
        write(&fx.main_dir, ".gitignore", "superpowers/\n");
        let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(
            pre.blocking_lines,
            vec![IgnoreLine {
                line_no: 1,
                text: "superpowers/".into()
            }],
            "while specs are shared, a rule hiding the folder blocks them"
        );
        // The direct spelling is diagnosed the same way.
        write(&fx.main_dir, ".gitignore", "docs/superpowers/\n");
        let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(
            pre.blocking_lines,
            vec![IgnoreLine {
                line_no: 1,
                text: "docs/superpowers/".into()
            }]
        );

        // Opted out: the very same rules are no blockers and are not offered
        // for removal — the folder is meant to stay ignored.
        fx.main_db
            .set_repo_share_specs(fx.main_repo, false)
            .unwrap();
        let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(pre.blocking_lines.is_empty(), "{:?}", pre.blocking_lines);
        // And back on, they are blockers again (diagnosed, never auto-removed).
        fx.main_db.set_repo_share_specs(fx.main_repo, true).unwrap();
        let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert_eq!(pre.blocking_lines.len(), 1);

        // A managed-only rule is the sync's own business, not the user's.
        write(
            &fx.main_dir,
            ".gitignore",
            "# --- solo-dev-hub:begin ---\ndocs/superpowers/\n# --- solo-dev-hub:end ---\n",
        );
        let pre = shared_mode_preflight(&fx.main_db, &gb(), fx.main_repo).unwrap();
        assert!(pre.blocking_lines.is_empty(), "{:?}", pre.blocking_lines);
    }

    /// Part 1: while the policy shares specs, they travel both ways — the
    /// owner's specs, plans and HTML mockups reach the contributor, the
    /// contributor's own specs reach the owner, and two sides adding different
    /// spec files both survive (no merge driver, different files never collide).
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn specs_travel_between_owner_and_contributor_while_shared() {
        let fx = two_installs();
        fx.make_participant();

        // Owner -> contributor: a spec, a plan and an HTML mockup.
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# spec\n");
        write(&fx.main_dir, "docs/superpowers/plans/p.md", "# plan\n");
        write(
            &fx.main_dir,
            "docs/superpowers/mockups/m.html",
            "<html><body>mock</body></html>\n",
        );
        fx.sync_main();
        for branch in ["dev", "helper-dev"] {
            for path in [
                "docs/superpowers/specs/x.md",
                "docs/superpowers/plans/p.md",
                "docs/superpowers/mockups/m.html",
            ] {
                assert!(
                    fx.remote_file(branch, path).is_some(),
                    "{} lacks {}",
                    branch,
                    path
                );
            }
        }
        fx.sync_helper();
        assert!(fx.helper_dir.join("docs/superpowers/specs/x.md").is_file());
        assert!(fx
            .helper_dir
            .join("docs/superpowers/mockups/m.html")
            .is_file());

        // Contributor -> owner: its own spec travels back.
        write(
            &fx.helper_dir,
            "docs/superpowers/specs/helper.md",
            "# helper spec\n",
        );
        fx.sync_helper();
        fx.sync_main();
        assert!(fx
            .main_dir
            .join("docs/superpowers/specs/helper.md")
            .is_file());

        // Two sides add DIFFERENT spec files: both survive everywhere.
        write(&fx.main_dir, "docs/superpowers/specs/mine.md", "# mine\n");
        write(
            &fx.helper_dir,
            "docs/superpowers/specs/theirs.md",
            "# theirs\n",
        );
        fx.sync_main();
        fx.sync_helper();
        fx.sync_main();
        fx.sync_helper();
        for branch in ["dev", "helper-dev"] {
            for path in [
                "docs/superpowers/specs/mine.md",
                "docs/superpowers/specs/theirs.md",
            ] {
                assert!(
                    fx.remote_file(branch, path).is_some(),
                    "{} lacks {}",
                    branch,
                    path
                );
            }
        }
        assert!(fx
            .main_dir
            .join("docs/superpowers/specs/theirs.md")
            .is_file());
        // The helper committed `theirs.md` while origin/helper-dev had already
        // taken main's plumbed `mine.md` commit, so its own branch diverged —
        // the transport's designed state, identical for every shared path in
        // this interleaving (see `main_plumbs_a_merged_todo_into_a_participant_
        // branch`): the files travelled (both remotes, and main's checkout got
        // theirs.md), the helper's checkout waits for the Merge button.
        assert_eq!(
            fx.helper_status().as_deref(),
            Some("needs_manual_merge"),
            "the diverged own branch is reported, not silently merged"
        );
        assert_eq!(
            fx.helper_status_detail().as_deref(),
            Some(r#"{"branch":"helper-dev","current":true}"#)
        );
        // Two sides adding DIFFERENT spec files: the merge keeps both (specs
        // carry no merge driver; different files never collide).
        let res = merge_shared_upstream(&fx.helper_db, &gb(), fx.helper_repo).unwrap();
        assert_eq!(res.outcome, "merged");
        assert!(fx
            .helper_dir
            .join("docs/superpowers/specs/mine.md")
            .is_file());
        assert!(fx
            .helper_dir
            .join("docs/superpowers/specs/theirs.md")
            .is_file());
        fx.sync_helper();
        assert_eq!(fx.helper_status(), None);
        // After the merge both files live on every branch.
        for branch in ["dev", "helper-dev"] {
            for path in [
                "docs/superpowers/specs/mine.md",
                "docs/superpowers/specs/theirs.md",
            ] {
                assert!(fx.remote_file(branch, path).is_some());
            }
        }
    }

    /// Part 2, the owner's case: opting out BEFORE the first specs commit keeps
    /// the folder out of the transport on both installs — nothing under
    /// docs/superpowers/ reaches origin/dev or the participant branch, while
    /// todo keeps travelling and no file is ever tracked.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn specs_opted_out_before_the_first_commit_never_leave_the_install() {
        let fx = two_installs();
        // The owner's choice is stored before sharing is even enabled.
        fx.main_db
            .set_repo_share_specs(fx.main_repo, false)
            .unwrap();
        // T-000243 review: the spec files of both installs exist BEFORE the
        // structure / join syncs, so the first syncs of the transport run over
        // pre-existing files (a bug exposing them at the first render or commit
        // is caught here, not only by files written later).
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# x\n");
        write(&fx.helper_dir, "docs/superpowers/specs/y.md", "# y\n");
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500000"]));
        fx.make_participant();
        // ... and is published in the trusted structure the helper accepts.
        assert!(fx
            .remote_file("dev", ".sdh/project.json")
            .unwrap()
            .contains("\"share_specs\": false"));

        fx.sync_main();
        fx.sync_helper();
        fx.sync_main();

        for branch in ["dev", "helper-dev"] {
            for path in ["docs/superpowers/specs/x.md", "docs/superpowers/specs/y.md"] {
                assert_eq!(
                    fx.remote_file(branch, path),
                    None,
                    "{} must not carry {}",
                    branch,
                    path
                );
            }
            // The contributor-mode backbone is untouched by the flag.
            assert!(fx
                .remote_file(branch, "docs/todo.md")
                .unwrap()
                .contains("T-500000"));
        }
        // Never tracked on either clone.
        assert!(g(&fx.main_dir, &["ls-files", "--", "docs/superpowers/"])
            .trim()
            .is_empty());
        assert!(g(&fx.helper_dir, &["ls-files", "--", "docs/superpowers/"])
            .trim()
            .is_empty());
        // The helper's own database still says the default on: the accepted
        // structure overrode it before any path was computed.
        assert!(
            fx.helper_db
                .get_shared_state(fx.helper_repo)
                .unwrap()
                .share_specs
        );
    }

    /// Part 2, off AFTER specs were committed: the files stay tracked, local
    /// edits stay visible modifications, the history stays reachable — and no
    /// NEW SDH commit carries the folder on the current branch or a recipient
    /// branch, for either role. On resumes sharing.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn off_keeps_committed_specs_tracked_and_out_of_new_sync_commits() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# x\n");
        fx.sync_main();
        assert!(fx
            .remote_file("dev", "docs/superpowers/specs/x.md")
            .is_some());
        fx.sync_helper();
        assert!(fx.helper_dir.join("docs/superpowers/specs/x.md").is_file());

        // The owner opts out and edits the file.
        fx.main_db
            .set_repo_share_specs(fx.main_repo, false)
            .unwrap();
        write(
            &fx.main_dir,
            "docs/superpowers/specs/x.md",
            "# x edited by owner\n",
        );
        fx.sync_main();
        // Tracked (off never untracks) and visibly modified, but carried by no
        // new commit: a plain work-tree modification, nothing staged. (`g()`
        // trims the leading space of a porcelain line, so judge via diff.)
        assert_eq!(
            g(&fx.main_dir, &["ls-files", "--", "docs/superpowers/"]).trim(),
            "docs/superpowers/specs/x.md"
        );
        assert_eq!(
            g(
                &fx.main_dir,
                &["diff", "--name-only", "--", "docs/superpowers/"]
            ),
            "docs/superpowers/specs/x.md",
            "the edit stays a visible local modification"
        );
        assert_eq!(
            g(
                &fx.main_dir,
                &["diff", "--cached", "--name-only", "--", "docs/superpowers/"]
            ),
            "",
            "nothing under the folder is staged into a new commit"
        );
        // The edit reached no NEW commit, on the current branch (dev) or the
        // participant branch (helper-dev).
        for branch in ["dev", "helper-dev"] {
            assert_eq!(
                fx.remote_file(branch, "docs/superpowers/specs/x.md")
                    .as_deref(),
                Some("# x\n"),
                "{} must keep the last shared version",
                branch
            );
        }
        // The history of the file is intact.
        assert!(g(
            &fx.main_dir,
            &["log", "--oneline", "--", "docs/superpowers/"]
        )
        .contains("sdh-sync"));

        // The contributor accepts the opt-out from the structure; its own edit
        // stays local too, while todo keeps travelling.
        fx.sync_helper();
        write(
            &fx.helper_dir,
            "docs/superpowers/specs/x.md",
            "# x edited by helper\n",
        );
        write(&fx.helper_dir, "docs/todo.md", &todo(&["T-500001"]));
        fx.sync_helper();
        for branch in ["helper-dev", "dev"] {
            assert_eq!(
                fx.remote_file(branch, "docs/superpowers/specs/x.md")
                    .as_deref(),
                Some("# x\n")
            );
        }
        assert!(fx
            .remote_file("helper-dev", "docs/todo.md")
            .unwrap()
            .contains("T-500001"));

        // On resumes sharing: the owner's edit finally travels. One more OFF
        // sync first, so dev is up to date with the helper's todo plumb — a
        // dirty specs file is outside the off pathspec, so the fast-forward
        // runs and leaves the work-tree edit alone (the incoming commit never
        // touched the file).
        fx.sync_main();
        assert_eq!(
            fx.remote_file("dev", "docs/superpowers/specs/x.md")
                .as_deref(),
            Some("# x\n"),
            "the off sync still carries no specs"
        );
        fx.main_db.set_repo_share_specs(fx.main_repo, true).unwrap();
        fx.sync_main();
        assert_eq!(
            fx.remote_file("dev", "docs/superpowers/specs/x.md")
                .as_deref(),
            Some("# x edited by owner\n")
        );
    }

    /// A specs commit created while the policy was ON but not yet pushed stays
    /// ours after the owner opts out: the branch is judged by the historical
    /// SDH allow-list, not by the current send set, so the pending commit is
    /// pushed as it is (off filters future commits only).
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn an_unsent_specs_commit_made_while_on_is_still_ours_after_off() {
        let fx = two_installs();
        fx.make_participant();
        // The push of the participant branch is refused: main's plumbed commit
        // carrying a spec stays local (created, not yet sent).
        let hook = block_pushes_to(&fx, "refs/heads/helper-dev");
        write(&fx.main_dir, "docs/superpowers/specs/x.md", "# x\n");
        fx.sync_main();
        assert_eq!(
            fx.remote_file("helper-dev", "docs/superpowers/specs/x.md"),
            None
        );
        fs::remove_file(&hook).unwrap();

        // The owner opts out; the already-created commit is not filtered away.
        fx.main_db
            .set_repo_share_specs(fx.main_repo, false)
            .unwrap();
        fx.sync_main();
        assert_eq!(
            fx.remote_file("helper-dev", "docs/superpowers/specs/x.md")
                .as_deref(),
            Some("# x\n")
        );
    }

    /// A missing (or empty) specs folder on either side is a successful no-op —
    /// no "pathspec did not match" — and a first file then travels normally.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_missing_specs_folder_is_a_noop_and_new_files_then_travel() {
        let fx = two_installs();
        fx.make_participant();
        // No specs folder on either side: both syncs are clean no-ops for the
        // folder — no "pathspec did not match", no bogus commit. (The first
        // owner sync after `make_participant` still plumbs the shared
        // structure into the participant branch, an ordinary transport step
        // that has nothing to do with specs, so `committed` is not asserted.)
        let out = fx.sync_main();
        assert!(out.issues.is_empty(), "{:?}", out.issues);
        for branch in ["dev", "helper-dev"] {
            assert!(
                g(&fx.origin, &["ls-tree", "-r", "--name-only", branch])
                    .lines()
                    .all(|l| !l.starts_with("docs/superpowers/")),
                "{} carries specs without the folder existing",
                branch
            );
        }
        let out = fx.sync_helper();
        assert!(out.issues.is_empty(), "{:?}", out.issues);

        write(
            &fx.helper_dir,
            "docs/superpowers/specs/first.md",
            "# first\n",
        );
        fx.sync_helper();
        fx.sync_main();
        assert!(fx
            .main_dir
            .join("docs/superpowers/specs/first.md")
            .is_file());
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

    // ----- T-000244: markers leave the shared branches as deletions only -----

    /// A commit this app could have made itself: the sync subject, the
    /// participant trailer and the SDH author, so a recipient branch still reads
    /// as holding nothing but our own sync commits.
    fn commit_as_sdh(dir: &Path, login: &str) {
        g(dir, &["add", "-A", "."]);
        g(
            dir,
            &[
                "-c",
                "user.name=Solo Dev Hub",
                "-c",
                "user.email=noreply@solodevhub.app",
                "commit",
                "-q",
                "-m",
                &format!(
                    "chore(sdh-sync): sync shared files\n\nShared project files merged and synced by Solo Dev Hub.\n\nSDH-Participant: {}",
                    login
                ),
            ],
        );
    }

    /// The marker files a branch still tracks in the bare remote.
    fn remote_markers(fx: &TwoInstalls, branch: &str) -> Vec<String> {
        use std::process::Command;
        let out = Command::new("git")
            .args([
                "ls-tree",
                "-r",
                "--name-only",
                branch,
                "--",
                "docs/sdh_skills/",
            ])
            .current_dir(&fx.origin)
            .output()
            .unwrap()
            .stdout;
        String::from_utf8_lossy(&out)
            .lines()
            .filter(|l| l.ends_with(".approved") || l.ends_with(".conflict"))
            .map(|l| l.to_string())
            .collect()
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn main_delivers_a_marker_deletion_from_his_current_branch() {
        let fx = two_installs();
        fx.make_participant();
        // The owner's agent committed the marker together with the rule edit.
        write(&fx.main_dir, "docs/sdh_skills/sdh-alpha.md", "ALPHA\n");
        write(
            &fx.main_dir,
            "docs/sdh_skills/sdh-alpha.md.approved",
            "approved in chat\n",
        );
        g(&fx.main_dir, &["add", "-A", "."]);
        g(&fx.main_dir, &["commit", "-q", "-m", "edit + marker"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
        // The refresh sweep staged the deletion; nothing else is left to commit.
        g(
            &fx.main_dir,
            &[
                "rm",
                "-q",
                "--cached",
                "docs/sdh_skills/sdh-alpha.md.approved",
            ],
        );

        let out = fx.sync_main();

        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert!(
            remote_markers(&fx, "dev").is_empty(),
            "the deletion reached the bare remote"
        );
        assert_eq!(
            fx.remote_file("dev", "docs/sdh_skills/sdh-alpha.md")
                .as_deref(),
            Some("ALPHA\n"),
            "the rule copy still travels"
        );
        assert_eq!(
            fs::read_to_string(fx.main_dir.join("docs/sdh_skills/sdh-alpha.md.approved")).unwrap(),
            "approved in chat\n",
            "the live marker is kept byte-for-byte"
        );
        // Nothing about the marker is left staged: the delivered deletion is
        // not a permanent `git status` line, and the live file is now simply
        // untracked.
        let status = g(&fx.main_dir, &["status", "--porcelain"]);
        assert!(
            !status
                .lines()
                .any(|l| l.starts_with('D') && l.contains(".approved")),
            "no staged marker deletion is left: {}",
            status
        );
        assert!(
            status.contains("?? docs/sdh_skills/sdh-alpha.md.approved"),
            "{}",
            status
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn main_never_adds_a_marker_not_even_a_modified_one() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.main_dir, "docs/sdh_skills/sdh-alpha.md", "ALPHA\n");
        write(
            &fx.main_dir,
            "docs/sdh_skills/sdh-alpha.md.approved",
            "approved\n",
        );
        g(&fx.main_dir, &["add", "-A", "."]);
        g(&fx.main_dir, &["commit", "-q", "-m", "edit + marker"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
        // A live marker of another rule, and the tracked one rewritten.
        write(
            &fx.main_dir,
            "docs/sdh_skills/sdh-beta.md.approved",
            "approved\n",
        );
        fs::write(
            fx.main_dir.join("docs/sdh_skills/sdh-alpha.md.approved"),
            "MODIFIED\n",
        )
        .unwrap();

        let out = fx.sync_main();

        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert!(remote_markers(&fx, "dev").is_empty(), "nothing was added");
        assert!(
            fx.remote_file("dev", "docs/sdh_skills/sdh-alpha.md")
                .is_some(),
            "the copy itself still travels"
        );
        assert_eq!(
            fs::read_to_string(fx.main_dir.join("docs/sdh_skills/sdh-alpha.md.approved")).unwrap(),
            "MODIFIED\n",
            "the local marker keeps its new text"
        );
        assert!(fx
            .main_dir
            .join("docs/sdh_skills/sdh-beta.md.approved")
            .exists());
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn main_plumbs_a_marker_deletion_into_a_participants_branch() {
        let fx = two_installs();
        fx.make_participant();
        // The helper's branch carries a marker committed by a sync-shaped
        // commit; the helper's own sync swept it out of its index.
        write(&fx.helper_dir, "docs/sdh_skills/sdh-alpha.md", "ALPHA\n");
        write(
            &fx.helper_dir,
            "docs/sdh_skills/sdh-alpha.md.approved",
            "approved\n",
        );
        commit_as_sdh(&fx.helper_dir, "helper");
        g(&fx.helper_dir, &["push", "-q", "origin", "helper-dev"]);
        g(
            &fx.helper_dir,
            &[
                "rm",
                "-q",
                "--cached",
                "docs/sdh_skills/sdh-alpha.md.approved",
            ],
        );

        let out = fx.sync_main();

        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert!(
            remote_markers(&fx, "helper-dev").is_empty(),
            "the plumbed deletion reached the participant's branch"
        );
        assert!(
            fx.remote_file("helper-dev", "docs/sdh_skills/sdh-alpha.md")
                .is_some(),
            "the rule copy is not deleted with it"
        );
        assert_eq!(
            fx.remote_file("dev", "docs/sdh_skills/sdh-alpha.md"),
            None,
            "the owner never publishes a rule copy he does not have"
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn contributor_delivers_a_marker_deletion_without_any_shared_content() {
        let fx = two_installs();
        fx.make_participant();
        // A contributor's checkout has no shared content at all: only the
        // deletion can travel, and it must — a branch may not keep a marker.
        write(
            &fx.helper_dir,
            "docs/sdh_skills/sdh-alpha.md.approved",
            "approved\n",
        );
        commit_as_sdh(&fx.helper_dir, "helper");
        g(&fx.helper_dir, &["push", "-q", "origin", "helper-dev"]);
        g(
            &fx.helper_dir,
            &[
                "rm",
                "-q",
                "--cached",
                "docs/sdh_skills/sdh-alpha.md.approved",
            ],
        );

        let out = fx.sync_helper();

        assert!(out.issues.is_empty(), "{:?}", out.issues);
        assert!(remote_markers(&fx, "helper-dev").is_empty());
        assert_eq!(
            fx.remote_file("helper-dev", "docs/sdh_skills/sdh-alpha.md"),
            None,
            "a contributor still publishes no rule copies"
        );
        assert!(
            fx.helper_dir
                .join("docs/sdh_skills/sdh-alpha.md.approved")
                .exists(),
            "the local marker survives the delivery"
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn pending_helper_plumbs_a_marker_deletion_out_of_mains_dev() {
        let fx = two_installs();
        fx.enable_main();
        // The owner's agent committed a marker into dev before the helper joined.
        write(&fx.main_dir, "docs/sdh_skills/sdh-alpha.md", "ALPHA\n");
        write(
            &fx.main_dir,
            "docs/sdh_skills/sdh-alpha.md.conflict",
            "# Hand-back not taken: conflict\n",
        );
        g(&fx.main_dir, &["add", "-A", "."]);
        g(
            &fx.main_dir,
            &["commit", "-q", "-m", "reason file committed"],
        );
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);

        let out = fx.sync_helper();

        // A pending helper always reports its pending status; that is the only
        // issue of this pass.
        assert_eq!(
            codes(&out),
            vec![Code::SharedPendingApproval],
            "{:?}",
            out.issues
        );
        assert!(
            remote_markers(&fx, "dev").is_empty(),
            "the pending helper's sync removed the marker from dev"
        );
        assert!(
            fx.remote_file("dev", "docs/sdh_skills/sdh-alpha.md")
                .is_some(),
            "the copy stays"
        );
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn a_marker_deletion_keeps_the_sync_commit_shape_of_the_branch() {
        let fx = two_installs();
        fx.make_participant();
        write(&fx.main_dir, "docs/sdh_skills/sdh-alpha.md", "ALPHA\n");
        write(
            &fx.main_dir,
            "docs/sdh_skills/sdh-alpha.md.approved",
            "approved\n",
        );
        g(&fx.main_dir, &["add", "-A", "."]);
        g(&fx.main_dir, &["commit", "-q", "-m", "edit + marker"]);
        g(&fx.main_dir, &["push", "-q", "origin", "dev"]);
        g(
            &fx.main_dir,
            &[
                "rm",
                "-q",
                "--cached",
                "docs/sdh_skills/sdh-alpha.md.approved",
            ],
        );
        fx.sync_main();

        // The deletion commit has the sync subject, the SDH author and the
        // participant trailer, and touches shared paths only — so the guard
        // that keeps a recipient branch pushable still accepts the branch.
        let shown = g(
            &fx.main_dir,
            &["show", "-s", "--format=%an%n%ae%n%s", "HEAD"],
        );
        assert!(shown.contains("Solo Dev Hub"), "{}", shown);
        assert!(
            shown.contains("chore(sdh-sync): sync shared files"),
            "{}",
            shown
        );
        assert!(
            g(&fx.main_dir, &["show", "-s", "--format=%b", "HEAD"])
                .contains("SDH-Participant: Owner"),
            "the participant trailer is in the body"
        );
        assert!(
            g(
                &fx.main_dir,
                &["diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD"]
            )
            .lines()
            .all(|f| f.starts_with("docs/sdh_skills/")),
            "the commit touches shared paths only"
        );
        // And the next sync is a no-op that pushes nothing new.
        let before = fx.remote_rev("dev").unwrap();
        let again = fx.sync_main();
        assert!(again.issues.is_empty(), "{:?}", again.issues);
        assert_eq!(fx.remote_rev("dev").unwrap(), before);
    }
}
