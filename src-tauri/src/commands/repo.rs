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

#[tauri::command]
pub fn set_repo_local_path(
    db: State<AppDb>,
    id: i64,
    local_path: Option<String>,
) -> Result<Repository, String> {
    db.set_repo_local_path(id, local_path.as_deref())
        .map_err(|e| e.to_string())
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

/// F-000045: `owner/repo` shape for a `github_name` — exactly one `/`, both
/// halves non-empty, and no URL / userinfo / query noise (a full URL or
/// `a/b/c` can never name a GitHub repository, so there is nothing to clone).
fn is_github_owner_repo(name: &str) -> bool {
    let name = name.trim();
    match name.split_once('/') {
        Some((owner, repo)) => {
            !owner.is_empty()
                && !repo.is_empty()
                && !repo.contains('/')
                && !name.contains("://")
                && !name.contains('@')
                && !name.contains('?')
        }
        None => false,
    }
}

/// The orchestration behind `clone_repo_to`, on a plain `&AppDb` so the
/// no-git failure paths are unit-testable without a Tauri runtime.
fn clone_repo_to_impl(db: &AppDb, id: i64, target_dir: String) -> Result<Repository, SyncIssue> {
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
    let url = git_sync::github_clone_url(&github_name);

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
                SyncIssue::new(SyncIssueCode::CloneFailed, subject).detail(net_err.message)
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

#[tauri::command]
pub fn scan_workspace_for_repos(
    workspace_root: String,
    github_names: Vec<String>,
) -> Result<HashMap<String, String>, String> {
    let root = std::path::Path::new(&workspace_root);
    if !root.is_dir() {
        return Err(format!("Directory not found: {}", workspace_root));
    }

    let mut found: HashMap<String, String> = HashMap::new();

    // Scan 2 levels deep for .git/config
    scan_dir(root, &github_names, &mut found, 0, 2);

    Ok(found)
}

/// F-000045: every fetch URL (`url = ...`) of the `[remote "..."]` sections of
/// a `.git/config`. `pushurl` is skipped — where a clone fetches from is the
/// proof, not where it may push to — and so are comments and any `url`-like
/// key (`url-other`) or key outside a remote section. The old scanner
/// lowercased the whole file and searched substrings, which also matched
/// commented URLs, `pushurl` lines and prefix repos; this parser hands
/// `git_ops::remote_matches_github` the real fetch URLs only.
fn remote_fetch_urls(config: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut in_remote_section = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[') {
            in_remote_section = section.trim_start().starts_with("remote");
            continue;
        }
        if !in_remote_section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case("url") {
                urls.push(value.trim().to_string());
            }
        }
    }
    urls
}

fn scan_dir(
    dir: &std::path::Path,
    github_names: &[String],
    found: &mut HashMap<String, String>,
    depth: usize,
    max_depth: usize,
) {
    if depth > max_depth {
        return;
    }

    let git_config = dir.join(".git").join("config");
    if git_config.exists() {
        if let Ok(content) = std::fs::read_to_string(&git_config) {
            // T-000242 / F-000045: the real fetch URLs go through the exact
            // GitHub matcher, which compares owner/repo case-insensitively
            // (the clone keeps the case GitHub gave it) but no longer treats
            // a commented URL, a `pushurl` or a prefix repo as a match.
            let fetch_urls = remote_fetch_urls(&content);
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
                    scan_dir(&entry.path(), github_names, found, depth + 1, max_depth);
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

        let found = scan_workspace_for_repos(
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

    #[test]
    fn test_scan_does_not_match_a_different_repo_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        repo_dir(
            tmp.path(),
            "airouter",
            "git@github.com:owner/server_airouter.git",
        );

        let found = scan_workspace_for_repos(
            tmp.path().to_string_lossy().to_string(),
            vec!["owner/airouter_extra".to_string()],
        )
        .unwrap();

        assert!(found.is_empty(), "{:?}", found);
    }

    /// F-000045: a config file as the old substring scanner would have
    /// mis-matched — a commented URL, a `pushurl`, a `url-other` key and a
    /// prefix repo — none of which proves the folder fetches from the repo.
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

        let found = scan_workspace_for_repos(
            tmp.path().to_string_lossy().to_string(),
            vec!["owner/repo".to_string()],
        )
        .unwrap();

        assert!(found.is_empty(), "{:?}", found);
    }

    /// F-000045: any remote section proves the match (not just `origin`), and
    /// the depth-2 walk of the old scanner is unchanged.
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

        let found = scan_workspace_for_repos(
            tmp.path().to_string_lossy().to_string(),
            vec!["owner/repo".to_string()],
        )
        .unwrap();

        assert_eq!(found.len(), 1, "{:?}", found);
        assert!(found["owner/repo"].ends_with("repo"));
    }

    #[test]
    fn test_remote_fetch_urls_parses_only_remote_url_keys() {
        let config = "[core]\n\turl = not-a-remote\n\
                      [remote \"origin\"]\n\turl = https://github.com/o/r.git\n\tpushurl = ssh://git@github.com/o/r.git\n\
                      [remote \"second\"]\n\tURL = git@github.com:o/r2.git\n\
                      [branch \"dev\"]\n\tremote = origin\n";
        assert_eq!(
            remote_fetch_urls(config),
            vec![
                "https://github.com/o/r.git".to_string(),
                "git@github.com:o/r2.git".to_string(),
            ]
        );
        assert!(remote_fetch_urls("").is_empty());
        assert!(remote_fetch_urls("# url = x\n; [remote]\nurl = y").is_empty());
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
        let err = clone_repo_to_impl(&db, repo.id, target.to_string_lossy().into_owned())
            .unwrap_err();
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
}
