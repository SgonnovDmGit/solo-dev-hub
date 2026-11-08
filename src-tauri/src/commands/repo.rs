use crate::db::AppDb;
use crate::models::*;
use crate::{git_ops, git_sync, sync};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::State;

// ── Repository commands ───────────────────────────────────────────────────────

#[tauri::command]
pub fn create_local_repository(
    db: State<AppDb>,
    local_path: String,
    display_name: String,
    project_id: Option<i64>,
    role: Option<String>,
) -> Result<Repository, String> {
    db.insert_local_repository(&local_path, &display_name, project_id, role.as_deref())
        .map_err(|e| e.to_string())
}

/// T-000216: contributor-mode state of one repo.
#[tauri::command]
pub fn get_repo_shared_state(db: State<AppDb>, repo_id: i64) -> Result<RepoSharedState, String> {
    db.get_shared_state(repo_id).map_err(|e| e.to_string())
}

/// T-000216: contributor-mode state of every repo that takes part in shared mode.
#[tauri::command]
pub fn list_repo_shared_states(db: State<AppDb>) -> Result<Vec<RepoSharedState>, String> {
    db.list_shared_states().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn upsert_repository(
    db: State<AppDb>,
    github_name: String,
    github_url: Option<String>,
    description: Option<String>,
    language: Option<String>,
    last_pushed_at: Option<String>,
    github_id: Option<i64>,
) -> Result<UpsertRepoOutcome, String> {
    db.upsert_repository_with_outcome(
        &github_name,
        github_url.as_deref(),
        description.as_deref(),
        language.as_deref(),
        last_pushed_at.as_deref(),
        github_id,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn resolve_merge_with_local(
    db: State<AppDb>,
    local_id: i64,
    github_name: String,
    github_url: Option<String>,
    description: Option<String>,
    language: Option<String>,
    last_pushed_at: Option<String>,
    github_id: Option<i64>,
) -> Result<Repository, String> {
    db.resolve_merge_with_local(
        local_id,
        &github_name,
        github_url.as_deref(),
        description.as_deref(),
        language.as_deref(),
        last_pushed_at.as_deref(),
        github_id,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn force_insert_github_repo(
    db: State<AppDb>,
    github_name: String,
    github_url: Option<String>,
    description: Option<String>,
    language: Option<String>,
    last_pushed_at: Option<String>,
    github_id: Option<i64>,
) -> Result<Repository, String> {
    db.force_insert_github_repo(
        &github_name,
        github_url.as_deref(),
        description.as_deref(),
        language.as_deref(),
        last_pushed_at.as_deref(),
        github_id,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn assign_repository(
    db: State<AppDb>,
    id: i64,
    project_id: Option<i64>,
    role: Option<String>,
) -> Result<Repository, String> {
    // Codex composition gate: when assigning INTO a project, block roles a codex
    // project can't hold. When project_id is None the repo is being unassigned — no gate.
    if let Some(pid) = project_id {
        db.enforce_codex_repo_gate(pid, role.as_deref(), id)?;
    }
    db.assign_repository(id, project_id, role.as_deref())
        .map_err(|e| e.to_string())
}

// ── F-025 Manual ordering commands ────────────────────────────────────────────

#[tauri::command]
pub fn reorder_project(db: State<AppDb>, id: i64, direction: String) -> Result<(), String> {
    db.reorder_project(id, &direction)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reorder_repo(db: State<AppDb>, repo_id: i64, direction: String) -> Result<(), String> {
    db.reorder_repo(repo_id, &direction)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rebalance_repo_group(db: State<AppDb>, ordered_ids: Vec<i64>) -> Result<(), String> {
    db.rebalance_repo_group(&ordered_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rebalance_projects(db: State<AppDb>, ordered_ids: Vec<i64>) -> Result<(), String> {
    db.rebalance_projects(&ordered_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn auto_sort_all(db: State<AppDb>) -> Result<(), String> {
    db.auto_sort_all().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_repos_by_project(
    db: State<AppDb>,
    project_id: Option<i64>,
) -> Result<Vec<Repository>, String> {
    db.list_repos_by_project(project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_all_repos(db: State<AppDb>) -> Result<Vec<Repository>, String> {
    db.list_all_repos().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_repository(db: State<AppDb>, id: i64) -> Result<Repository, String> {
    db.get_repository(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_repository_by_name(db: State<AppDb>, github_name: String) -> Result<Repository, String> {
    db.get_repository_by_name(&github_name)
        .map_err(|e| e.to_string())
}

// ── local_path command ────────────────────────────────────────────────────────

// Async: the rebind takes the per-repo policy guard, which phase B holds
// across git/network work.
#[tauri::command(async)]
pub fn set_repo_local_path(
    db: State<AppDb>,
    id: i64,
    local_path: Option<String>,
) -> Result<Repository, String> {
    // T-000243 review (eighth pass): binding write and generation invalidation
    // are ONE guarded operation — a pass initializing concurrently sees either
    // the old binding with its pre-bump generation (then goes stale) or the
    // new binding with the post-bump one.
    crate::sync::shared::rebind_local_path(&db, id, local_path.as_deref())
}

#[tauri::command]
pub fn update_repo_description(
    db: State<AppDb>,
    repo_id: i64,
    new_description: String,
) -> Result<Repository, String> {
    db.update_repo_description(repo_id, &new_description)
        .map_err(|e| e.to_string())
}

// B-003: full repo removal — DB row, plus optional local `.git` cleanup.
// GitHub deletion is handled on the JS side (Octokit). DB cascades handle bug_notes, project_microservices.
#[tauri::command]
pub fn delete_repository(
    db: State<AppDb>,
    id: i64,
    clear_git_local: bool,
    local_path: Option<String>,
) -> Result<(), String> {
    db.delete_repository(id).map_err(|e| e.to_string())?;
    // v0.20.0: cleanup grid-state settings to prevent orphan rows
    let _ = db.delete_setting(&format!("tasks_grid_state_{}", id));
    let _ = db.delete_setting(&format!("done_grid_state_{}", id));
    if clear_git_local {
        if let Some(lp) = local_path {
            sync::remove_git_dir(std::path::Path::new(&lp))?;
        }
    }
    Ok(())
}

// ── F-000041: untrack gitignored files ────────────────────────────────────────
// Async commands: each spawns git, and a git spawn on the owner's Windows
// machine measured 0.7–7.8 s, which a sync command spent on the UI thread with
// the window frozen (2026-10-24, the repo screen asks on every open). The git
// binary comes from the per-process probe (`cached_git`). UI gates the Untrack
// button on `check_git_available_for_repo` so the two listing/untracking
// commands are only called when both binary and `.git/` are known to exist.

/// True iff a git binary is discoverable AND the repo has a `local_path`
/// pointing at something that looks like a git work tree. Lookup failure
/// (no such repo, no local_path) returns Ok(false) so the UI quietly hides
/// the Untrack button — symmetric with the missing-local-path UX elsewhere.
#[tauri::command(async)]
pub fn check_git_available_for_repo(db: State<AppDb>, repository_id: i64) -> Result<bool, String> {
    let repo = match db.get_repository(repository_id) {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };
    let local_path = match repo.local_path.as_deref() {
        Some(p) if !p.is_empty() => p,
        _ => return Ok(false),
    };
    let path = Path::new(local_path);
    Ok(git_ops::cached_git().is_some() && git_ops::is_git_repo(path))
}

/// Read `git ls-files -ci --exclude-standard -z` plus repo-state and
/// other-staged count in one call so the dialog has everything it needs on
/// open. Callers should have already gated on `check_git_available_for_repo`,
/// but we re-validate locally (errors here are surfaced to the dialog's error
/// state — see UntrackGitignoredDialog).
#[tauri::command(async)]
pub fn list_gitignored_tracked(
    db: State<AppDb>,
    repository_id: i64,
) -> Result<GitignoredListing, String> {
    let repo = db
        .get_repository(repository_id)
        .map_err(|e| e.to_string())?;
    let local_path = repo
        .local_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "Repository has no local_path".to_string())?;
    let path = Path::new(local_path);

    let git = git_ops::cached_git().ok_or_else(|| "git not available".to_string())?;

    let files = git_ops::list_gitignored_tracked(&git, path)?;
    let repo_state = match git_ops::detect_repo_state(path) {
        git_ops::RepoState::Clean => "clean",
        git_ops::RepoState::MidMerge => "mid_merge",
        git_ops::RepoState::MidRebase => "mid_rebase",
    }
    .to_string();
    let other_staged_count = git_ops::count_other_staged_changes(&git, path, &files)?;

    let display_files: Vec<String> = files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    Ok(GitignoredListing {
        files: display_files,
        repo_state,
        other_staged_count,
    })
}

/// Run `git rm --cached <files...>` for the user-selected subset. Errors are
/// per-chunk rather than per-file (matches the `git_ops` chunking model);
/// the dialog displays them as a partial-success toast.
#[tauri::command(async)]
pub fn untrack_files(
    db: State<AppDb>,
    repository_id: i64,
    files: Vec<String>,
) -> Result<UntrackReport, String> {
    let repo = db
        .get_repository(repository_id)
        .map_err(|e| e.to_string())?;
    let local_path = repo
        .local_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "Repository has no local_path".to_string())?;
    let path = Path::new(local_path);

    let git = git_ops::cached_git().ok_or_else(|| "git not available".to_string())?;

    let file_bufs: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    git_ops::untrack_files(&git, path, &file_bufs)
}

// ── F-000045: clone a repo that has no local folder ───────────────────────────

/// F-000045: clone a GitHub-tracked repo into `target_dir` (or adopt an
/// existing clone of the same repo found there) and bind the folder to the
/// repo row. Only for a repo whose `local_path` is missing/empty — an existing
/// binding is never overwritten (`set_repo_local_path` is the explicit path
/// for that). The branch comes from the repo's auto-commit setting, defaulting
/// to `dev`; a configured branch that is not a valid branch name is an error
/// (nothing is cloned), the setting itself is never rewritten. On any failure
/// the folder is NOT bound: the error is a `SyncIssue` with an F-000045 code
/// (`clone_target_invalid` / `clone_failed` / `clone_attach_failed`).
///
/// Async like the untrack commands above: a clone spends up to a minute on the
/// network, far too long for the UI thread. The DB mutex is never held across
/// the git work — every DB touch here is a short locked call.
#[tauri::command(async)]
pub fn clone_repo_to(
    db: State<AppDb>,
    id: i64,
    target_dir: String,
) -> Result<Repository, SyncIssue> {
    clone_repo_to_impl(&db, id, target_dir)
}

/// F-000045 review: one component of an `owner/repo` name. GitHub names are
/// alphanumerics, `-`, `_` and `.` — the allow-list rejects every URL-ish
/// character (`: / ? # @ \`, whitespace, control chars) in a single rule, and
/// the two traversal specials `.` and `..` are refused explicitly
/// (documented choice: this is the character set GitHub itself accepts when
/// creating an owner or repository name).
fn is_github_component(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// F-000045: `owner/repo` shape for a `github_name` — trimmed, exactly one
/// `/`, and both components valid GitHub names. A full URL, `a/b/c`, a
/// fragment, a backslash path or an embedded space can never name a GitHub
/// repository, so there is nothing to clone.
fn is_github_owner_repo(name: &str) -> bool {
    match name.trim().split_once('/') {
        Some((owner, repo)) => is_github_component(owner) && is_github_component(repo),
        None => false,
    }
}

/// The orchestration behind `clone_repo_to`, on a plain `&AppDb` so the
/// no-git failure paths are unit-testable without a Tauri runtime. Production
/// delegates with `url = None` (the GitHub URL built from the validated
/// `github_name`); tests drive the same orchestration with a local bare-repo
/// path through `clone_repo_to_impl_with_url`.
fn clone_repo_to_impl(db: &AppDb, id: i64, target_dir: String) -> Result<Repository, SyncIssue> {
    clone_repo_to_impl_with_url(db, id, target_dir, None)
}

fn clone_repo_to_impl_with_url(
    db: &AppDb,
    id: i64,
    target_dir: String,
    url: Option<&str>,
) -> Result<Repository, SyncIssue> {
    let repo = db.get_repository(id).map_err(|e| {
        SyncIssue::new(SyncIssueCode::CloneFailed, format!("#{}", id)).detail(e.to_string())
    })?;
    let subject = repo.display_name();

    // Local-only / malformed name: nothing GitHub-shaped to clone.
    let github_name = match repo.github_name.as_deref() {
        Some(name) if is_github_owner_repo(name) => name.trim().to_string(),
        _ => {
            return Err(SyncIssue::new(SyncIssueCode::CloneTargetInvalid, subject)
                .target(&target_dir)
                .detail("repository has no github_name of the form owner/repo"))
        }
    };
    // The command is for a repo with no folder: an existing binding is never
    // overwritten by a clone.
    if repo.local_path.as_deref().map_or(false, |p| !p.is_empty()) {
        return Err(SyncIssue::new(SyncIssueCode::CloneTargetInvalid, subject)
            .target(&target_dir)
            .detail("repository already has a local folder"));
    }

    // Fail fast on a target the prep step would refuse anyway.
    if target_dir.trim().is_empty() {
        return Err(SyncIssue::new(SyncIssueCode::CloneTargetInvalid, subject)
            .target(&target_dir)
            .detail("target folder path is empty"));
    }
    let target_path = PathBuf::from(&target_dir);
    if !target_path.is_absolute() {
        return Err(SyncIssue::new(SyncIssueCode::CloneTargetInvalid, subject)
            .target(&target_dir)
            .detail("target folder must be an absolute path"));
    }

    // Branch: the repo's auto-commit setting, default `dev`. A garbage name is
    // refused before anything is cloned; the setting is left as the user set it.
    let branch = db
        .get_autocommit_branch(id)
        .ok()
        .flatten()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| "dev".to_string());
    if let Err(msg) = git_sync::validate_branch(&branch) {
        return Err(SyncIssue::new(SyncIssueCode::CloneFailed, subject).detail(msg));
    }

    let git = git_ops::cached_git().ok_or_else(|| {
        SyncIssue::new(SyncIssueCode::CloneFailed, &subject).detail("git binary not available")
    })?;
    // `None` is production: the GitHub URL of the validated name. Tests pass
    // a local bare-repo path so the whole orchestration runs offline.
    let url = match url {
        Some(url) => url.to_string(),
        None => git_sync::github_clone_url(&github_name),
    };

    // FreshClone and ExistingClone both end in a bindable clone; for the
    // latter the origin check inside `prepare_clone_target` already proved it
    // is the same repo (ambiguous multi-URL origins are refused there).
    if let Err(e) = git_sync::prepare_clone_target(&git, &url, &github_name, &target_path, &branch)
    {
        return Err(match e {
            git_sync::CloneTargetError::Invalid(msg) => {
                SyncIssue::new(SyncIssueCode::CloneTargetInvalid, subject)
                    .target(&target_dir)
                    .detail(msg)
            }
            git_sync::CloneTargetError::Failed(net_err) => {
                // R3: the incomplete-clone marker (and the partial clone or
                // failed-checkout leftovers it names) sits next to the
                // RESOLVED target identity — with an aliased pick that is not
                // the lexical parent shown in `.target()`. Recovery is manual
                // by design, so the issue names the actual marker path and
                // what to delete. `net_err.message` is already
                // credential-redacted by git_sync.
                let marker = git_sync::clone_incomplete_marker(&target_path)
                    .map(|m| m.display().to_string())
                    .unwrap_or_else(|| ".sdh-clone-incomplete-*".to_string());
                SyncIssue::new(SyncIssueCode::CloneFailed, subject)
                    .target(&target_dir)
                    .detail(format!(
                        "{} — an incomplete-clone marker ({}) was left next to the clone \
                         leftovers; delete the marker and the leftovers by hand before \
                         retrying",
                        net_err.message, marker
                    ))
            }
        });
    }

    // Bind only if nobody attached a folder meanwhile; a loser never overwrites.
    match db.bind_repo_local_path_if_unset(id, &target_dir) {
        Ok(Some(bound)) => Ok(bound),
        Ok(None) => Err(SyncIssue::new(SyncIssueCode::CloneAttachFailed, subject)
            .detail("a local folder was bound concurrently")),
        Err(e) => {
            Err(SyncIssue::new(SyncIssueCode::CloneAttachFailed, subject).detail(e.to_string()))
        }
    }
}

// ── T-000191: peer links (InOutBox channel between equals) ────────────────────

/// Declare two repositories to be peers. Symmetric and idempotent. Every pair
/// is explicit: nothing is inferred from a shared role.
#[tauri::command]
pub fn add_peer_link(db: State<AppDb>, repo_a_id: i64, repo_b_id: i64) -> Result<(), String> {
    db.add_peer_link(repo_a_id, repo_b_id)
}

/// Drop an explicit peer link. Files already exchanged are left untouched —
/// unlinking only stops future syncing of the channel.
#[tauri::command]
pub fn remove_peer_link(db: State<AppDb>, repo_a_id: i64, repo_b_id: i64) -> Result<(), String> {
    db.remove_peer_link(repo_a_id, repo_b_id)
        .map_err(|e| e.to_string())
}

/// All explicit peer links in the portfolio, newest first.
#[tauri::command]
pub fn list_peer_links(db: State<AppDb>) -> Result<Vec<PeerLink>, String> {
    db.list_peer_links().map_err(|e| e.to_string())
}

/// Every repository explicitly linked as a peer of this one.
#[tauri::command]
pub fn list_repo_peers(db: State<AppDb>, repo_id: i64) -> Result<Vec<Repository>, String> {
    db.peers_of_repo(repo_id).map_err(|e| e.to_string())
}

// ── Workspace scanner ─────────────────────────────────────────────────────────

/// Workspace scanner: walk `workspace_root` two levels deep (skipping
/// dot-folders, `node_modules` and `target`) and map every requested GitHub
/// name to the local folder whose git remotes fetch from it. Needs a git
/// binary on PATH — every candidate repo's remotes are read by `git config`
/// itself (see `remote_fetch_urls`); without git the scan is an `Err`, never
/// a silent empty result.
///
/// Async like the untrack/clone commands above: every candidate repo costs a
/// git subprocess (a single git spawn measured 0.7–7.8 s on the owner's
/// Windows machine), and as a synchronous command that whole sequential wait
/// was spent on Tauri's UI thread with the window frozen.
#[tauri::command(async)]
pub fn scan_workspace_for_repos(
    workspace_root: String,
    github_names: Vec<String>,
) -> Result<HashMap<String, String>, String> {
    scan_workspace_for_repos_impl(workspace_root, github_names)
}

/// The orchestration behind `scan_workspace_for_repos`, on plain owned args
/// (no `State`) so it is unit-testable without a Tauri runtime — same shape
/// as `clone_repo_to_impl`. The git binary comes from the per-process probe
/// (`cached_git`).
fn scan_workspace_for_repos_impl(
    workspace_root: String,
    github_names: Vec<String>,
) -> Result<HashMap<String, String>, String> {
    scan_workspace_for_repos_with_git(git_ops::cached_git(), workspace_root, github_names)
}

/// `scan_workspace_for_repos_impl` with the git binary injected, mirroring
/// `clone_repo_to_impl_with_url`: tests drive the missing-git failure path
/// with `None` instead of needing a git-less environment.
fn scan_workspace_for_repos_with_git(
    git: Option<git_ops::GitBinary>,
    workspace_root: String,
    github_names: Vec<String>,
) -> Result<HashMap<String, String>, String> {
    let root = std::path::Path::new(&workspace_root);
    if !root.is_dir() {
        return Err(format!("Directory not found: {}", workspace_root));
    }

    let mut found: HashMap<String, String> = HashMap::new();

    // R1: the remote URLs are read by git itself (`git config --file ...`),
    // so the scan needs a git binary — one per-process probe per scan via
    // `cached_git`. Without git the URLs cannot be read, and an empty map
    // here would make Settings report "Scan complete: found 0 repositories"
    // as if the scan had run, so the missing binary is an actionable error
    // instead (same stance as `list_gitignored_tracked`'s "git not
    // available").
    let git = git.ok_or_else(|| {
        "git is not available on PATH — the workspace scan needs git to read repository remotes"
            .to_string()
    })?;

    // Scan 2 levels deep for .git/config
    scan_dir(root, &github_names, &mut found, 0, 2, &git);

    Ok(found)
}

/// F-000045: the fetch-URL keys of every remote, as a `git config
/// --get-regexp` pattern. Both spellings of a remote subsection — quoted
/// `[remote "x"]` AND dotted `[remote.x]` — key to `remote.<name>.url`, and
/// git lowercases section and key names (so `[REMOTE "x"]` / `URL =` key the
/// same). The pattern matches every remote's `url` and nothing else:
/// * `remote.<name>.pushurl` ends in `.pushurl`, not `.url`;
/// * a subsection-less `[remote]` keys `remote.url` — no `.` + name in
///   between, so no match (it is not any remote's URL);
/// * `[remoteExtra "x"]` keys `remoteextra.x.url`, which does not start with
///   `remote.` — a different section, not a remote.
const REMOTE_URL_KEY_PATTERN: &str = "^remote\\..*\\.url$";

/// F-000045: decode `git config --null --get-regexp` stdout. With `--null`
/// git emits one `key\nvalue\0` record per match — verified empirically
/// against git 2.52: a LF separates the lowercased key from the raw value,
/// a NUL ends the record, and a trailing NUL leaves one final empty
/// segment. Only the value half is kept — the regexp already selected the
/// key. A value may itself contain LFs (a quoted `"a\nb"` config value), so
/// the FIRST LF of a record is the separator; non-UTF-8 bytes (a path
/// spelling) are kept lossily rather than dropping the URL.
fn remote_urls_from_config_output(stdout: &[u8]) -> Vec<String> {
    stdout
        .split(|b| *b == 0)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            // First LF separates key from value (`<[u8]>::split_once` is
            // still unstable, so split by position).
            record
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| &record[i + 1..])
        })
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .collect()
}

/// F-000045: every fetch URL (`url = ...`) of the remote sections of a
/// `.git/config`, read by GIT'S OWN config parser — never hand-rolled — so
/// every spelling git accepts is honored: quoted `[remote "x"]` AND dotted
/// `[remote.x]` sections, upper-case `[REMOTE]` spellings, a section header
/// and its key on the same line, backslash line continuation, quoted values
/// with escapes, and inline `;`/`#` comments. `pushurl` is skipped — where a
/// clone fetches from is the proof, not where it may push to — and so are
/// comments and look-alike sections (`[remoteExtra "x"]`, bare `[remote]`),
/// all excluded by `REMOTE_URL_KEY_PATTERN`.
///
/// Spawned through `git_ops::spawn_cmd` (CREATE_NO_WINDOW on Windows) with
/// the config path as a single argument — no shell, so spaces in the
/// workspace path are safe. `--null` makes the output NUL-delimited and
/// therefore parseable without heuristics.
///
/// `None` means the listing cannot be trusted — a spawn failure or any exit
/// code other than 0/1 — and the caller treats the directory as having no
/// URLs, exactly like an unreadable config file before. `--get-regexp`
/// exits 1 when nothing matches; that is a plain empty vec, not an error.
fn remote_fetch_urls(git: &git_ops::GitBinary, config_path: &Path) -> Option<Vec<String>> {
    let out = git_ops::spawn_cmd(&git.0)
        .arg("config")
        .arg("--file")
        .arg(config_path)
        .arg("--null")
        .arg("--get-regexp")
        .arg(REMOTE_URL_KEY_PATTERN)
        .output()
        .ok()?;
    match out.status.code() {
        Some(0) => Some(remote_urls_from_config_output(&out.stdout)),
        Some(1) => Some(Vec::new()),
        _ => None,
    }
}

fn scan_dir(
    dir: &std::path::Path,
    github_names: &[String],
    found: &mut HashMap<String, String>,
    depth: usize,
    max_depth: usize,
    git: &git_ops::GitBinary,
) {
    if depth > max_depth {
        return;
    }

    let git_config = dir.join(".git").join("config");
    if git_config.exists() {
        // T-000242 / F-000045: git's own parser reads the config, then the
        // real fetch URLs go through the exact GitHub matcher, which compares
        // owner/repo case-insensitively (the clone keeps the case GitHub gave
        // it) but never treats a commented URL, a `pushurl` or a prefix repo
        // as a match. An untrustworthy listing (spawn/exit failure → None)
        // skips the directory, exactly like an unreadable config file before.
        if let Some(fetch_urls) = remote_fetch_urls(git, &git_config) {
            for name in github_names {
                if fetch_urls
                    .iter()
                    .any(|url| git_ops::remote_matches_github(url, name))
                {
                    found.insert(name.clone(), dir.to_string_lossy().to_string());
                }
            }
        }
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !name_str.starts_with('.') && name_str != "node_modules" && name_str != "target"
                {
                    scan_dir(
                        &entry.path(),
                        github_names,
                        found,
                        depth + 1,
                        max_depth,
                        git,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo_dir(root: &Path, folder: &str, remote: &str) -> PathBuf {
        let dir = root.join(folder);
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(
            dir.join(".git").join("config"),
            format!("[remote \"origin\"]\n\turl = {}\n", remote),
        )
        .unwrap();
        dir
    }

    // ── scanner: the failure paths that need no git spawn ──────────────────

    /// The `_impl`/`_with_git` split exists for these: the command's body is
    /// now just the `#[tauri::command(async)]` delegate, so the orchestration
    /// is tested without a Tauri runtime — same arrangement as the
    /// `clone_repo_to_impl` tests below.
    #[test]
    fn test_scan_workspace_impl_rejects_a_missing_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing = tmp.path().join("no-such-dir");
        let err = scan_workspace_for_repos_impl(
            missing.to_string_lossy().to_string(),
            vec!["owner/repo".to_string()],
        )
        .unwrap_err();
        assert!(err.starts_with("Directory not found: "), "{}", err);
    }

    /// MINOR 3: a missing git binary is an actionable error, never a silent
    /// empty map that Settings would toast as "Scan complete: found 0
    /// repositories". Driven through the binary-injecting variant with `None`
    /// — no git-less environment needed.
    #[test]
    fn test_scan_workspace_without_git_is_an_error_not_an_empty_map() {
        let tmp = tempfile::TempDir::new().unwrap();
        let err = scan_workspace_for_repos_with_git(
            None,
            tmp.path().to_string_lossy().to_string(),
            vec!["owner/repo".to_string()],
        )
        .unwrap_err();
        assert!(err.contains("git"), "must name git: {}", err);
    }

    // R1: the scanner reads each config through `git config` now, so every
    // scanner test spawns git — ignored like the other git-spawning tests.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn test_scan_matches_remote_url_regardless_of_case() {
        // T-000242: on Linux `Server_Airouter` and `server_airouter` are
        // different folders; the remote URL keeps the clone's case while
        // github_name keeps GitHub's — the match must not care.
        let tmp = tempfile::TempDir::new().unwrap();
        repo_dir(
            tmp.path(),
            "server_airouter",
            "git@github.com:owner/Server_Airouter.git",
        );
        repo_dir(tmp.path(), "other", "https://github.com/Owner/Other_Repo");

        let found = scan_workspace_for_repos_impl(
            tmp.path().to_string_lossy().to_string(),
            vec![
                "owner/server_airouter".to_string(),
                "owner/other_repo".to_string(),
            ],
        )
        .unwrap();

        assert_eq!(found.len(), 2, "{:?}", found);
        assert!(found["owner/server_airouter"].ends_with("server_airouter"));
        assert!(found["owner/other_repo"].ends_with("other"));
    }

    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn test_scan_does_not_match_a_different_repo_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        repo_dir(
            tmp.path(),
            "airouter",
            "git@github.com:owner/server_airouter.git",
        );

        let found = scan_workspace_for_repos_impl(
            tmp.path().to_string_lossy().to_string(),
            vec!["owner/airouter_extra".to_string()],
        )
        .unwrap();

        assert!(found.is_empty(), "{:?}", found);
    }

    /// F-000045: a config file as the old substring scanner would have
    /// mis-matched — a commented URL, a `pushurl`, a `url-other` key and a
    /// prefix repo — none of which proves the folder fetches from the repo.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn test_scan_ignores_commented_pushurl_and_prefix_urls() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("lookalike");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(
            dir.join(".git").join("config"),
            concat!(
                "[core]\n",
                "\trepositoryformatversion = 0\n",
                "# url = git@github.com:owner/repo.git\n",
                "; url = https://github.com/owner/repo.git\n",
                "[remote \"origin\"]\n",
                "\turl = git@github.com:owner/repo-other.git\n",
                "\tpushurl = https://github.com/owner/repo.git\n",
                "\turl-other = https://github.com/owner/repo.git\n",
                "[url \"https://github.com/\"]\n",
                "\tinsteadof = https://example.com/\n",
            ),
        )
        .unwrap();

        let found = scan_workspace_for_repos_impl(
            tmp.path().to_string_lossy().to_string(),
            vec!["owner/repo".to_string()],
        )
        .unwrap();

        assert!(found.is_empty(), "{:?}", found);
    }

    /// F-000045: any remote section proves the match (not just `origin`), and
    /// the depth-2 walk of the old scanner is unchanged.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn test_scan_matches_any_remote_and_nested_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("work").join("repo"); // depth 2
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(
            dir.join(".git").join("config"),
            // The GitHub remote is `upstream`, origin points elsewhere.
            "[remote \"origin\"]\n\turl = /local/backup.git\n\
             [remote \"upstream\"]\n\turl = https://github.com/Owner/Repo.git\n",
        )
        .unwrap();

        let found = scan_workspace_for_repos_impl(
            tmp.path().to_string_lossy().to_string(),
            vec!["owner/repo".to_string()],
        )
        .unwrap();

        assert_eq!(found.len(), 1, "{:?}", found);
        assert!(found["owner/repo"].ends_with("repo"));
    }

    /// R1: `git config --null --get-regexp` emits one `key\nvalue\0` record
    /// per match (the format was verified empirically against git 2.52).
    /// Pure parser — no git is spawned, so no `#[ignore]`.
    #[test]
    fn test_remote_urls_from_config_output_parses_the_null_format() {
        // Two records with a trailing NUL: only the value halves survive, in
        // order, with their original case.
        assert_eq!(
            remote_urls_from_config_output(
                b"remote.origin.url\nhttps://github.com/o/r.git\0\
                  remote.up.url\ngit@github.com:o/U.git\0"
            ),
            vec![
                "https://github.com/o/r.git".to_string(),
                "git@github.com:o/U.git".to_string(),
            ]
        );
        // A value may contain LFs, quotes, backslashes and CRs: only the
        // FIRST LF of a record separates the key from the value.
        assert_eq!(
            remote_urls_from_config_output(b"remote.nl.url\nline1\nline\"2\\x\r\n\0"),
            vec!["line1\nline\"2\\x\r\n".to_string()]
        );
        // An empty value is still a record (`url =`).
        assert_eq!(
            remote_urls_from_config_output(b"remote.e.url\n\0"),
            vec![String::new()]
        );
        // Empty stdout, a bare trailing NUL and a record without the
        // key/value LF all yield nothing rather than garbage.
        assert!(remote_urls_from_config_output(b"").is_empty());
        assert!(remote_urls_from_config_output(b"\0").is_empty());
        assert!(remote_urls_from_config_output(b"remote.only-key\0").is_empty());
    }

    /// R1: the scanner reads `.git/config` through git itself, so EVERY
    /// spelling git accepts is a match — dotted `[remote.origin]`, quoted
    /// subsections, upper-case sections, quoted values with inline comments,
    /// a header and its key on one line, backslash line continuation — and
    /// every look-alike is NOT: a `pushurl` decoy, `[remoteExtra "x"]`, a
    /// bare `[remote]` and commented `url` lines. One config, one scan: each
    /// spelling names a different repo, so each must be found on its own.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn test_scan_reads_remote_urls_through_gits_own_parser() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("spellings");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(
            dir.join(".git").join("config"),
            // Raw string: the continuation line's trailing backslash is a
            // literal character, exactly as git reads it.
            r#"[core]
	repositoryformatversion = 0
[remote "q"]
	url = https://github.com/Owner/Quoted.git
[REMOTE "UP"]
	url = "https://github.com/owner/Uppercase.git" ; quoted value + inline comment
[remote.origin]
	url = git@github.com:owner/Dotted.git
[remote "same"] url = https://github.com/owner/SameLine.git # header + key on one line
[remote "cont"]
	url = https://github.com/owner/Contin\
uation.git
[remote "decoy"]
	pushurl = https://github.com/owner/Decoy.git
[remoteExtra "x"]
	url = https://github.com/owner/Decoy.git
[remote]
	url = https://github.com/owner/Decoy.git
# url = https://github.com/owner/Decoy.git
; url = https://github.com/owner/Decoy.git
"#,
        )
        .unwrap();

        let found = scan_workspace_for_repos_impl(
            tmp.path().to_string_lossy().to_string(),
            vec![
                "owner/quoted".to_string(),
                "owner/uppercase".to_string(),
                "owner/dotted".to_string(),
                "owner/sameline".to_string(),
                "owner/continuation".to_string(),
                "owner/decoy".to_string(),
            ],
        )
        .unwrap();

        for name in [
            "owner/quoted",
            "owner/uppercase",
            "owner/dotted",
            "owner/sameline",
            "owner/continuation",
        ] {
            assert!(found.contains_key(name), "{name} missing: {found:?}");
            assert!(found[name].ends_with("spellings"), "{name}: {found:?}");
        }
        assert!(
            !found.contains_key("owner/decoy"),
            "pushurl / remoteExtra / bare [remote] / commented url must not match: {found:?}"
        );
    }

    // ── F-000045: clone_repo_to — the failure paths that need no git ─────────

    fn make_db() -> AppDb {
        AppDb::new(std::path::PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn test_clone_repo_to_rejects_early_bad_inputs() {
        let db = make_db();

        // Unknown repo id: a CloneFailed issue naming the id.
        let err = clone_repo_to_impl(&db, 99999, "/abs/target".into()).unwrap_err();
        assert_eq!(err.code, SyncIssueCode::CloneFailed);
        assert!(err.subject.contains("99999"));

        // Local-only repo (no github_name): nothing to clone.
        let local = db
            .insert_local_repository("/tmp/local", "My Folder", None, None)
            .unwrap();
        let err = clone_repo_to_impl(&db, local.id, "/abs/target".into()).unwrap_err();
        assert_eq!(err.code, SyncIssueCode::CloneTargetInvalid);
        assert_eq!(err.target.as_deref(), Some("/abs/target"));
        assert!(err.detail.as_deref().unwrap_or("").contains("owner/repo"));

        // A github_name that is not `owner/repo` (URL, 3 segments) is refused.
        for bad in ["github.com/owner/repo", "owner/repo/extra", "owner/"] {
            let repo = db
                .upsert_repository(bad, None, None, None, None, None)
                .unwrap();
            let err = clone_repo_to_impl(&db, repo.id, "/abs/target".into()).unwrap_err();
            assert_eq!(err.code, SyncIssueCode::CloneTargetInvalid, "{}", bad);
        }

        // Already-bound repo: the clone command never overwrites a binding.
        let bound = db
            .upsert_repository("owner/bound", None, None, None, None, None)
            .unwrap();
        db.set_repo_local_path(bound.id, Some("/already/here"))
            .unwrap();
        let err = clone_repo_to_impl(&db, bound.id, "/abs/target".into()).unwrap_err();
        assert_eq!(err.code, SyncIssueCode::CloneTargetInvalid);
        assert!(err
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("already has a local folder"));

        // Relative / empty target folder.
        let plain = db
            .upsert_repository("owner/plain", None, None, None, None, None)
            .unwrap();
        for target in ["", "relative/path"] {
            let err = clone_repo_to_impl(&db, plain.id, target.to_string()).unwrap_err();
            assert_eq!(err.code, SyncIssueCode::CloneTargetInvalid, "{:?}", target);
            assert_eq!(err.target.as_deref(), Some(target));
        }
    }

    #[test]
    fn test_clone_repo_to_rejects_a_garbage_configured_branch() {
        let db = make_db();
        let repo = db
            .upsert_repository("owner/repo", None, None, None, None, None)
            .unwrap();
        db.set_autocommit_branch(repo.id, Some("a..b")).unwrap();

        // An absolute target that does not exist yet: the branch name is
        // rejected before git is even looked up, so nothing is created.
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("clone-target");
        let err =
            clone_repo_to_impl(&db, repo.id, target.to_string_lossy().into_owned()).unwrap_err();
        assert_eq!(err.code, SyncIssueCode::CloneFailed);
        assert!(err
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("invalid branch name"));
        // The setting itself is untouched.
        assert_eq!(
            db.get_autocommit_branch(repo.id).unwrap(),
            Some("a..b".to_string())
        );
        assert_eq!(db.get_repository(repo.id).unwrap().local_path, None);
    }

    /// F-000045 review: `is_github_owner_repo` with an explicit per-component
    /// policy — fragments, traversal specials, backslashes and embedded
    /// whitespace must all be refused before anything is cloned.
    #[test]
    fn test_is_github_owner_repo_shape() {
        for good in [
            "owner/repo",
            "Owner/Repo_1.2-x",
            "o.r/re.po",
            "  owner/repo  ",
        ] {
            assert!(is_github_owner_repo(good), "{good:?} should be valid");
        }
        for bad in [
            "",
            "repo",
            "owner/",
            "/repo",
            "a/b/c",
            "owner/repo#fragment",
            "owner/..",
            "../repo",
            ".",
            "..",
            "owner/re po",
            "owner/re\tpo",
            "owner/re?po",
            "owner/re@po",
            "owner/:repo",
            "owner\\repo",
            "https://github.com/owner/repo",
            "git@github.com:owner/repo.git",
            "owner/\u{1}repo",
        ] {
            assert!(!is_github_owner_repo(bad), "{bad:?} should be invalid");
        }
    }

    // ── F-000045 review: clone_repo_to against a real git remote ────────────
    // Minimal local fixtures (git CLI + TempDir + in-memory AppDb). git_sync's
    // test module is private — its helpers are deliberately replicated here
    // in the smallest shape the command needs.

    /// Run git, assert success, return trimmed stdout.
    fn g(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
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

    /// A bare remote whose HEAD is `default`, plus one commit on every name in
    /// `branches`, pushed from a throw-away seed clone.
    fn bare_remote(
        tmp: &tempfile::TempDir,
        name: &str,
        default: &str,
        branches: &[&str],
    ) -> PathBuf {
        let remote = tmp.path().join(name);
        fs::create_dir(&remote).unwrap();
        g(&remote, &["init", "--bare", "-q"]);
        let head = format!("refs/heads/{default}");
        g(&remote, &["symbolic-ref", "HEAD", head.as_str()]);
        let seed = tmp.path().join(format!("{name}-seed"));
        fs::create_dir(&seed).unwrap();
        g(&seed, &["init", "-q"]);
        g(&seed, &["symbolic-ref", "HEAD", head.as_str()]);
        for (key, val) in [
            ("user.email", "repo@example.com"),
            ("user.name", "Repo User"),
            ("commit.gpgsign", "false"),
        ] {
            g(&seed, &["config", key, val]);
        }
        g(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        fs::write(seed.join("README.md"), "hello\n").unwrap();
        g(&seed, &["add", "."]);
        g(&seed, &["commit", "-q", "-m", "init"]);
        let default_ref = format!("HEAD:refs/heads/{default}");
        g(&seed, &["push", "-q", "origin", default_ref.as_str()]);
        for b in branches {
            let spec = format!("HEAD:refs/heads/{b}");
            g(&seed, &["push", "-q", "origin", spec.as_str()]);
        }
        remote
    }

    fn untracked_repo(db: &AppDb, github_name: &str) -> i64 {
        let repo = db
            .upsert_repository(github_name, None, None, None, None, None)
            .unwrap();
        assert_eq!(repo.local_path, None);
        repo.id
    }

    /// (a) The happy path: fresh clone → checkout of the default branch
    /// (nothing configured) → atomic bind, and no marker left behind.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn clone_repo_to_clones_checks_out_and_binds() {
        let db = make_db();
        let tmp = tempfile::TempDir::new().unwrap();
        let remote = bare_remote(&tmp, "remote.git", "dev", &["shared"]);
        let url = remote.to_string_lossy().into_owned();
        let id = untracked_repo(&db, "owner/repo");
        let target = tmp.path().join("t");
        let target_str = target.to_string_lossy().into_owned();

        let bound = clone_repo_to_impl_with_url(&db, id, target_str.clone(), Some(&url)).unwrap();
        assert_eq!(bound.local_path.as_deref(), Some(target_str.as_str()));
        assert_eq!(
            db.get_repository(id).unwrap().local_path.as_deref(),
            Some(target_str.as_str()),
            "the DB row is bound"
        );
        assert_eq!(g(&target, &["branch", "--show-current"]), "dev");
        assert!(
            !git_sync::clone_incomplete_marker(&target)
                .expect("target has a parent")
                .exists(),
            "a successful preparation leaves no marker"
        );
    }

    /// (b) A configured `autocommit_branch` wins over the remote default.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn clone_repo_to_prefers_the_configured_branch_over_dev() {
        let db = make_db();
        let tmp = tempfile::TempDir::new().unwrap();
        let remote = bare_remote(&tmp, "remote.git", "dev", &["shared"]);
        let url = remote.to_string_lossy().into_owned();
        let id = untracked_repo(&db, "owner/repo");
        db.set_autocommit_branch(id, Some("shared")).unwrap();
        let target = tmp.path().join("t");
        let target_str = target.to_string_lossy().into_owned();

        let bound = clone_repo_to_impl_with_url(&db, id, target_str, Some(&url)).unwrap();
        assert_eq!(
            bound.local_path.as_deref(),
            Some(target.to_string_lossy().as_ref())
        );
        assert_eq!(g(&target, &["branch", "--show-current"]), "shared");
        assert_eq!(g(&target, &["config", "branch.shared.remote"]), "origin");
    }

    /// (c) A requested branch that is absent on the remote: the clone is
    /// bound, HEAD stays on the remote default, and nothing is created.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn clone_repo_to_with_absent_branch_binds_on_the_default() {
        let db = make_db();
        let tmp = tempfile::TempDir::new().unwrap();
        let remote = bare_remote(&tmp, "remote.git", "main", &[]);
        let url = remote.to_string_lossy().into_owned();
        let id = untracked_repo(&db, "owner/repo");
        db.set_autocommit_branch(id, Some("staging")).unwrap();
        let target = tmp.path().join("t");
        let target_str = target.to_string_lossy().into_owned();

        let bound = clone_repo_to_impl_with_url(&db, id, target_str.clone(), Some(&url)).unwrap();
        assert_eq!(bound.local_path.as_deref(), Some(target_str.as_str()));
        assert_eq!(g(&target, &["branch", "--show-current"]), "main");
        assert!(
            g(&target, &["branch", "--list", "staging"]).is_empty(),
            "the configured branch is never created from nothing"
        );
    }

    /// (d) A clone/network failure: nothing is bound, the marker stays in the
    /// parent folder, and a second attempt at the same target is refused
    /// naming the leftover — even though the remote is fine by then.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn clone_repo_to_failure_leaves_no_binding_but_a_marker() {
        let db = make_db();
        let tmp = tempfile::TempDir::new().unwrap();
        let remote = bare_remote(&tmp, "remote.git", "dev", &[]);
        let good_url = remote.to_string_lossy().into_owned();
        let dead_url = tmp.path().join("nope.git").to_string_lossy().into_owned();
        let id = untracked_repo(&db, "owner/repo");
        let target = tmp.path().join("t");
        let target_str = target.to_string_lossy().into_owned();

        let err =
            clone_repo_to_impl_with_url(&db, id, target_str.clone(), Some(&dead_url)).unwrap_err();
        assert_eq!(err.code, SyncIssueCode::CloneFailed, "{}", err);
        assert_eq!(
            db.get_repository(id).unwrap().local_path,
            None,
            "a failed clone is never bound"
        );
        let marker = git_sync::clone_incomplete_marker(&target).unwrap();
        assert!(marker.exists(), "the marker must outlive the failure");

        // Second attempt at the same target: refused naming the leftover.
        let err =
            clone_repo_to_impl_with_url(&db, id, target_str.clone(), Some(&good_url)).unwrap_err();
        assert_eq!(err.code, SyncIssueCode::CloneTargetInvalid, "{}", err);
        assert_eq!(err.target.as_deref(), Some(target_str.as_str()));
        let detail = err.detail.unwrap_or_default();
        assert!(
            detail.contains("unfinished clone") && detail.contains(".sdh-clone-incomplete-"),
            "{}",
            detail
        );
        assert!(marker.exists(), "the refusal deletes nothing");
        assert_eq!(db.get_repository(id).unwrap().local_path, None);
    }

    /// (e) A folder that already holds a clone of the same repo: bound as-is,
    /// nothing re-cloned, the worktree (branch, HEAD, local edit) untouched.
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    #[test]
    fn clone_repo_to_attaches_an_existing_clone_without_cloning() {
        let db = make_db();
        let tmp = tempfile::TempDir::new().unwrap();
        let remote = bare_remote(&tmp, "remote.git", "dev", &["shared"]);
        // A URL that can never be cloned from: any attempt to clone fails, so
        // success proves the ExistingClone path was taken.
        let dead_url = tmp.path().join("nope.git").to_string_lossy().into_owned();
        let id = untracked_repo(&db, "owner/repo");
        let target = tmp.path().join("t");
        let target_str = target.to_string_lossy().into_owned();

        g(
            tmp.path(),
            &[
                "clone",
                "-q",
                remote.to_str().unwrap(),
                target.to_str().unwrap(),
            ],
        );
        g(
            &target,
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/owner/repo.git",
            ],
        );
        g(&target, &["checkout", "-q", "shared"]);
        fs::write(target.join("README.md"), "local edit\n").unwrap();
        let head = g(&target, &["rev-parse", "HEAD"]);

        let bound = clone_repo_to_impl_with_url(&db, id, target_str.clone(), Some(&dead_url))
            .expect("an existing clone is adoptable without any clone attempt");
        assert_eq!(bound.local_path.as_deref(), Some(target_str.as_str()));
        assert_eq!(g(&target, &["rev-parse", "--abbrev-ref", "HEAD"]), "shared");
        assert_eq!(g(&target, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            fs::read_to_string(target.join("README.md")).unwrap(),
            "local edit\n"
        );
        assert!(g(&target, &["status", "--porcelain"]).contains("README.md"));
    }
}
