use crate::db::AppDb;
use crate::models::*;
use crate::{git_ops, sync};
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
// Sync commands — `git_ops::*` are sync (subprocess output() blocks) and finish
// in well under 500ms on realistic repos; an async wrapper would not pay for
// itself. UI gates the Untrack button on `check_git_available_for_repo` so the
// two listing/untracking commands are only called when both binary and `.git/`
// are known to exist.

/// True iff a git binary is discoverable AND the repo has a `local_path`
/// pointing at something that looks like a git work tree. Lookup failure
/// (no such repo, no local_path) returns Ok(false) so the UI quietly hides
/// the Untrack button — symmetric with the missing-local-path UX elsewhere.
#[tauri::command]
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
    Ok(git_ops::check_git_available().is_some() && git_ops::is_git_repo(path))
}

/// Read `git ls-files -ci --exclude-standard -z` plus repo-state and
/// other-staged count in one call so the dialog has everything it needs on
/// open. Callers should have already gated on `check_git_available_for_repo`,
/// but we re-validate locally (errors here are surfaced to the dialog's error
/// state — see UntrackGitignoredDialog).
#[tauri::command]
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

    let git = git_ops::check_git_available().ok_or_else(|| "git not available".to_string())?;

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
#[tauri::command]
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

    let git = git_ops::check_git_available().ok_or_else(|| "git not available".to_string())?;

    let file_bufs: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
    git_ops::untrack_files(&git, path, &file_bufs)
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
            for name in github_names {
                // Match against common remote URL patterns
                let patterns = [
                    format!("github.com/{}.git", name),
                    format!("github.com/{}", name),
                    format!("github.com:{}.git", name),
                    format!("github.com:{}", name),
                ];
                for pat in &patterns {
                    if content.contains(pat) {
                        found.insert(name.clone(), dir.to_string_lossy().to_string());
                        break;
                    }
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
