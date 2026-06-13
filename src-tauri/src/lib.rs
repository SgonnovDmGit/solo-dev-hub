mod crypto;
mod db;
mod export;
mod git_ops;
mod keyring_store;
mod models;
mod sync;
mod template_meta;
mod template_render;
mod template_seeder;

#[allow(unused_imports)]
use chrono;
use db::AppDb;
use models::{
    BugView, CreateDeployEnvironmentArgs, DailyFlowDay, DashboardData, DashboardFilter,
    DeployEnvironment, DeployReportRow, DeploySecret, FileBugNote, GitignoredListing, KpiCard,
    MetaSecretHint, MigrationReport, Project, ProjectGraph, ReadBugsResult, ReadDoneResult,
    ReadTodoResult, RenderedFile, RepoRename, Repository, RequirementInfo, SecretBundle,
    SecretBundleItemValue, StatsSummary, SyncResult, TemplateFile, TemplateLanguage,
    UntrackReport, UpdateDeployEnvironmentArgs, UpsertRepoOutcome, WriteError, WriteResult,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::State;

fn get_db_path() -> PathBuf {
    let local = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let new_dir = local.join("solo-dev-hub");
    let new_db = new_dir.join("data.db");

    // T-000063: one-time copy-once migration from the legacy app dir.
    // We copy (not move) so that if the new build crashes mid-migration the
    // legacy SQLite stays intact as a recovery breadcrumb. The user can
    // delete the legacy folder manually once they're satisfied the rebrand
    // build works. Idempotent: only fires when new doesn't exist yet.
    if !new_db.exists() {
        let legacy_db = local.join("github-repo-manager").join("data.db");
        if legacy_db.exists() {
            std::fs::create_dir_all(&new_dir).ok();
            if let Err(e) = std::fs::copy(&legacy_db, &new_db) {
                eprintln!(
                    "warn: failed to migrate legacy DB {:?} → {:?}: {}",
                    legacy_db, new_db, e
                );
            }
        }
    }

    std::fs::create_dir_all(&new_dir).ok();
    new_db
}

// ── Project commands ──────────────────────────────────────────────────────────

#[tauri::command]
fn create_project(
    db: State<AppDb>,
    name: String,
    description: Option<String>,
    project_type: String,
) -> Result<Project, String> {
    if project_type != "standard" && project_type != "microservice" {
        return Err(format!("Invalid project_type: {}", project_type));
    }
    db.create_project(&name, description.as_deref(), &project_type)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_projects(db: State<AppDb>) -> Result<Vec<Project>, String> {
    db.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
fn update_project(
    db: State<AppDb>,
    id: i64,
    name: String,
    description: Option<String>,
) -> Result<Project, String> {
    db.update_project(id, &name, description.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_project(db: State<AppDb>, id: i64) -> Result<(), String> {
    db.delete_project(id).map_err(|e| e.to_string())
}

// ── Repository commands ───────────────────────────────────────────────────────

#[tauri::command]
fn create_local_repository(
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
fn upsert_repository(
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
fn resolve_merge_with_local(
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
fn force_insert_github_repo(
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
fn assign_repository(
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
fn reorder_project(db: State<AppDb>, id: i64, direction: String) -> Result<(), String> {
    db.reorder_project(id, &direction)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reorder_repo(db: State<AppDb>, repo_id: i64, direction: String) -> Result<(), String> {
    db.reorder_repo(repo_id, &direction)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn rebalance_repo_group(db: State<AppDb>, ordered_ids: Vec<i64>) -> Result<(), String> {
    db.rebalance_repo_group(&ordered_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn rebalance_projects(db: State<AppDb>, ordered_ids: Vec<i64>) -> Result<(), String> {
    db.rebalance_projects(&ordered_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn auto_sort_all(db: State<AppDb>) -> Result<(), String> {
    db.auto_sort_all().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_repos_by_project(
    db: State<AppDb>,
    project_id: Option<i64>,
) -> Result<Vec<Repository>, String> {
    db.list_repos_by_project(project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_all_repos(db: State<AppDb>) -> Result<Vec<Repository>, String> {
    db.list_all_repos().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_repository(db: State<AppDb>, id: i64) -> Result<Repository, String> {
    db.get_repository(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_repository_by_name(db: State<AppDb>, github_name: String) -> Result<Repository, String> {
    db.get_repository_by_name(&github_name)
        .map_err(|e| e.to_string())
}

// ── PAT / Keyring commands ────────────────────────────────────────────────────

#[tauri::command]
fn store_pat(token: String) -> Result<(), String> {
    keyring_store::store_pat(&token)
}

#[tauri::command]
fn get_pat() -> Result<Option<String>, String> {
    keyring_store::get_pat()
}

#[tauri::command]
fn delete_pat() -> Result<(), String> {
    keyring_store::delete_pat()
}

// ── local_path command ────────────────────────────────────────────────────────

#[tauri::command]
fn set_repo_local_path(
    db: State<AppDb>,
    id: i64,
    local_path: Option<String>,
) -> Result<Repository, String> {
    db.set_repo_local_path(id, local_path.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_repo_description(
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
fn delete_repository(
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
fn check_git_available_for_repo(db: State<AppDb>, repository_id: i64) -> Result<bool, String> {
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
fn list_gitignored_tracked(
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
fn untrack_files(
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
fn scan_workspace_for_repos(
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

// ── File-based bug read/write ─────────────────────────────────────────────────

#[tauri::command]
fn read_bugs_from_file(file_path: String) -> Result<ReadBugsResult, String> {
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Ok(ReadBugsResult {
            bugs: vec![],
            warnings: vec![],
        });
    }
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;

    // Detect format: legacy starts with "# Bug List:", new format does not
    if content.trim_start().starts_with("# Bug List:") {
        // Legacy format — parse with old parser, map to new FileBugNote fields
        let parsed = export::parse_markdown_legacy(&content)
            .ok_or_else(|| "Failed to parse legacy markdown header".to_string())?;

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut counter = 0u32;
        let bugs: Vec<FileBugNote> = parsed
            .bugs
            .iter()
            .map(|b| {
                counter += 1;
                let severity = match b.priority.as_str() {
                    "high" => "major",
                    "critical" => "critical",
                    _ => "minor", // low, medium → minor
                };
                let status = if b.is_resolved { "confirmed" } else { "open" };
                // F-026 v2 format: no screen/reproduction fields.
                // Merge legacy `description` (body text) into the single description field.
                let description = match &b.description {
                    Some(body) if !body.is_empty() => format!("{}\n\n{}", b.title, body),
                    _ => b.title.clone(),
                };
                FileBugNote {
                    id: format!("B-{:06}", counter),
                    date: b.created_at.clone().unwrap_or_else(|| today.clone()),
                    description,
                    severity: severity.to_string(),
                    category: b.category.clone(),
                    status: status.to_string(),
                    fix_attempts: b.fix_attempts,
                    comment: None,
                }
            })
            .collect();

        let mut warnings: Vec<String> = vec![];
        if parsed.skipped_lines > 0 {
            warnings.push(format!(
                "{} line(s) could not be parsed",
                parsed.skipped_lines
            ));
        }

        Ok(ReadBugsResult { bugs, warnings })
    } else {
        // New format
        let (bugs, warnings) = export::parse_bug_reports(&content);
        Ok(ReadBugsResult { bugs, warnings })
    }
}

#[tauri::command]
fn write_bugs_to_file(
    file_path: String,
    repo_root: String,
    bugs: Vec<FileBugNote>,
) -> Result<(), String> {
    // B-001: guard against writing into a moved/deleted repo folder.
    sync::ensure_root_exists(std::path::Path::new(&repo_root))?;
    let path = std::path::Path::new(&file_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let md = export::generate_bug_reports(&bugs);
    std::fs::write(path, md).map_err(|e| e.to_string())?;
    Ok(())
}

// ── Microservice connection commands (F-012) ─────────────────────────────────

#[tauri::command]
fn connect_microservice(
    db: State<AppDb>,
    project_id: i64,
    microservice_project_id: i64,
) -> Result<(), String> {
    db.connect_microservice(project_id, microservice_project_id)
}

#[tauri::command]
fn disconnect_microservice(
    db: State<AppDb>,
    project_id: i64,
    microservice_project_id: i64,
) -> Result<(), String> {
    db.disconnect_microservice(project_id, microservice_project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_project_microservices(db: State<AppDb>, project_id: i64) -> Result<Vec<i64>, String> {
    db.list_project_microservices(project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_microservice_projects(db: State<AppDb>) -> Result<Vec<Project>, String> {
    db.list_microservice_projects().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_parents_of_microservice(
    db: State<AppDb>,
    ms_project_id: i64,
) -> Result<Vec<Project>, String> {
    db.list_parents_of_microservice(ms_project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_project_type(db: State<AppDb>, id: i64, new_type: String) -> Result<Project, String> {
    db.update_project_type(id, &new_type)
}

#[tauri::command]
fn server_repo_of_microservice(db: State<AppDb>, ms_project_id: i64) -> Result<Repository, String> {
    db.server_repo_of_microservice(ms_project_id)
}

// ── Settings commands ─────────────────────────────────────────────────────────

#[tauri::command]
fn get_setting(db: State<AppDb>, key: String) -> Result<Option<String>, String> {
    db.get_setting(&key).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_setting(db: State<AppDb>, key: String, value: String) -> Result<(), String> {
    db.set_setting(&key, &value).map_err(|e| e.to_string())
}

// ── Bugs (v0.16.0, SQLite SoT) ───────────────────────────────────────────────

/// Idempotent lazy MD→DB migration for a repo. Call BEFORE reconcile.
/// Returns report with imported/archived counts; `already=true` means it
/// was already migrated on a prior call (no-op).
#[tauri::command]
fn ensure_bugs_migrated(db: State<AppDb>, repo_id: i64) -> Result<MigrationReport, String> {
    sync::migrate_bugs_for_repo(&db, repo_id)
}

/// 2-way sync MD ↔ DB: ingest LLM-edited status/comment from MD, silently
/// correct protected-field mismatches and restore deleted rows via regen.
/// Caller must have migrated the repo first — returns Err otherwise.
#[tauri::command]
fn reconcile_bugs_for_repo(db: State<AppDb>, repo_id: i64) -> Result<(), String> {
    sync::reconcile_bugs_for_repo(&db, repo_id)
}

#[derive(serde::Serialize)]
struct ReconcileAllReport {
    repos_scanned: usize,
    errors: Vec<String>,
}

/// B-000016 (dogfood follow-up): portfolio-wide reconcile for Dashboard ↻.
/// Walks every repo and runs MD→DB reconcile for bugs + tasks. No cross-repo
/// file copies (those live in `sync_project` per-project). "Not migrated yet"
/// errors are suppressed — that's normal for newly added repos before their
/// first ensure_*_migrated call. All other errors are collected so the UI can
/// surface them via toast without aborting the rest of the walk.
#[tauri::command]
fn reconcile_all_projects(db: State<AppDb>) -> Result<ReconcileAllReport, String> {
    let repos = db.list_all_repos().map_err(|e| e.to_string())?;
    let repos_scanned = repos.len();
    let mut errors: Vec<String> = Vec::new();

    for r in repos {
        // Bugs reconcile — silent-skip pre-migration state.
        if let Err(e) = sync::reconcile_bugs_for_repo(&db, r.id) {
            if !e.contains("not migrated") {
                errors.push(format!("Bugs {}: {}", r.display_name(), e));
            }
        }
        // Tasks reconcile — same silent-skip rule. SyncTasksReport.events_emitted
        // is informational; we don't surface it (the user sees the effect in the
        // refreshed Dashboard numbers).
        if let Err(e) = sync::sync_tasks_for_repo(&db, r.id) {
            if !e.contains("not migrated") {
                errors.push(format!("Tasks {}: {}", r.display_name(), e));
            }
        }
    }

    Ok(ReconcileAllReport {
        repos_scanned,
        errors,
    })
}

/// List bugs for a repo as frontend DTOs. `include_confirmed=false` excludes
/// archived bugs (default for BugNotes list view). `=true` includes them
/// (used when user toggles "Показать закрытые").
#[tauri::command]
fn read_bugs_from_db(
    db: State<AppDb>,
    repo_id: i64,
    include_confirmed: bool,
) -> Result<Vec<BugView>, String> {
    let bugs = db
        .list_bugs_by_repo(repo_id, include_confirmed)
        .map_err(|e| e.to_string())?;
    Ok(bugs.iter().map(|b| b.to_view()).collect())
}

/// Count of `status='confirmed'` bugs for the "Показать закрытые (N)" label.
#[tauri::command]
fn count_confirmed_bugs(db: State<AppDb>, repo_id: i64) -> Result<i64, String> {
    db.count_confirmed_bugs(repo_id).map_err(|e| e.to_string())
}

/// Create a new bug via app UI (+ Add button). Starts in `created` status
/// with `fix_attempts=0`. `numeric_id` auto-allocated as max+1 per-repo.
/// Regenerates `docs/bug-reports.md` from DB so the new row is visible to LLM.
#[tauri::command]
fn create_bug(
    db: State<AppDb>,
    repo_id: i64,
    description: String,
    severity: String,
    category: String,
) -> Result<BugView, String> {
    // T-000128: ingest any pending LLM MD edits (status/comment) BEFORE the
    // DB mutation so the final regen doesn't overwrite them with stale DB
    // state. "not migrated" errors are expected on first call and ignored.
    let _ = sync::reconcile_bugs_for_repo(&db, repo_id);
    let nid = db.next_numeric_id(repo_id).map_err(|e| e.to_string())?;
    let now = db::utc_now_rfc3339();
    let bug = db
        .insert_bug(
            repo_id,
            nid,
            &now,
            &description,
            &severity,
            &category,
            "created",
            0,
            None,
            None,
        )
        .map_err(|e| e.to_string())?;
    db.insert_bug_event(bug.id, "created", None, Some("created"), &bug.created_at)
        .map_err(|e| e.to_string())?;
    let _ = sync::regenerate_bugs_md(&db, repo_id);
    Ok(bug.to_view())
}

/// Mark a bug as confirmed (✓ button). Valid from `testing` status only.
/// Sets `confirmed_at` to current UTC. Row stays in DB with status='confirmed';
/// it drops out of MD on regen.
#[tauri::command]
fn resolve_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<BugView, String> {
    // T-000128: reconcile LLM MD edits before reading + mutating.
    let _ = sync::reconcile_bugs_for_repo(&db, repo_id);
    let bug = db
        .get_bug_by_display_id(repo_id, &display_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("bug {} not found", display_id))?;
    if bug.status != "testing" {
        return Err(format!(
            "cannot confirm from status '{}' — must be in 'testing' first",
            bug.status
        ));
    }
    let now = db::utc_now_rfc3339();
    db.update_bug_status(bug.id, "confirmed", None, Some(&now))
        .map_err(|e| e.to_string())?;
    db.insert_bug_event(
        bug.id,
        "confirmed",
        Some("testing"),
        Some("confirmed"),
        &now,
    )
    .map_err(|e| e.to_string())?;
    let refreshed = db
        .get_bug_by_id(bug.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "bug vanished after update".to_string())?;
    let _ = sync::regenerate_bugs_md(&db, repo_id);
    Ok(refreshed.to_view())
}

/// Update user-owned fields on an existing bug. Any of description/severity/
/// category/comment can be updated individually via `Some(_)`; `None` leaves
/// the DB value unchanged. `comment: Some(None)` explicitly clears the field
/// (distinguished from "don't touch comment" which is outer `None`).
/// Always regenerates MD so the new values propagate to the LLM-facing view.
/// Update user-owned fields. Any of the optional args = `None` leaves the DB value
/// unchanged. For `comment`, `Some("")` clears the field (DB NULL); `Some("text")` sets it.
#[tauri::command]
fn update_bug_fields(
    db: State<AppDb>,
    repo_id: i64,
    display_id: String,
    description: Option<String>,
    severity: Option<String>,
    category: Option<String>,
    comment: Option<String>,
) -> Result<BugView, String> {
    // T-000128: reconcile LLM MD edits before user-field update so LLM
    // status/comment edits aren't clobbered by the final regen.
    let _ = sync::reconcile_bugs_for_repo(&db, repo_id);
    let bug = db
        .get_bug_by_display_id(repo_id, &display_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("bug {} not found", display_id))?;
    // Map comment: None → don't touch; Some("") → clear to NULL; Some("x") → set
    let comment_arg = comment
        .as_ref()
        .map(|s| if s.is_empty() { None } else { Some(s.as_str()) });
    db.update_bug_fields(
        bug.id,
        description.as_deref(),
        severity.as_deref(),
        category.as_deref(),
        comment_arg,
    )
    .map_err(|e| e.to_string())?;
    let refreshed = db
        .get_bug_by_id(bug.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "bug vanished after update".to_string())?;
    let _ = sync::regenerate_bugs_md(&db, repo_id);
    Ok(refreshed.to_view())
}

/// Hard-delete a bug. UI gates visibility to `status='created'` (accidental
/// creation escape hatch). For real closed bugs, use `resolve_bug` — the row
/// stays in DB for history.
#[tauri::command]
fn delete_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<(), String> {
    // T-000128: reconcile LLM MD edits for OTHER bugs before deleting this one.
    let _ = sync::reconcile_bugs_for_repo(&db, repo_id);
    let bug = db
        .get_bug_by_display_id(repo_id, &display_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("bug {} not found", display_id))?;
    db.delete_bug(bug.id).map_err(|e| e.to_string())?;
    let _ = sync::regenerate_bugs_md(&db, repo_id);
    Ok(())
}

/// Mark a bug as rejected (✗ button). Valid from `testing` status only.
/// Row stays in MD — `rejected` is not terminal, next fix attempt loops
/// back to `in-progress → testing`.
#[tauri::command]
fn reject_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<BugView, String> {
    // T-000128: reconcile LLM MD edits before rejecting (LLM may have edited
    // comment with rejection rationale that should reach DB first).
    let _ = sync::reconcile_bugs_for_repo(&db, repo_id);
    let bug = db
        .get_bug_by_display_id(repo_id, &display_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("bug {} not found", display_id))?;
    if bug.status != "testing" {
        return Err(format!(
            "cannot reject from status '{}' — must be in 'testing' first",
            bug.status
        ));
    }
    let now = db::utc_now_rfc3339();
    db.update_bug_status(bug.id, "rejected", None, None)
        .map_err(|e| e.to_string())?;
    db.insert_bug_event(bug.id, "rejected", Some("testing"), Some("rejected"), &now)
        .map_err(|e| e.to_string())?;
    let refreshed = db
        .get_bug_by_id(bug.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "bug vanished after update".to_string())?;
    let _ = sync::regenerate_bugs_md(&db, repo_id);
    Ok(refreshed.to_view())
}

/// T-000130: reopen a `confirmed` or `rejected` bug back to `testing` so the
/// user can undo an accidental ✓ or ✗ verdict. Reopen is a user-initiated
/// rollback — not a new fix attempt — so `fix_attempts` is preserved and the
/// `entered_testing` invariant (`COUNT(bug_events.entered_testing) ==
/// bugs.fix_attempts`) holds. A `reopened` bug_event is logged so Dashboard /
/// activity feed see the action, but it does NOT contribute to KPI5
/// (avg attempts per closed in period) which filters by `entered_testing`.
/// `confirmed_at` and `archived_from_md_at` are cleared so the bug rejoins the
/// "active" set and reappears in MD on next regen.
#[tauri::command]
fn reopen_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<BugView, String> {
    // T-000128: reconcile LLM MD edits before mutating.
    let _ = sync::reconcile_bugs_for_repo(&db, repo_id);
    let bug = db
        .get_bug_by_display_id(repo_id, &display_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("bug {} not found", display_id))?;
    if bug.status != "confirmed" && bug.status != "rejected" {
        return Err(format!(
            "cannot reopen from status '{}' — must be 'confirmed' or 'rejected'",
            bug.status
        ));
    }
    let from_status = bug.status.clone();
    let now = db::utc_now_rfc3339();
    db.reopen_bug(bug.id).map_err(|e| e.to_string())?;
    db.insert_bug_event(
        bug.id,
        "reopened",
        Some(from_status.as_str()),
        Some("testing"),
        &now,
    )
    .map_err(|e| e.to_string())?;
    let refreshed = db
        .get_bug_by_id(bug.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "bug vanished after update".to_string())?;
    let _ = sync::regenerate_bugs_md(&db, repo_id);
    Ok(refreshed.to_view())
}

// ── Stats / Graph summaries ──────────────────────────────────────────────────
// Stats are live-computed from the `bugs` and `bug_events` tables — no
// persisted counters, no recalc commands. The legacy `*_stat` write-stubs
// (kept from v0.16.0 stats-table→VIEW migration) were removed in v0.30.0
// (T-000093) along with their unused TS wrappers.

#[tauri::command]
fn get_repo_stats_summary(db: State<AppDb>, repository_id: i64) -> Result<StatsSummary, String> {
    db.stats_summary_for_repo(repository_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_project_stats_summary(db: State<AppDb>, project_id: i64) -> Result<StatsSummary, String> {
    db.stats_summary_for_project(project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_project_graph(db: State<AppDb>, project_id: i64) -> Result<ProjectGraph, String> {
    db.get_project_graph(project_id).map_err(|e| e.to_string())
}

// ── Dashboard v0.17.0 ────────────────────────────────────────────────────────

/// v0.17.0 Dashboard — single aggregator command.
/// Returns full DashboardData snapshot for given filter (period + projects).
#[tauri::command]
fn read_dashboard(db: State<AppDb>, filter: DashboardFilter) -> Result<DashboardData, String> {
    let project_ids_opt: Option<Vec<i64>> = match &filter.project_ids {
        Some(ids) if !ids.is_empty() => Some(ids.clone()),
        _ => None,
    };
    let pid_slice: Option<&[i64]> = project_ids_opt.as_deref();

    let p_start = &filter.period.start;
    let p_end = &filter.period.end;
    let cp = filter.compare_period.as_ref();

    // KPI 1: Active bugs
    let active = db.count_active_bugs(pid_slice).map_err(|e| e.to_string())?;
    let critical = db
        .count_active_bugs_with_severity(pid_slice, "critical")
        .map_err(|e| e.to_string())?;

    // KPI 2: Closed in period (+ compare)
    let closed = db
        .count_closed_bugs_in_period(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;
    let closed_prev = if let Some(c) = cp {
        Some(
            db.count_closed_bugs_in_period(pid_slice, &c.start, &c.end)
                .map_err(|e| e.to_string())? as f64,
        )
    } else {
        None
    };

    // KPI 3: Tasks done (from done.md per repo)
    let tasks_done = aggregate_tasks_done(&db, pid_slice, p_start, p_end)?;
    let tasks_done_prev = if let Some(c) = cp {
        Some(aggregate_tasks_done(&db, pid_slice, &c.start, &c.end)? as f64)
    } else {
        None
    };

    // KPI 4: % solve
    let opened = db
        .count_opened_bugs_in_period(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;
    let solve_rate = if closed + opened > 0 {
        Some((closed as f64 / (closed + opened) as f64) * 100.0)
    } else {
        None
    };
    let solve_rate_prev = if let Some(c) = cp {
        let cl = db
            .count_closed_bugs_in_period(pid_slice, &c.start, &c.end)
            .map_err(|e| e.to_string())?;
        let op = db
            .count_opened_bugs_in_period(pid_slice, &c.start, &c.end)
            .map_err(|e| e.to_string())?;
        if cl + op > 0 {
            Some((cl as f64 / (cl + op) as f64) * 100.0)
        } else {
            None
        }
    } else {
        None
    };

    // KPI 5: avg attempts
    let avg_attempts = db
        .avg_attempts_per_closed_in_period(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;
    let avg_attempts_prev = if let Some(c) = cp {
        db.avg_attempts_per_closed_in_period(pid_slice, &c.start, &c.end)
            .map_err(|e| e.to_string())?
    } else {
        None
    };

    // Top-hot (shown when >1 or all projects selected)
    let show_top_hot = match &filter.project_ids {
        Some(ids) => ids.len() > 1 || ids.is_empty(),
        None => true,
    };
    let top_hot = if show_top_hot {
        db.top_hot_projects(pid_slice, Some((p_start, p_end)), 3)
            .map_err(|e| e.to_string())?
    } else {
        vec![]
    };

    // Bugs per day
    let bugs_daily = db
        .bugs_per_day(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;

    // Tasks per day
    let tasks_daily = tasks_daily_flow(&db, pid_slice, p_start, p_end)?;

    // Categories
    let categories = db
        .category_efficiency(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;

    Ok(DashboardData {
        // active_bugs is a point-in-time count, not a period flow — delta
        // vs prev_value would be misleading (e.g. "10 active" today vs
        // "8 active" three months ago says nothing about throughput).
        // The critical-count sub-line carries the only meaningful overlay.
        active_bugs: KpiCard {
            value: Some(active as f64),
            prev_value: None,
            critical_count: Some(critical),
        },
        closed_in_period: KpiCard {
            value: Some(closed as f64),
            prev_value: closed_prev,
            critical_count: None,
        },
        tasks_done: KpiCard {
            value: Some(tasks_done as f64),
            prev_value: tasks_done_prev,
            critical_count: None,
        },
        solve_rate: KpiCard {
            value: solve_rate,
            prev_value: solve_rate_prev,
            critical_count: None,
        },
        attempts_per_closed: KpiCard {
            value: avg_attempts,
            prev_value: avg_attempts_prev,
            critical_count: None,
        },
        top_hot,
        bugs_daily,
        tasks_daily,
        categories,
    })
}

/// Helper: walks all filtered repos that have local_path, parses done.md, sums entries.
fn aggregate_tasks_done(
    db: &AppDb,
    project_ids: Option<&[i64]>,
    start: &str,
    end: &str,
) -> Result<i64, String> {
    let repos = db
        .list_repos_with_local_path(project_ids)
        .map_err(|e| e.to_string())?;
    let mut total = 0i64;
    for r in repos {
        if let Some(lp) = &r.local_path {
            let done_path = std::path::PathBuf::from(lp).join("docs").join("done.md");
            let entries = crate::export::parse_done_entries_in_period(&done_path, start, end)
                .map_err(|e| e.to_string())?;
            total += entries.iter().map(|(_, n)| n).sum::<i64>();
        }
    }
    Ok(total)
}

/// Helper: produces DailyFlowDay vec for tasks (done only).
fn tasks_daily_flow(
    db: &AppDb,
    project_ids: Option<&[i64]>,
    start: &str,
    end: &str,
) -> Result<Vec<DailyFlowDay>, String> {
    let repos = db
        .list_repos_with_local_path(project_ids)
        .map_err(|e| e.to_string())?;
    use std::collections::BTreeMap;
    let mut per_day: BTreeMap<String, i64> = BTreeMap::new();

    for r in repos {
        if let Some(lp) = &r.local_path {
            let done_path = std::path::PathBuf::from(lp).join("docs").join("done.md");
            let entries = crate::export::parse_done_entries_in_period(&done_path, start, end)
                .map_err(|e| e.to_string())?;
            for (date, count) in entries {
                *per_day.entry(date).or_insert(0) += count;
            }
        }
    }

    let start_d =
        chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let end_d = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let today = chrono::Local::now().date_naive();

    let mut out = Vec::new();
    let mut d = start_d;
    while d <= end_d {
        let key = d.format("%Y-%m-%d").to_string();
        out.push(DailyFlowDay {
            date: key.clone(),
            opened: None,
            closed: None,
            done: Some(*per_day.get(&key).unwrap_or(&0)),
            is_future: d > today,
        });
        match d.succ_opt() {
            Some(next) => d = next,
            None => break,
        }
    }
    Ok(out)
}

// ── Activity feed (v0.19.0) ──────────────────────────────────────────────────

#[tauri::command]
fn read_recent_activity(
    db: State<AppDb>,
    limit: u32,
) -> Result<Vec<crate::models::ActivityEvent>, String> {
    db.recent_activity(limit).map_err(|e| e.to_string())
}

// ── Requirements sync commands ───────────────────────────────────────────────

#[derive(serde::Serialize)]
struct SyncGlobalClaudeResult {
    path: String,
    synced_at: String,
}

#[tauri::command]
fn sync_global_claude_md(db: State<AppDb>) -> Result<SyncGlobalClaudeResult, String> {
    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let claude_path = home.join(".claude").join("CLAUDE.md");
    sync::update_claude_md_global(&db, &claude_path)?;
    let now = chrono::Utc::now().to_rfc3339();
    db.set_setting("ai_rules_last_sync_at", &now)
        .map_err(|e| e.to_string())?;
    Ok(SyncGlobalClaudeResult {
        path: claude_path.display().to_string(),
        synced_at: now,
    })
}

/// Init skeletons for a single repo — F-016 manual trigger.
/// Copies-if-missing: docs/todo.md, docs/bug-reports.md; section-merges .gitignore + .gitattributes.
/// Returns list of filenames actually created (empty list = all already exist).
#[tauri::command]
fn init_docs_for_repo(db: State<AppDb>, repo_id: i64) -> Result<Vec<String>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let path = repo
        .local_path
        .as_deref()
        .ok_or_else(|| "Repository has no local_path".to_string())?;
    let base = Path::new(path);
    sync::ensure_root_exists(base)?;

    let todo_template = db
        .get_template_file("_global", "todo.md.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();
    let bug_reports_template = db
        .get_template_file("_global", "bug-reports.md.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();
    let gitignore_template = db
        .get_template_file("_global", ".gitignore.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();
    let gitattributes_template = db
        .get_template_file("_global", ".gitattributes.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();

    let mut updated = Vec::new();
    if sync::copy_doc_skeleton_if_missing(&todo_template, base, "todo.md")? {
        updated.push("docs/todo.md".to_string());
    }
    if sync::copy_doc_skeleton_if_missing(&bug_reports_template, base, "bug-reports.md")? {
        updated.push("docs/bug-reports.md".to_string());
    }
    if sync::sync_gitignore_section(&gitignore_template, base)? {
        updated.push(".gitignore (section)".to_string());
    }
    if sync::sync_gitattributes_section(&gitattributes_template, base)? {
        updated.push(".gitattributes (section)".to_string());
    }
    // App-owned files (project.md + CLAUDE.md section) — always overwritten when
    // the repo is attached to a project. Orphan repos (project_id=None) skip this
    // since project-context wouldn't render meaningfully.
    if let Some(pid) = repo.project_id {
        sync::generate_project_md(&db, pid, base)?;
        updated.push("docs/project.md".to_string());
        sync::update_claude_md_section(
            &db,
            Some(pid),
            repo.role.as_deref(),
            &base.join("CLAUDE.md"),
        )?;
        updated.push("CLAUDE.md (section)".to_string());
    } else {
        // M6 review-fix: surface that the repo is orphan and the app-owned
        // files were intentionally skipped. Without this entry the success
        // toast lists what was actually written, leaving the user wondering
        // why project.md / CLAUDE.md weren't updated.
        updated.push("(project.md + CLAUDE.md skipped — repo has no project assigned)".to_string());
    }
    Ok(updated)
}

#[tauri::command]
fn sync_project(db: State<AppDb>, project_id: i64) -> Result<SyncResult, String> {
    let all_repos = db
        .list_repos_by_project(Some(project_id))
        .map_err(|e| e.to_string())?;
    let microservice_ids = db
        .list_project_microservices(project_id)
        .map_err(|e| e.to_string())?;

    let server = all_repos
        .iter()
        .find(|r| r.role.as_deref() == Some("server"));
    let clients: Vec<&Repository> = all_repos
        .iter()
        .filter(|r| {
            matches!(
                r.role.as_deref(),
                Some("client") | Some("admin_client") | Some("test_client")
            )
        })
        .collect();
    // F-012: microservice_ids are now microservice-PROJECT ids (not repo ids).
    // For each, resolve its single server-repo at sync-time.
    let mut copied = 0usize;
    let mut responses = 0usize;
    let mut migrated = 0usize;
    let mut errors: Vec<String> = vec![];

    // 0.10.0 pre-phase: write project.md + CLAUDE.md section + .gitignore + .gitattributes to all repos.
    // 0.11.0 extends with todo.md + bug-reports.md skeletons.
    let gitignore_template = db
        .get_template_file("_global", ".gitignore.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();
    let gitattributes_template = db
        .get_template_file("_global", ".gitattributes.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();
    let todo_template = db
        .get_template_file("_global", "todo.md.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();
    let bug_reports_template = db
        .get_template_file("_global", "bug-reports.md.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();

    for repo in &all_repos {
        let label = repo.display_name();
        let Some(path) = repo.local_path.as_deref() else {
            // B-000002: surface silently-skipped repos so user sees why project.md /
            // CLAUDE.md / .gitignore weren't written for this repo.
            errors.push(format!(
                "Repo {}: no local_path set — project.md / CLAUDE.md / .gitignore skipped",
                label
            ));
            continue;
        };
        let base = Path::new(path);
        if let Err(e) = sync::ensure_root_exists(base) {
            errors.push(format!(
                "Repo {}: {} — project.md / CLAUDE.md / .gitignore skipped",
                label, e
            ));
            continue;
        }
        if let Err(e) = sync::generate_project_md(&db, project_id, base) {
            errors.push(format!("project.md for {}: {}", label, e));
        }
        if let Err(e) = sync::update_claude_md_section(
            &db,
            Some(project_id),
            repo.role.as_deref(),
            &base.join("CLAUDE.md"),
        ) {
            errors.push(format!("CLAUDE.md for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitignore_section(&gitignore_template, base) {
            errors.push(format!(".gitignore for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitattributes_section(&gitattributes_template, base) {
            errors.push(format!(".gitattributes for {}: {}", label, e));
        }
        if let Err(e) = sync::copy_doc_skeleton_if_missing(&todo_template, base, "todo.md") {
            errors.push(format!("todo.md for {}: {}", label, e));
        }
        if let Err(e) =
            sync::copy_doc_skeleton_if_missing(&bug_reports_template, base, "bug-reports.md")
        {
            errors.push(format!("bug-reports.md for {}: {}", label, e));
        }
    }
    for ms_id in &microservice_ids {
        let Ok(ms_server) = db.server_repo_of_microservice(*ms_id) else {
            errors.push(format!(
                "Microservice project {}: no server-repo resolved — project.md skipped",
                ms_id
            ));
            continue;
        };
        let label = ms_server.display_name();
        let Some(path) = ms_server.local_path.as_deref() else {
            errors.push(format!(
                "Microservice {}: no local_path set — project.md skipped",
                label
            ));
            continue;
        };
        let base = Path::new(path);
        if let Err(e) = sync::ensure_root_exists(base) {
            errors.push(format!(
                "Microservice {}: {} — project.md skipped",
                label, e
            ));
            continue;
        }
        if let Err(e) = sync::generate_project_md(&db, *ms_id, base) {
            errors.push(format!("project.md for {}: {}", label, e));
        }
        if let Err(e) = sync::update_claude_md_section(
            &db,
            Some(*ms_id),
            ms_server.role.as_deref(),
            &base.join("CLAUDE.md"),
        ) {
            errors.push(format!("CLAUDE.md for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitignore_section(&gitignore_template, base) {
            errors.push(format!(".gitignore for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitattributes_section(&gitattributes_template, base) {
            errors.push(format!(".gitattributes for {}: {}", label, e));
        }
        if let Err(e) = sync::copy_doc_skeleton_if_missing(&todo_template, base, "todo.md") {
            errors.push(format!("todo.md for {}: {}", label, e));
        }
        if let Err(e) =
            sync::copy_doc_skeleton_if_missing(&bug_reports_template, base, "bug-reports.md")
        {
            errors.push(format!("bug-reports.md for {}: {}", label, e));
        }
    }

    // Server/client checks only matter for `standard` project type. A
    // `microservice` project intentionally has neither — surfacing those as
    // errors produces false-positive warning toasts on every sync.
    let project_type = db
        .get_project(project_id)
        .ok()
        .map(|p| p.project_type)
        .unwrap_or_else(|| "standard".to_string());
    if project_type == "standard" {
        // P6 review-fix: "No clients" is only relevant once the server is
        // in place. Server-only build-out phase (server first, clients
        // later) shouldn't generate a warning toast on every sync — that
        // trains the user to ignore them. The "No server" case stays a
        // warning because the server is the core of the standard flow.
        if server.is_none() {
            errors.push("No server found in project".to_string());
        } else if clients.is_empty() {
            errors.push("No clients found in project".to_string());
        }
    }

    if let Some(srv) = server {
        if srv.local_path.is_none() {
            errors.push(format!("Server {} has no local_path", srv.display_name()));
        }
        if let Some(ref srv_path) = srv.local_path {
            let srv_base = Path::new(srv_path);

            // B-001: guard — do not silently recreate a moved/deleted server folder.
            if let Err(e) = sync::ensure_root_exists(srv_base) {
                errors.push(format!("Server {}: {}", srv.display_name(), e));
                return Ok(SyncResult {
                    copied,
                    responses,
                    migrated,
                    errors,
                });
            }

            // Client → Server sync
            for client in &clients {
                if let Some(ref client_path) = client.local_path {
                    let client_base = Path::new(client_path);
                    // B-001: skip clients whose folder was moved/deleted.
                    if let Err(e) = sync::ensure_root_exists(client_base) {
                        errors.push(format!("Client {}: {}", client.display_name(), e));
                        continue;
                    }
                    let client_req_dir = client_base.join("docs").join("backend-requirements");
                    // F-033: canonical_folder_name() is the single source of truth.
                    let client_name = client.canonical_folder_name();
                    let client_requirements_parent =
                        srv_base.join("docs").join("client-requirements");
                    // F-033 Stage 1e: replay client renames on server side before sync.
                    match db.list_renames_for_repo(client.id) {
                        Ok(renames) => {
                            for r in renames {
                                match sync::replay_rename_in_dir(
                                    &client_requirements_parent,
                                    &r.old_canonical,
                                    &r.new_canonical,
                                ) {
                                    Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                    Ok(sync::RenameOutcome::NoOp) => {}
                                    Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                        "Rename collision on server side: both {}/ and {}/ exist under client-requirements — manual intervention needed",
                                        r.old_canonical, r.new_canonical
                                    )),
                                    Err(e) => errors.push(format!(
                                        "Rename replay {} → {} on server: {}",
                                        r.old_canonical, r.new_canonical, e
                                    )),
                                }
                            }
                        }
                        Err(e) => {
                            errors.push(format!("List renames for client {}: {}", client.id, e))
                        }
                    }
                    let srv_client_dir = client_requirements_parent.join(&client_name);

                    // Copy REQ-*.md from client (source of truth) to server.
                    // Overwrite on change so sender edits propagate to the recipient.
                    for req_file in sync::scan_requirements(&client_req_dir) {
                        let src = client_req_dir.join(&req_file);
                        let dst = srv_client_dir.join(&req_file);
                        match sync::copy_file_if_changed(&src, &dst) {
                            Ok(true) => copied += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!("Copy {} -> server: {}", req_file, e)),
                        }
                    }

                    // Copy *.response.md from server (source of truth) back to client.
                    // Overwrite on change so recipient edits propagate to the sender.
                    for resp_file in sync::scan_responses(&srv_client_dir) {
                        let src = srv_client_dir.join(&resp_file);
                        let dst = client_req_dir.join(&resp_file);
                        match sync::copy_file_if_changed(&src, &dst) {
                            Ok(true) => responses += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!("Copy {} -> client: {}", resp_file, e)),
                        }
                    }

                    // 0.9.0: api.md + handlers.md target moved to docs/server-api/
                    // Auto-migrate old docs/api.md to docs/server-api/api.md.
                    let old_api = client_base.join("docs").join("api.md");
                    let new_api = client_base.join("docs").join("server-api").join("api.md");
                    if old_api.exists() && !new_api.exists() {
                        match sync::migrate_file(&old_api, &new_api) {
                            Ok(()) => migrated += 1,
                            Err(e) => errors.push(format!(
                                "Migrate api.md on {}: {}",
                                client.display_name(),
                                e
                            )),
                        }
                    }

                    // Copy server's docs/api.md to client's docs/server-api/api.md
                    let srv_api = srv_base.join("docs").join("api.md");
                    if srv_api.exists() {
                        match sync::copy_file_if_changed(&srv_api, &new_api) {
                            Ok(true) => copied += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!(
                                "Copy api.md -> {}: {}",
                                client.display_name(),
                                e
                            )),
                        }
                    }
                    // M5 review-fix: `api.md` absent → silent skip, symmetric
                    // with `handlers.md`. Freshly scaffolded servers that haven't
                    // yet written their contract were generating an error toast
                    // on every sync. Once the server writes api.md the next sync
                    // picks it up normally; the "missing api.md" condition is
                    // reported in the client's pre-flight check (see global
                    // CLAUDE.md `# API contract sync`), not via sync errors.

                    // Copy server's docs/handlers.md to client's docs/server-api/handlers.md
                    // (optional — silent skip if missing)
                    let srv_handlers = srv_base.join("docs").join("handlers.md");
                    let client_handlers = client_base
                        .join("docs")
                        .join("server-api")
                        .join("handlers.md");
                    if srv_handlers.exists() {
                        match sync::copy_file_if_changed(&srv_handlers, &client_handlers) {
                            Ok(true) => copied += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!(
                                "Copy handlers.md -> {}: {}",
                                client.display_name(),
                                e
                            )),
                        }
                    }
                }
            }

            // F-012: Server → Microservice sync.
            // For each connected microservice-project, resolve its single server-repo
            // and sync REQ-*.md into that repo's docs/server-requirements/.
            for ms_project_id in &microservice_ids {
                let ms_server_repo = match db.server_repo_of_microservice(*ms_project_id) {
                    Ok(r) => r,
                    Err(e) => {
                        errors.push(format!("Microservice project {}: {}", ms_project_id, e));
                        continue;
                    }
                };
                let ms_path = match ms_server_repo.local_path.as_deref() {
                    Some(p) => p,
                    None => {
                        errors.push(format!(
                            "Microservice server-repo {} has no local_path",
                            ms_server_repo.display_name()
                        ));
                        continue;
                    }
                };
                let ms_base = Path::new(ms_path);
                // B-001: skip microservices whose folder was moved/deleted.
                if let Err(e) = sync::ensure_root_exists(ms_base) {
                    errors.push(format!(
                        "Microservice {}: {}",
                        ms_server_repo.display_name(),
                        e
                    ));
                    continue;
                }
                // Resolve microservice-project name for subfolder on server side.
                let ms_project = match db.get_project(*ms_project_id) {
                    Ok(p) => p,
                    Err(e) => {
                        errors.push(format!(
                            "Microservice project {} lookup: {}",
                            ms_project_id, e
                        ));
                        continue;
                    }
                };
                let ms_name = ms_project.name; // retained for microservice-api/<ms_name>/ path
                                               // F-033: REQ sync folders use canonical repo name, not project name.
                let ms_canonical = ms_server_repo.canonical_folder_name();
                let parent_folder = srv.canonical_folder_name();
                let ms_req_parent = srv_base.join("docs").join("microservice-requirements");
                let ms_srv_parent = ms_base.join("docs").join("server-requirements");

                // F-033 Stage 1f Case B: rename server-side folder <project-name>/ → <ms_canonical>/
                // for existing installations (one-time migration; idempotent on subsequent syncs).
                if ms_name != ms_canonical {
                    let mut case_b_warnings = Vec::new();
                    match sync::migrate_subfolder_rename(
                        &ms_req_parent,
                        &ms_name,
                        &ms_canonical,
                        &mut case_b_warnings,
                    ) {
                        Ok(true) => migrated += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!(
                            "Case B migrate {} → {}: {}",
                            ms_name, ms_canonical, e
                        )),
                    }
                    errors.append(&mut case_b_warnings);
                }

                // F-033 Stage 1e: replay ms-server-repo renames on parent side (microservice-requirements/).
                match db.list_renames_for_repo(ms_server_repo.id) {
                    Ok(renames) => {
                        for r in renames {
                            match sync::replay_rename_in_dir(
                                &ms_req_parent,
                                &r.old_canonical,
                                &r.new_canonical,
                            ) {
                                Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                Ok(sync::RenameOutcome::NoOp) => {}
                                Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                    "Rename collision on server side: both {}/ and {}/ exist under microservice-requirements — manual intervention needed",
                                    r.old_canonical, r.new_canonical
                                )),
                                Err(e) => errors.push(format!(
                                    "Rename replay {} → {} on server: {}",
                                    r.old_canonical, r.new_canonical, e
                                )),
                            }
                        }
                    }
                    Err(e) => errors.push(format!(
                        "List renames for ms-server-repo {}: {}",
                        ms_server_repo.id, e
                    )),
                }

                // T-000092: replay ms-PROJECT renames on parent side
                // (microservice-api/<ms-project-name>/). repo_renames doesn't
                // cover project renames because the folder is keyed by project
                // name, not repo canonical name.
                let ms_api_parent = srv_base.join("docs").join("microservice-api");
                match db.list_renames_for_project(*ms_project_id) {
                    Ok(renames) => {
                        for r in renames {
                            match sync::replay_rename_in_dir(
                                &ms_api_parent,
                                &r.old_name,
                                &r.new_name,
                            ) {
                                Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                Ok(sync::RenameOutcome::NoOp) => {}
                                Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                    "Rename collision on parent side: both {}/ and {}/ exist under microservice-api — manual intervention needed",
                                    r.old_name, r.new_name
                                )),
                                Err(e) => errors.push(format!(
                                    "Project rename replay {} → {} on parent: {}",
                                    r.old_name, r.new_name, e
                                )),
                            }
                        }
                    }
                    Err(e) => errors.push(format!(
                        "List renames for ms-project {}: {}",
                        ms_project_id, e
                    )),
                }

                // F-033 Stage 1e: replay server renames on ms side (server-requirements/).
                match db.list_renames_for_repo(srv.id) {
                    Ok(renames) => {
                        for r in renames {
                            match sync::replay_rename_in_dir(
                                &ms_srv_parent,
                                &r.old_canonical,
                                &r.new_canonical,
                            ) {
                                Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                Ok(sync::RenameOutcome::NoOp) => {}
                                Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                    "Rename collision on microservice side: both {}/ and {}/ exist under server-requirements — manual intervention needed",
                                    r.old_canonical, r.new_canonical
                                )),
                                Err(e) => errors.push(format!(
                                    "Rename replay {} → {} on microservice: {}",
                                    r.old_canonical, r.new_canonical, e
                                )),
                            }
                        }
                    }
                    Err(e) => errors.push(format!("List renames for server {}: {}", srv.id, e)),
                }

                // F-033 Stage 1f Case C: migrate flat server-requirements/*.md to nested
                // server-requirements/<parent-canonical>/<filename>. Runs per (parent, ms) iteration
                // but idempotent — first parent's pass does the heavy lift, later parents see empty.
                let parents_for_ms = db
                    .list_parents_of_microservice(*ms_project_id)
                    .unwrap_or_default();
                let mut parent_candidates: Vec<(String, PathBuf)> = Vec::new();
                for parent_proj in &parents_for_ms {
                    // server_repo_of_microservice works for any project_id — returns role=server repo.
                    if let Ok(p_srv_repo) = db.server_repo_of_microservice(parent_proj.id) {
                        if let Some(ref lp) = p_srv_repo.local_path {
                            let parent_req_dir = Path::new(lp)
                                .join("docs")
                                .join("microservice-requirements")
                                .join(&ms_canonical);
                            parent_candidates
                                .push((p_srv_repo.canonical_folder_name(), parent_req_dir));
                        }
                    }
                }
                if !parent_candidates.is_empty() {
                    let lookup = |name: &str| -> Vec<(String, PathBuf)> {
                        parent_candidates
                            .iter()
                            .map(|(c, d)| (c.clone(), d.join(name)))
                            .collect()
                    };
                    let mut case_c_warnings = Vec::new();
                    match sync::migrate_flat_to_nested(&ms_srv_parent, lookup, &mut case_c_warnings)
                    {
                        Ok(n) => migrated += n,
                        Err(e) => errors.push(format!("Case C migration: {}", e)),
                    }
                    errors.append(&mut case_c_warnings);
                }

                let srv_ms_dir = ms_req_parent.join(&ms_canonical);
                // F-033: nested per parent-server so multi-parent microservices don't collide.
                let ms_srv_dir = ms_srv_parent.join(&parent_folder);

                // Copy REQ-*.md from server (source of truth) to microservice server-repo.
                // Overwrite on change so sender edits propagate to the recipient.
                for req_file in sync::scan_requirements(&srv_ms_dir) {
                    let src = srv_ms_dir.join(&req_file);
                    let dst = ms_srv_dir.join(&req_file);
                    match sync::copy_file_if_changed(&src, &dst) {
                        Ok(true) => copied += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!("Copy {} -> microservice: {}", req_file, e)),
                    }
                }

                // Copy *.response.md from microservice (source of truth) back to server.
                // Overwrite on change so recipient edits propagate to the sender.
                for resp_file in sync::scan_responses(&ms_srv_dir) {
                    let src = ms_srv_dir.join(&resp_file);
                    let dst = srv_ms_dir.join(&resp_file);
                    match sync::copy_file_if_changed(&src, &dst) {
                        Ok(true) => responses += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!("Copy {} -> server: {}", resp_file, e)),
                    }
                }

                // 0.9.0: Microservice → Parent server — api.md + handlers.md
                // Target on parent: docs/microservice-api/<ms-project-name>/{api,handlers}.md
                let ms_api_src = ms_base.join("docs").join("api.md");
                let ms_api_dst = srv_base
                    .join("docs")
                    .join("microservice-api")
                    .join(&ms_name)
                    .join("api.md");
                if ms_api_src.exists() {
                    match sync::copy_file_if_changed(&ms_api_src, &ms_api_dst) {
                        Ok(true) => copied += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!(
                            "Copy ms api.md from {}: {}",
                            ms_server_repo.display_name(),
                            e
                        )),
                    }
                }
                let ms_handlers_src = ms_base.join("docs").join("handlers.md");
                let ms_handlers_dst = srv_base
                    .join("docs")
                    .join("microservice-api")
                    .join(&ms_name)
                    .join("handlers.md");
                if ms_handlers_src.exists() {
                    match sync::copy_file_if_changed(&ms_handlers_src, &ms_handlers_dst) {
                        Ok(true) => copied += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!(
                            "Copy ms handlers.md from {}: {}",
                            ms_server_repo.display_name(),
                            e
                        )),
                    }
                }
            }
        }
    }

    // B-000019/B-000020: MS-driven sync — when the current project is a
    // microservice, fan out to each connected parent server. Mirrors the
    // parent-driven block above but with the MS as initiator. Without this,
    // pressing Sync on an MS project is a no-op (its clients/microservices
    // loops are empty), so api.md edits never propagate and parents stay out
    // of sync until they happen to trigger Sync themselves. Rename-replay is
    // intentionally not duplicated here — parent-driven sync remains the
    // authority for that, this block does the steady-state file copies only.
    if project_type == "microservice" {
        if let Some(ms_server) = server {
            if let Some(ref ms_path) = ms_server.local_path {
                let ms_base = Path::new(ms_path);
                if let Err(e) = sync::ensure_root_exists(ms_base) {
                    errors.push(format!("Microservice {}: {}", ms_server.display_name(), e));
                } else {
                    let ms_canonical = ms_server.canonical_folder_name();
                    let ms_project_name = db
                        .get_project(project_id)
                        .map(|p| p.name)
                        .unwrap_or_default();
                    let parents = db
                        .list_parents_of_microservice(project_id)
                        .unwrap_or_default();
                    for parent_project in &parents {
                        let parent_repos = match db.list_repos_by_project(Some(parent_project.id)) {
                            Ok(r) => r,
                            Err(e) => {
                                errors.push(format!(
                                    "Parent {} list repos: {}",
                                    parent_project.name, e
                                ));
                                continue;
                            }
                        };
                        let Some(parent_server) = parent_repos
                            .iter()
                            .find(|r| r.role.as_deref() == Some("server"))
                        else {
                            errors.push(format!(
                                "Parent {}: no server-repo found",
                                parent_project.name
                            ));
                            continue;
                        };
                        let Some(ref parent_local) = parent_server.local_path else {
                            errors.push(format!(
                                "Parent {} ({}): server-repo has no local_path",
                                parent_project.name,
                                parent_server.display_name()
                            ));
                            continue;
                        };
                        let parent_base = Path::new(parent_local);
                        if let Err(e) = sync::ensure_root_exists(parent_base) {
                            errors.push(format!(
                                "Parent {} ({}): {}",
                                parent_project.name,
                                parent_server.display_name(),
                                e
                            ));
                            continue;
                        }
                        let parent_canonical = parent_server.canonical_folder_name();

                        // MS → parent: api.md + handlers.md
                        for filename in ["api.md", "handlers.md"] {
                            let src = ms_base.join("docs").join(filename);
                            if !src.exists() {
                                continue;
                            }
                            let dst = parent_base
                                .join("docs")
                                .join("microservice-api")
                                .join(&ms_project_name)
                                .join(filename);
                            match sync::copy_file_if_changed(&src, &dst) {
                                Ok(true) => copied += 1,
                                Ok(false) => {}
                                Err(e) => errors.push(format!(
                                    "Copy {} -> parent {}: {}",
                                    filename, parent_project.name, e
                                )),
                            }
                        }

                        // parent → MS: REQ-*.md (source of truth on parent side)
                        let parent_ms_dir = parent_base
                            .join("docs")
                            .join("microservice-requirements")
                            .join(&ms_canonical);
                        let ms_parent_dir = ms_base
                            .join("docs")
                            .join("server-requirements")
                            .join(&parent_canonical);
                        for req_file in sync::scan_requirements(&parent_ms_dir) {
                            let src = parent_ms_dir.join(&req_file);
                            let dst = ms_parent_dir.join(&req_file);
                            match sync::copy_file_if_changed(&src, &dst) {
                                Ok(true) => copied += 1,
                                Ok(false) => {}
                                Err(e) => errors.push(format!(
                                    "Copy {} from parent {} to MS: {}",
                                    req_file, parent_project.name, e
                                )),
                            }
                        }

                        // MS → parent: *.response.md (source of truth on MS side)
                        for resp_file in sync::scan_responses(&ms_parent_dir) {
                            let src = ms_parent_dir.join(&resp_file);
                            let dst = parent_ms_dir.join(&resp_file);
                            match sync::copy_file_if_changed(&src, &dst) {
                                Ok(true) => responses += 1,
                                Ok(false) => {}
                                Err(e) => errors.push(format!(
                                    "Copy {} from MS to parent {}: {}",
                                    resp_file, parent_project.name, e
                                )),
                            }
                        }
                    }
                }
            }
        }
    }

    // v0.20.0: record sync event. SyncResult fields per models.rs:282 — copied + responses + migrated
    let total_changes = (copied + responses + migrated) as i64;
    let _ = db.insert_sync_event(
        None,
        "project_sync",
        &chrono::Utc::now().to_rfc3339(),
        total_changes,
        Some(&format!(r#"{{"project_id":{}}}"#, project_id)),
    );

    Ok(SyncResult {
        copied,
        responses,
        migrated,
        errors,
    })
}

#[tauri::command]
fn list_project_requirements(
    db: State<AppDb>,
    project_id: i64,
) -> Result<Vec<RequirementInfo>, String> {
    let all_repos = db
        .list_repos_by_project(Some(project_id))
        .map_err(|e| e.to_string())?;
    let microservice_ids = db
        .list_project_microservices(project_id)
        .map_err(|e| e.to_string())?;

    let server = all_repos
        .iter()
        .find(|r| r.role.as_deref() == Some("server"));
    let clients: Vec<&Repository> = all_repos
        .iter()
        .filter(|r| {
            matches!(
                r.role.as_deref(),
                Some("client") | Some("admin_client") | Some("test_client")
            )
        })
        .collect();
    // F-012: resolve microservice-projects and their single server-repos.
    let mut ms_entries: Vec<(String, Repository)> = vec![]; // (ms-project name, server-repo)
    for ms_project_id in &microservice_ids {
        let Ok(ms_project) = db.get_project(*ms_project_id) else {
            continue;
        };
        let Ok(ms_server_repo) = db.server_repo_of_microservice(*ms_project_id) else {
            continue;
        };
        ms_entries.push((ms_project.name, ms_server_repo));
    }

    let mut result: Vec<RequirementInfo> = vec![];

    if let Some(srv) = server {
        if let Some(ref srv_path) = srv.local_path {
            let srv_base = Path::new(srv_path);

            // Client → Server requirements
            for client in &clients {
                if let Some(ref client_path) = client.local_path {
                    let client_base = Path::new(client_path);
                    let client_req_dir = client_base.join("docs").join("backend-requirements");
                    // F-033: canonical_folder_name() is the single source of truth.
                    let client_name = client.canonical_folder_name();
                    let srv_client_dir = srv_base
                        .join("docs")
                        .join("client-requirements")
                        .join(&client_name);

                    for req_file in sync::scan_requirements(&client_req_dir) {
                        let on_server = srv_client_dir.join(&req_file).exists();
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = srv_client_dir.join(&response_name).exists()
                            || client_req_dir.join(&response_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else if on_server {
                            "sent".to_string()
                        } else {
                            "new".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "client_to_server".to_string(),
                            status,
                            source_repo: client.display_name().clone(),
                            target_repo: srv.display_name().clone(),
                            is_reverse_lookup: false,
                        });
                    }

                    // Show api.md + handlers.md sync status (server → client, docs/server-api/)
                    for (name, src_filename) in
                        [("api.md", "api.md"), ("handlers.md", "handlers.md")]
                    {
                        let srv_file = srv_base.join("docs").join(src_filename);
                        if !srv_file.exists() {
                            continue;
                        }
                        let client_file = client_base
                            .join("docs")
                            .join("server-api")
                            .join(src_filename);
                        let status = if !client_file.exists() {
                            "new".to_string()
                        } else {
                            let src = std::fs::read(&srv_file).unwrap_or_default();
                            let dst = std::fs::read(&client_file).unwrap_or_default();
                            if src == dst {
                                "sent".to_string()
                            } else {
                                "new".to_string()
                            }
                        };
                        result.push(RequirementInfo {
                            filename: name.to_string(),
                            direction: "server_to_client".to_string(),
                            status,
                            source_repo: srv.display_name().clone(),
                            target_repo: client.display_name().clone(),
                            is_reverse_lookup: false,
                        });
                    }

                    // Also check server side for reqs that may only exist there
                    for req_file in sync::scan_requirements(&srv_client_dir) {
                        if result.iter().any(|r| {
                            r.filename == req_file && r.source_repo == client.display_name()
                        }) {
                            continue;
                        }
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = srv_client_dir.join(&response_name).exists()
                            || client_req_dir.join(&response_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else {
                            "sent".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "client_to_server".to_string(),
                            status,
                            source_repo: client.display_name().clone(),
                            target_repo: srv.display_name().clone(),
                            is_reverse_lookup: false,
                        });
                    }
                }
            }

            // F-012: Server → Microservice requirements
            for (ms_name, ms_server_repo) in &ms_entries {
                if let Some(ref ms_path) = ms_server_repo.local_path {
                    let ms_base = Path::new(ms_path);
                    // F-033: REQ folders use canonical repo names; nested per parent on ms side.
                    let ms_canonical = ms_server_repo.canonical_folder_name();
                    let parent_folder = srv.canonical_folder_name();
                    let srv_ms_dir = srv_base
                        .join("docs")
                        .join("microservice-requirements")
                        .join(&ms_canonical);
                    let ms_srv_dir = ms_base
                        .join("docs")
                        .join("server-requirements")
                        .join(&parent_folder);

                    for req_file in sync::scan_requirements(&srv_ms_dir) {
                        let on_ms = ms_srv_dir.join(&req_file).exists();
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = ms_srv_dir.join(&response_name).exists()
                            || srv_ms_dir.join(&response_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else if on_ms {
                            "sent".to_string()
                        } else {
                            "new".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: srv.display_name().clone(),
                            target_repo: ms_server_repo.display_name().clone(),
                            is_reverse_lookup: false,
                        });
                    }

                    // Also check ms side
                    for req_file in sync::scan_requirements(&ms_srv_dir) {
                        if result.iter().any(|r| {
                            r.filename == req_file && r.target_repo == ms_server_repo.display_name()
                        }) {
                            continue;
                        }
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = ms_srv_dir.join(&response_name).exists()
                            || srv_ms_dir.join(&response_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else {
                            "sent".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: srv.display_name().clone(),
                            target_repo: ms_server_repo.display_name().clone(),
                            is_reverse_lookup: false,
                        });
                    }

                    // 0.9.0: Microservice → Parent server — api.md + handlers.md
                    for (filename, direction) in [
                        ("api.md", "microservice_to_server_api"),
                        ("handlers.md", "microservice_to_server_handlers"),
                    ] {
                        let ms_src = ms_base.join("docs").join(filename);
                        if !ms_src.exists() {
                            continue;
                        }
                        let parent_dst = srv_base
                            .join("docs")
                            .join("microservice-api")
                            .join(ms_name)
                            .join(filename);
                        let status = if !parent_dst.exists() {
                            "new".to_string()
                        } else {
                            let src = std::fs::read(&ms_src).unwrap_or_default();
                            let dst = std::fs::read(&parent_dst).unwrap_or_default();
                            if src == dst {
                                "sent".to_string()
                            } else {
                                "new".to_string()
                            }
                        };
                        result.push(RequirementInfo {
                            filename: filename.to_string(),
                            direction: direction.to_string(),
                            status,
                            source_repo: ms_server_repo.display_name().clone(),
                            target_repo: srv.display_name().clone(),
                            is_reverse_lookup: false,
                        });
                    }
                }
            }
        }
    }

    // B-000018 reverse-lookup: открывая ms-проект, показать requirements от parent серверов.
    // Sender = parent server, recipient = текущий ms. Confirm-✓ скрыт в UI — sender'у проще
    // подтвердить из своего собственного SyncScreen (project_microservices direct view).
    let current_project = db.get_project(project_id).map_err(|e| e.to_string())?;
    if current_project.project_type == "microservice" {
        if let Some(ms_server) = server {
            if let Some(ref ms_local) = ms_server.local_path {
                let ms_base = Path::new(ms_local);
                let ms_canonical = ms_server.canonical_folder_name();
                let parents = db
                    .list_parents_of_microservice(project_id)
                    .map_err(|e| e.to_string())?;

                for parent_project in &parents {
                    let Ok(parent_repos) = db.list_repos_by_project(Some(parent_project.id)) else {
                        continue;
                    };
                    let Some(parent_server) = parent_repos
                        .iter()
                        .find(|r| r.role.as_deref() == Some("server"))
                    else {
                        continue;
                    };
                    let Some(ref parent_local) = parent_server.local_path else {
                        continue;
                    };
                    let parent_base = Path::new(parent_local);
                    let parent_canonical = parent_server.canonical_folder_name();

                    let parent_ms_dir = parent_base
                        .join("docs")
                        .join("microservice-requirements")
                        .join(&ms_canonical);
                    let ms_parent_dir = ms_base
                        .join("docs")
                        .join("server-requirements")
                        .join(&parent_canonical);

                    // REQ files parent → this ms (server_to_microservice direction)
                    for req_file in sync::scan_requirements(&parent_ms_dir) {
                        let response_name = req_file.replace(".md", ".response.md");
                        let on_ms = ms_parent_dir.join(&req_file).exists();
                        let has_response = ms_parent_dir.join(&response_name).exists()
                            || parent_ms_dir.join(&response_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else if on_ms {
                            "sent".to_string()
                        } else {
                            "new".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: parent_server.display_name().clone(),
                            target_repo: ms_server.display_name().clone(),
                            is_reverse_lookup: true,
                        });
                    }

                    // ms-side files без зеркала на parent (например ms ответил, до next sync)
                    for req_file in sync::scan_requirements(&ms_parent_dir) {
                        if result.iter().any(|r| {
                            r.filename == req_file
                                && r.direction == "server_to_microservice"
                                && r.source_repo == parent_server.display_name()
                                && r.target_repo == ms_server.display_name()
                        }) {
                            continue;
                        }
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = ms_parent_dir.join(&response_name).exists()
                            || parent_ms_dir.join(&response_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else {
                            "sent".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: parent_server.display_name().clone(),
                            target_repo: ms_server.display_name().clone(),
                            is_reverse_lookup: true,
                        });
                    }

                    // ms api.md / handlers.md going to each parent
                    for (filename, direction) in [
                        ("api.md", "microservice_to_server_api"),
                        ("handlers.md", "microservice_to_server_handlers"),
                    ] {
                        let ms_src = ms_base.join("docs").join(filename);
                        if !ms_src.exists() {
                            continue;
                        }
                        let parent_dst = parent_base
                            .join("docs")
                            .join("microservice-api")
                            .join(&current_project.name)
                            .join(filename);
                        let status = if !parent_dst.exists() {
                            "new".to_string()
                        } else {
                            let src = std::fs::read(&ms_src).unwrap_or_default();
                            let dst = std::fs::read(&parent_dst).unwrap_or_default();
                            if src == dst {
                                "sent".to_string()
                            } else {
                                "new".to_string()
                            }
                        };
                        result.push(RequirementInfo {
                            filename: filename.to_string(),
                            direction: direction.to_string(),
                            status,
                            source_repo: ms_server.display_name().clone(),
                            target_repo: parent_server.display_name().clone(),
                            is_reverse_lookup: true,
                        });
                    }
                }
            }
        }
    }

    Ok(result)
}

#[tauri::command]
fn confirm_requirement(
    db: State<AppDb>,
    project_id: i64,
    filename: String,
    source_repo_id: i64,
    target_repo_id: i64,
) -> Result<(), String> {
    // B-000021: paths are derived from source_repo + target_repo directly via
    // sync::confirm_pair. `project_id` is kept in the signature for frontend
    // compatibility and audit-trail purposes but plays no role in path
    // resolution — confirm works symmetrically from either side's SyncScreen.
    let _ = project_id;
    let source_repo = db
        .get_repository(source_repo_id)
        .map_err(|e| e.to_string())?;
    let target_repo = db
        .get_repository(target_repo_id)
        .map_err(|e| e.to_string())?;
    sync::confirm_pair(&source_repo, &target_repo, &filename)
}

// ── Rename log (F-033) ────────────────────────────────────────────────────────

#[tauri::command]
fn list_rename_history(db: State<AppDb>) -> Result<Vec<RepoRename>, String> {
    db.list_all_renames().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_renames_for_repo(db: State<AppDb>, repo_id: i64) -> Result<Vec<RepoRename>, String> {
    db.list_renames_for_repo(repo_id).map_err(|e| e.to_string())
}

// ── Templates (0.6.0) ─────────────────────────────────────────────────────────

#[tauri::command]
fn list_template_languages(db: State<AppDb>) -> Result<Vec<TemplateLanguage>, String> {
    let keys = db.list_template_languages().map_err(|e| e.to_string())?;
    let mut result: Vec<TemplateLanguage> = Vec::with_capacity(keys.len());
    for key in keys {
        let files = db.list_template_files(&key).map_err(|e| e.to_string())?;
        // Parse display_name from meta.json if present, fallback to language_key.
        let display_name = files
            .iter()
            .find(|f| f.file_name == "meta.json")
            .and_then(|f| serde_json::from_str::<serde_json::Value>(&f.content).ok())
            .and_then(|v| {
                v.get("display_name")
                    .and_then(|s| s.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| key.clone());
        result.push(TemplateLanguage {
            language_key: key,
            display_name,
            file_count: files.len() as i64,
        });
    }
    Ok(result)
}

#[tauri::command]
fn list_template_files(
    db: State<AppDb>,
    language_key: String,
) -> Result<Vec<TemplateFile>, String> {
    db.list_template_files(&language_key)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_template_file(
    db: State<AppDb>,
    language_key: String,
    file_name: String,
) -> Result<Option<TemplateFile>, String> {
    db.get_template_file(&language_key, &file_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_template_file(
    db: State<AppDb>,
    language_key: String,
    file_name: String,
    content: String,
) -> Result<(), String> {
    db.upsert_template_file(&language_key, &file_name, &content, true)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reset_template_file(
    db: State<AppDb>,
    language_key: String,
    file_name: String,
) -> Result<(), String> {
    let bundled = template_seeder::bundled_file_content(&language_key, &file_name)
        .ok_or_else(|| format!("No bundled default for {}/{}", language_key, file_name))?;
    db.upsert_template_file(&language_key, &file_name, &bundled, false)
        .map_err(|e| e.to_string())
}

// ── Deploy (0.7.0) ────────────────────────────────────────────────────────────

#[tauri::command]
fn set_deploy_target(
    db: State<AppDb>,
    id: i64,
    target: Option<String>,
) -> Result<Repository, String> {
    db.set_deploy_target(id, target.as_deref())
        .map_err(|e| e.to_string())
}

// T-000103 Task 1: repo-wide deploy config (placeholder values shared across
// envs — e.g. GO_VERSION baked into the single Dockerfile).
#[tauri::command]
fn get_repo_deploy_config(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<HashMap<String, String>, String> {
    db.get_repo_deploy_config(repo_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_repo_deploy_config(
    db: State<AppDb>,
    repo_id: i64,
    config: HashMap<String, String>,
) -> Result<(), String> {
    db.set_repo_deploy_config(repo_id, &config)
        .map_err(|e| e.to_string())
}

// ── Secret bundles (v1.3.0) ───────────────────────────────────────────────────
#[tauri::command]
fn list_secret_bundles(db: State<AppDb>) -> Result<Vec<SecretBundle>, String> {
    db.list_secret_bundles().map_err(|e| e.to_string())
}

#[tauri::command]
fn create_secret_bundle(db: State<AppDb>, name: String, description: String) -> Result<i64, String> {
    db.create_secret_bundle(&name, &description).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_secret_bundle(db: State<AppDb>, id: i64, name: String, description: String) -> Result<(), String> {
    db.rename_secret_bundle(id, &name, &description).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_secret_bundle(db: State<AppDb>, id: i64) -> Result<(), String> {
    db.delete_secret_bundle(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn upsert_bundle_item(db: State<AppDb>, bundle_id: i64, secret_name: String, value: String) -> Result<(), String> {
    db.upsert_bundle_item(bundle_id, &secret_name, &value).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_bundle_item(db: State<AppDb>, item_id: i64) -> Result<(), String> {
    db.delete_bundle_item(item_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_bundle_decrypted(db: State<AppDb>, bundle_id: i64) -> Result<Vec<SecretBundleItemValue>, String> {
    db.get_bundle_decrypted(bundle_id).map_err(|e| e.to_string())
}

// ── Deploy render (v0.18.0, multi-env) ────────────────────────────────────────

/// v0.18.0: render workflow/Dockerfile files for a single deploy_env.
/// Returns Vec<RenderedFile> with paths substituted per `meta.json.file_targets`
/// ({name} → deploy_env.name). Shared files (Dockerfile) appear ONCE per call —
/// multi-env consumers may get the same Dockerfile twice; caller dedupes.
///
/// Placeholder composition:
///   - core 5 from deploy_env (WORKFLOW_NAME, IMAGE_TAG, COMPOSE_SERVICE, DOMAIN, DEPLOY_BRANCH)
///   - extras (APP_PORT, NETWORK_NAME, COMPOSE_PROJECT, ENV_FILE_PATH, …)
///   - ENV_NAME = deploy_env.name
///   - BUILD_ARGS, RUNTIME_ENV_ARGS — per-env, from deploy_secrets with included=1 + role
///   - DOCKERFILE_ARGS, DART_DEFINES — UNION of build-role secrets across ALL deploy_envs of this repo
pub fn render_files_for_deploy_env(
    db: &AppDb,
    deploy_env_id: i64,
) -> Result<Vec<RenderedFile>, String> {
    let env = db
        .get_deploy_environment(deploy_env_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("deploy_environment {} not found", deploy_env_id))?;
    let repo = db
        .get_repository(env.repository_id)
        .map_err(|e| e.to_string())?;
    let target = repo
        .deploy_target
        .clone()
        .ok_or_else(|| "No deploy target set for this repository".to_string())?;

    // Load meta.json for file_targets + placeholder defaults
    let meta_file = db
        .get_template_file(&target, "meta.json")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("meta.json missing for language '{}'", target))?;
    let meta: serde_json::Value = serde_json::from_str(&meta_file.content)
        .map_err(|e| format!("Invalid meta.json: {}", e))?;
    let file_targets = meta
        .get("file_targets")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "meta.json missing 'file_targets'".to_string())?;

    // T-000103 Task 3: parse meta.placeholders strict (also gives us each
    // placeholder's `scope` for the schema-aware merger below).
    let meta_placeholders = template_meta::parse_meta_placeholders(&target, &meta)?;

    // T-000103 Task 3: fetch repo-wide deploy config (placeholder values for
    // repo-scope keys like GO_VERSION that render a single repo-wide
    // Dockerfile). Empty map on first render before user fills anything in.
    let repo_config = db
        .get_repo_deploy_config(env.repository_id)
        .map_err(|e| e.to_string())?;

    // Gather build/runtime secrets for THIS env
    let secrets = db
        .list_deploy_secrets(deploy_env_id)
        .map_err(|e| e.to_string())?;
    let build_for_this_env: Vec<String> = secrets
        .iter()
        .filter(|s| s.included && s.role.as_deref() == Some("build"))
        .map(|s| s.secret_name.clone())
        .collect();
    let runtime_for_this_env: Vec<String> = secrets
        .iter()
        .filter(|s| s.included && s.role.as_deref() == Some("runtime"))
        .map(|s| s.secret_name.clone())
        .collect();

    // UNION build-role secrets across ALL envs of this repo (for shared Dockerfile)
    let all_envs = db
        .list_deploy_environments(env.repository_id)
        .map_err(|e| e.to_string())?;
    let mut union_build: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in &all_envs {
        let esec = db
            .list_deploy_secrets(e.id)
            .map_err(|err| err.to_string())?;
        for s in esec {
            if s.included && s.role.as_deref() == Some("build") {
                union_build.insert(s.secret_name);
            }
        }
    }
    let union_build_vec: Vec<String> = union_build.into_iter().collect();

    // Build placeholder map. Order matters:
    //  1. Seed with each placeholder's `default` from meta.json.
    //  2. Overlay scope-driven values via build_placeholder_vars — sources
    //     `scope: "repo"` keys from `repo_config` and `scope: "environment"`
    //     (the default) from `env.extras`. Orphan keys in either source are
    //     filtered out by the merger.
    //  3. Override the typed columns from `deploy_environments` (core 5 +
    //     ENV_NAME) — these are not in `env.extras` but are listed in
    //     `meta.placeholders` with `scope: "environment"`; the merger emits
    //     nothing for them in step 2, so the explicit override below fills
    //     them from the typed columns.
    //  4. Insert v0.18.0-specific synthetic vars (BUILD_ARGS etc.) — these
    //     are NOT in meta.placeholders, they're rendered separately by the
    //     helper fns and substituted into placeholders that appear in the
    //     templates verbatim.
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(phs) = meta.get("placeholders").and_then(|v| v.as_object()) {
        for (k, spec) in phs {
            if let Some(default) = spec.get("default").and_then(|v| v.as_str()) {
                vars.insert(k.clone(), default.to_string());
            }
        }
    }
    // Schema-aware merge: pick each declared placeholder's value from the
    // correct source based on its scope. Orphan keys in either source are
    // ignored.
    for (k, v) in
        template_render::build_placeholder_vars(&meta_placeholders, &repo_config, &env.extras)
    {
        vars.insert(k, v);
    }
    // Core 5 from deploy_env typed columns (overrides defaults — these
    // values live on the typed columns, not in `extras`).
    vars.insert("WORKFLOW_NAME".to_string(), env.workflow_name.clone());
    vars.insert("IMAGE_TAG".to_string(), env.image_tag.clone());
    vars.insert("COMPOSE_SERVICE".to_string(), env.compose_service.clone());
    vars.insert("DOMAIN".to_string(), env.domain.clone());
    vars.insert("DEPLOY_BRANCH".to_string(), env.deploy_branch.clone());
    // v0.18.0-specific synthetic placeholders (not declared in meta.placeholders)
    vars.insert("ENV_NAME".to_string(), env.name.clone());
    vars.insert(
        "BUILD_ARGS".to_string(),
        template_render::render_build_args(&build_for_this_env),
    );
    vars.insert(
        "RUNTIME_ENV_ARGS".to_string(),
        template_render::render_runtime_env_args(&runtime_for_this_env),
    );
    vars.insert(
        "DOCKERFILE_ARGS".to_string(),
        template_render::render_dockerfile_args(&union_build_vec),
    );
    vars.insert(
        "DOCKERFILE_ENVS".to_string(),
        template_render::render_dockerfile_envs(&union_build_vec),
    );
    vars.insert(
        "DART_DEFINES".to_string(),
        template_render::render_dart_defines(&union_build_vec),
    );

    // Render each file from the template dir whose file_name is listed in file_targets
    let all_files = db.list_template_files(&target).map_err(|e| e.to_string())?;
    let mut rendered: Vec<RenderedFile> = Vec::new();
    for f in &all_files {
        let Some(target_path_tmpl) = file_targets.get(&f.file_name).and_then(|v| v.as_str()) else {
            continue;
        };
        let target_path = target_path_tmpl.replace("{name}", &env.name);
        let content = template_render::render_template(&f.content, &vars)?;
        rendered.push(RenderedFile {
            path: target_path,
            content,
        });
    }
    Ok(rendered)
}

#[tauri::command]
fn render_deploy_files_for_env(
    db: State<AppDb>,
    deploy_env_id: i64,
) -> Result<Vec<RenderedFile>, String> {
    render_files_for_deploy_env(&db, deploy_env_id)
}

#[tauri::command]
fn list_deploy_environments(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<DeployEnvironment>, String> {
    db.list_deploy_environments(repo_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_deploy_report(db: State<AppDb>) -> Result<Vec<DeployReportRow>, String> {
    db.list_deploy_report().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_deploy_environment(db: State<AppDb>, id: i64) -> Result<Option<DeployEnvironment>, String> {
    db.get_deploy_environment(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_deploy_environment(
    db: State<AppDb>,
    args: CreateDeployEnvironmentArgs,
) -> Result<DeployEnvironment, String> {
    validate_env_name(&args.name)?;
    db.insert_deploy_environment(&args)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clone_deploy_environment(
    db: State<AppDb>,
    source_id: i64,
    new_name: String,
) -> Result<DeployEnvironment, String> {
    validate_env_name(&new_name)?;
    db.clone_deploy_environment(source_id, &new_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_deploy_environment(
    db: State<AppDb>,
    args: UpdateDeployEnvironmentArgs,
) -> Result<DeployEnvironment, String> {
    db.update_deploy_environment(&args)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_deploy_environment(db: State<AppDb>, id: i64) -> Result<(), String> {
    db.delete_deploy_environment(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn reorder_deploy_environments(
    db: State<AppDb>,
    repo_id: i64,
    ordered_ids: Vec<i64>,
) -> Result<(), String> {
    db.reorder_deploy_environments(repo_id, &ordered_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_deploy_secrets(db: State<AppDb>, deploy_env_id: i64) -> Result<Vec<DeploySecret>, String> {
    db.list_deploy_secrets(deploy_env_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn upsert_deploy_secret(
    db: State<AppDb>,
    deploy_env_id: i64,
    secret_name: String,
    role: Option<String>,
    included: bool,
    override_enabled: bool,
) -> Result<(), String> {
    db.upsert_deploy_secret(
        deploy_env_id,
        &secret_name,
        role.as_deref(),
        included,
        override_enabled,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_deploy_secret(
    db: State<AppDb>,
    deploy_env_id: i64,
    secret_name: String,
) -> Result<(), String> {
    db.delete_deploy_secret(deploy_env_id, &secret_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn ensure_deploy_secrets_populated(
    db: State<AppDb>,
    deploy_env_id: i64,
    repo_secret_names: Vec<String>,
) -> Result<(), String> {
    // Parse meta.json to get hints
    let env = db
        .get_deploy_environment(deploy_env_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("deploy_env {} not found", deploy_env_id))?;
    let repo = db
        .get_repository(env.repository_id)
        .map_err(|e| e.to_string())?;
    let target = repo.deploy_target.clone().unwrap_or_default();
    let hints = if target.is_empty() {
        Vec::new()
    } else {
        parse_meta_secret_hints(&db, &target)?
    };
    db.ensure_deploy_secrets_populated(deploy_env_id, &repo_secret_names, &hints)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn register_repo_secret_in_deploys(
    db: State<AppDb>,
    repo_id: i64,
    secret_name: String,
) -> Result<(), String> {
    db.register_repo_secret_in_deploys(repo_id, &secret_name)
        .map_err(|e| e.to_string())
}

fn validate_env_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Environment name is required".to_string());
    }
    if name.len() > 255 {
        return Err("Environment name too long (max 255)".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Environment name must contain only letters, digits, hyphens and underscores"
                .to_string(),
        );
    }
    Ok(())
}

/// Parse `required_secrets` for a given template (`target` = `language_key`)
/// from the `templates` SQLite table. v0.31.0+ uses the strict parser in
/// `template_meta` — custom templates with the obsolete `"scope": "repo"`
/// secret value fail to load with a UI-friendly error.
fn parse_meta_secret_hints(db: &AppDb, target: &str) -> Result<Vec<MetaSecretHint>, String> {
    let meta_file = db
        .get_template_file(target, "meta.json")
        .map_err(|e| e.to_string())?;
    let Some(mf) = meta_file else {
        return Ok(Vec::new());
    };
    let meta: serde_json::Value =
        serde_json::from_str(&mf.content).map_err(|e| format!("Invalid meta.json: {}", e))?;
    template_meta::parse_meta_secret_hints(target, &meta)
}

/// Read a single file from a repo by its database id + relative path.
/// Returns `Ok(None)` if the repo has no `local_path`, the file doesn't exist,
/// or the contents aren't valid UTF-8. Returns `Err` only for DB errors.
/// Used by DeployScreen's `auto_detect` to pre-fill placeholders (e.g. GO_VERSION from go.mod).
#[tauri::command]
fn read_repo_file(
    db: State<AppDb>,
    repo_id: i64,
    rel_path: String,
) -> Result<Option<String>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(None);
    };
    let full = std::path::Path::new(&local_path).join(&rel_path);
    if !full.exists() {
        return Ok(None);
    }
    Ok(std::fs::read_to_string(&full).ok())
}

// ── F-021 Docs viewer commands ───────────────────────────────────────────────

#[tauri::command]
fn read_repo_todo(db: State<AppDb>, repo_id: i64) -> Result<ReadTodoResult, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(ReadTodoResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    };
    let full = std::path::Path::new(&local_path).join("docs/todo.md");
    if !full.exists() {
        return Ok(ReadTodoResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    }
    let content = std::fs::read_to_string(&full).map_err(|e| e.to_string())?;
    let (tasks, warnings) = export::parse_todo_tasks(&content);
    Ok(ReadTodoResult { tasks, warnings })
}

#[tauri::command]
fn read_repo_done(db: State<AppDb>, repo_id: i64) -> Result<ReadDoneResult, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(ReadDoneResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    };
    let full = std::path::Path::new(&local_path).join("docs/done.md");
    if !full.exists() {
        return Ok(ReadDoneResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    }
    let content = std::fs::read_to_string(&full).map_err(|e| e.to_string())?;
    let (tasks, warnings) = export::parse_done_tasks(&content);
    Ok(ReadDoneResult { tasks, warnings })
}

/// Debug/testing command: parse done.md for a single repo in a period.
/// Mainly for diagnostics; Dashboard aggregates this internally via read_dashboard.
#[tauri::command]
fn parse_done_entries_in_period_cmd(
    db: State<AppDb>,
    repo_id: i64,
    start: String,
    end: String,
) -> Result<Vec<(String, i64)>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    match repo.local_path {
        None => Ok(vec![]),
        Some(lp) => {
            let path = std::path::PathBuf::from(lp).join("docs").join("done.md");
            crate::export::parse_done_entries_in_period(&path, &start, &end)
                .map_err(|e| e.to_string())
        }
    }
}

#[tauri::command]
fn read_repo_files(
    db: State<AppDb>,
    repo_id: i64,
    rel_paths: Vec<String>,
) -> Result<Vec<Option<String>>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(rel_paths.iter().map(|_| None).collect());
    };
    let root = std::path::Path::new(&local_path);
    let mut result: Vec<Option<String>> = Vec::with_capacity(rel_paths.len());
    for rel in &rel_paths {
        // Reject `..`-escapes and absolute paths — symmetric with the
        // write_deploy_files guard so the read side respects the same
        // repo-root boundary the write side enforces.
        if !sync::is_safe_subpath(rel) {
            result.push(None);
            continue;
        }
        let p = root.join(rel);
        if p.exists() {
            match std::fs::read_to_string(&p) {
                Ok(s) => result.push(Some(s)),
                Err(_) => result.push(None),
            }
        } else {
            result.push(None);
        }
    }
    Ok(result)
}

#[tauri::command]
fn write_deploy_files(
    db: State<AppDb>,
    deploy_env_id: i64,
    repo_id: i64,
    local_path: String,
    files: Vec<RenderedFile>,
) -> Result<WriteResult, String> {
    let root = std::path::Path::new(&local_path);
    // Guard: never silently recreate a deleted/moved repo folder (B-001 invariant).
    sync::ensure_root_exists(root)?;

    let mut written: Vec<String> = Vec::new();
    let mut errors: Vec<WriteError> = Vec::new();
    for f in &files {
        // Path traversal guard: reject `..`, absolute paths, drive letters.
        // `f.path` ultimately comes from meta.json `file_targets`, which a user
        // can edit via TemplatesScreen; without this check a malicious template
        // could write outside the repo root.
        if !sync::is_safe_subpath(&f.path) {
            errors.push(WriteError {
                path: f.path.clone(),
                error: format!("unsafe path rejected: {}", f.path),
            });
            continue;
        }
        let target = root.join(&f.path);
        if let Some(parent) = target.parent() {
            if parent != root {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(WriteError {
                        path: f.path.clone(),
                        error: e.to_string(),
                    });
                    continue;
                }
            }
        }
        match std::fs::write(&target, &f.content) {
            Ok(_) => written.push(f.path.clone()),
            Err(e) => errors.push(WriteError {
                path: f.path.clone(),
                error: e.to_string(),
            }),
        }
    }

    // v0.20.0: record deploy event after successful file write.
    // H8 review-fix: report `written.len()` (actually written) rather than
    // `files.len()` (input total). Path-rejects + fs::write failures used
    // to be silently counted into the metric.
    let _ = db.insert_deploy_event(
        Some(deploy_env_id),
        repo_id,
        "render",
        &chrono::Utc::now().to_rfc3339(),
        Some(&serde_json::json!({ "file_count": written.len() }).to_string()),
    );

    Ok(WriteResult { written, errors })
}

// ── v0.20.0: Task sync commands ───────────────────────────────────────────────

#[tauri::command]
fn sync_tasks_for_repo_cmd(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<crate::sync::SyncTasksReport, String> {
    let result = crate::sync::sync_tasks_for_repo(&db, repo_id)?;
    if result.events_emitted > 0 || result.imported > 0 {
        let _ = db.insert_sync_event(
            Some(repo_id),
            "tasks",
            &chrono::Utc::now().to_rfc3339(),
            (result.events_emitted + result.imported) as i64,
            None,
        );
    }
    Ok(result)
}

#[tauri::command]
fn read_tasks_from_db(db: State<AppDb>, repo_id: i64) -> Result<Vec<crate::models::Task>, String> {
    db.list_tasks_by_repo(repo_id, "todo")
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn read_done_from_db(db: State<AppDb>, repo_id: i64) -> Result<Vec<crate::models::Task>, String> {
    db.list_tasks_by_repo(repo_id, "done")
        .map_err(|e| e.to_string())
}

// ── v0.20.0: Event recording commands (called from TS after GitHub API calls) ─

#[tauri::command]
fn record_deploy_secret_event(
    db: State<AppDb>,
    deploy_env_id: i64,
    repo_id: i64,
    action: String,
    secret_name: String,
) -> Result<(), String> {
    let details = serde_json::json!({ "name": secret_name }).to_string();
    db.insert_deploy_event(
        Some(deploy_env_id),
        repo_id,
        action.as_str(),
        &chrono::Utc::now().to_rfc3339(),
        Some(&details),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn record_secret_event(
    db: State<AppDb>,
    repo_id: i64,
    action: String,
    secret_name: String,
) -> Result<(), String> {
    let details = serde_json::json!({ "action": action, "name": secret_name }).to_string();
    db.insert_sync_event(
        Some(repo_id),
        "secret",
        &chrono::Utc::now().to_rfc3339(),
        1,
        Some(&details),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn read_timeline(
    db: State<AppDb>,
    filter: crate::models::TimelineFilter,
    offset: u32,
    limit: u32,
) -> Result<Vec<crate::models::ActivityEvent>, String> {
    db.read_timeline_filtered(&filter, offset, limit)
        .map_err(|e| e.to_string())
}

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = AppDb::new(get_db_path()).expect("Failed to initialize database");

    // T-000063: copy legacy PAT from old keyring service to new one. Idempotent;
    // only fires when new service has no entry yet.
    keyring_store::migrate_legacy_pat();

    // Seed bundled templates (e.g. flutter_web) if language is missing in DB.
    if let Err(e) = template_seeder::seed_bundled_templates(&db) {
        eprintln!("Warning: template seeding failed: {}", e);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|_app| {
            // B-000017 v5 (reverted v3+v4 set_icon override): comparing
            // against a sibling Tauri-2 + Svelte app (MySafeSpace) on the
            // same Win11 high-DPI display revealed that the default Tauri
            // window-icon path (NO programmatic set_icon) renders sharp
            // because Tauri/tao does pick the right frame from icon.ico's
            // multi-frame resource for each context (taskbar / alt-tab /
            // title). Earlier explicit set_icon attempts (v1=64×64 PNG,
            // v3=hub-spokes PNG, v4=square-frame PNG) forced a single RGBA
            // bitmap on every DPI context — Windows then downscaled with
            // poor filtering on non-integer ratios. Removing the override
            // restores multi-frame ICO behaviour for free.
            //
            // The original B-000010 comment claimed default Tauri picked
            // "the largest frame and downscaled it" — that was likely true
            // of an earlier tao version; current Tauri 2.x handles
            // multi-frame ICO correctly. icon.ico already has the right
            // frames (16/20/24/32/40/48/64/96/128/256, SDH-crop on small,
            // full logo on large) per the B-000010 rebuild — no changes
            // needed there.
            //
            // Does NOT affect the .exe file icon either way (still
            // multi-frame icon.ico via tauri-bundler's embedded resource).
            //
            // Generator + explored override variants left at
            // docs/superpowers/plans/2026-05-24-sdh-icon-v2.html for
            // history.
            Ok(())
        })
        .manage(db)
        .invoke_handler(tauri::generate_handler![
            // Projects
            create_project,
            list_projects,
            update_project,
            delete_project,
            // Repositories
            create_local_repository,
            upsert_repository,
            resolve_merge_with_local,
            force_insert_github_repo,
            assign_repository,
            reorder_project,
            reorder_repo,
            rebalance_repo_group,
            rebalance_projects,
            auto_sort_all,
            list_repos_by_project,
            list_all_repos,
            get_repository,
            get_repository_by_name,
            // PAT / Keyring
            store_pat,
            get_pat,
            delete_pat,
            // local_path
            set_repo_local_path,
            update_repo_description,
            // Repo deletion (B-003)
            delete_repository,
            // F-000041: untrack gitignored files
            check_git_available_for_repo,
            list_gitignored_tracked,
            untrack_files,
            // Workspace scanner
            scan_workspace_for_repos,
            // File-based bugs
            read_bugs_from_file,
            write_bugs_to_file,
            // Bugs (v0.16.0, SQLite SoT)
            ensure_bugs_migrated,
            reconcile_bugs_for_repo,
            reconcile_all_projects,
            read_bugs_from_db,
            count_confirmed_bugs,
            create_bug,
            update_bug_fields,
            delete_bug,
            resolve_bug,
            reject_bug,
            reopen_bug,
            // Microservice connections
            connect_microservice,
            disconnect_microservice,
            list_project_microservices,
            list_microservice_projects,
            list_parents_of_microservice,
            update_project_type,
            server_repo_of_microservice,
            // Settings
            get_setting,
            set_setting,
            // Stats / Graph
            get_repo_stats_summary,
            get_project_stats_summary,
            get_project_graph,
            // Dashboard v0.17.0
            read_dashboard,
            // Activity feed v0.19.0
            read_recent_activity,
            // Timeline v0.20.0
            read_timeline,
            // Requirements sync
            sync_global_claude_md,
            sync_project,
            init_docs_for_repo,
            list_project_requirements,
            confirm_requirement,
            // Rename log (F-033)
            list_rename_history,
            list_renames_for_repo,
            // Templates (0.6.0)
            list_template_languages,
            list_template_files,
            get_template_file,
            save_template_file,
            reset_template_file,
            // Deploy (0.7.0 / v0.18.0 multi-env)
            set_deploy_target,
            get_repo_deploy_config,
            set_repo_deploy_config,
            render_deploy_files_for_env,
            list_deploy_environments,
            list_deploy_report,
            get_deploy_environment,
            create_deploy_environment,
            clone_deploy_environment,
            update_deploy_environment,
            delete_deploy_environment,
            reorder_deploy_environments,
            list_deploy_secrets,
            upsert_deploy_secret,
            delete_deploy_secret,
            ensure_deploy_secrets_populated,
            register_repo_secret_in_deploys,
            read_repo_file,
            read_repo_files,
            read_repo_todo,
            read_repo_done,
            parse_done_entries_in_period_cmd,
            write_deploy_files,
            sync_tasks_for_repo_cmd,
            read_tasks_from_db,
            read_done_from_db,
            record_secret_event,
            record_deploy_secret_event,
            // Secret bundles (v1.3.0)
            list_secret_bundles,
            create_secret_bundle,
            rename_secret_bundle,
            delete_secret_bundle,
            upsert_bundle_item,
            delete_bundle_item,
            get_bundle_decrypted,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod render_deploy_tests {
    use super::*;
    use crate::db::AppDb;
    use crate::models::CreateDeployEnvironmentArgs;
    use crate::template_seeder::seed_bundled_templates;

    fn setup() -> (AppDb, i64, i64) {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        std::mem::forget(tmp);
        seed_bundled_templates(&db).unwrap();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r", "r", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("go")).unwrap();
        // T-000103 Task 3: repo-scope placeholders (GO_VERSION, BINARY_NAME,
        // ENTRY_POINT, APP_PORT) now live in `repositories.deploy_repo_config`,
        // not per-env `extras`. They bake into the single repo-wide Dockerfile.
        let repo_config: std::collections::HashMap<String, String> = [
            ("GO_VERSION", "1.23"),
            ("BINARY_NAME", "app"),
            ("ENTRY_POINT", "./cmd/api/"),
            ("APP_PORT", "8080"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        db.set_repo_deploy_config(repo.id, &repo_config).unwrap();
        let env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo.id,
                name: "prod".to_string(),
                workflow_name: "Deploy".to_string(),
                image_tag: "latest".to_string(),
                compose_service: "backend".to_string(),
                domain: "x.com".to_string(),
                deploy_branch: "master".to_string(),
                extras: {
                    // Env-scope placeholders only — repo-scope ones moved to repo_config above.
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m.insert("NETWORK_NAME".to_string(), "app_prod_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_prod".to_string());
                    m
                },
            })
            .unwrap();
        (db, repo.id, env.id)
    }

    #[test]
    fn test_render_for_env_produces_deploy_yml_with_env_name() {
        let (db, _repo, env_id) = setup();
        // Seed 1 runtime secret for this env
        db.upsert_deploy_secret(env_id, "DATABASE_URL", Some("runtime"), true, true)
            .unwrap();
        let files = render_files_for_deploy_env(&db, env_id).unwrap();

        let deploy_yml = files
            .iter()
            .find(|f| f.path == ".github/workflows/deploy-prod.yml")
            .expect("deploy-prod.yml must be produced");
        assert!(deploy_yml.content.contains("environment: prod"));
        assert!(deploy_yml
            .content
            .contains("--env DATABASE_URL=\"${{ secrets.DATABASE_URL }}\""));
        assert!(deploy_yml.content.contains("--network app_prod_net"));
        assert!(deploy_yml
            .content
            .contains("com.docker.compose.project=app_prod"));
    }

    #[test]
    fn test_render_multiple_envs_produces_separate_workflow_files() {
        let (db, repo_id, prod_id) = setup();
        // T-000103 Task 3: repo-scope keys live in deploy_repo_config (seeded
        // by setup() for both envs of this repo). Only env-scope keys go in
        // each env's `extras`.
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m
                },
            })
            .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        assert!(prod_files
            .iter()
            .any(|f| f.path == ".github/workflows/deploy-prod.yml"));
        assert!(test_files
            .iter()
            .any(|f| f.path == ".github/workflows/deploy-test.yml"));
    }

    #[test]
    fn test_multi_env_go_isolation_per_env_values_baked_in() {
        // v0.29.0 multi-deploy smoke: same Go repo, two envs, each rendered
        // workflow file must contain its own env-specific values and NOT leak
        // values from the other env.
        // T-000103 Task 3: repo-scope keys live in deploy_repo_config (seeded
        // by setup()). Only env-scope keys go in `extras`.
        let (db, repo_id, prod_id) = setup();
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m
                },
            })
            .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        let prod_yml = &prod_files
            .iter()
            .find(|f| f.path.ends_with("deploy-prod.yml"))
            .unwrap()
            .content;
        let test_yml = &test_files
            .iter()
            .find(|f| f.path.ends_with("deploy-test.yml"))
            .unwrap()
            .content;

        // Prod-specific values present in prod, absent from test.
        assert!(prod_yml.contains("environment: prod"));
        assert!(prod_yml.contains("--network app_prod_net"));
        assert!(prod_yml.contains("com.docker.compose.project=app_prod"));
        assert!(prod_yml.contains("branches: [ master ]"));
        assert!(prod_yml.contains("DOMAIN=x.com"));
        assert!(
            !prod_yml.contains("app_test_net"),
            "prod must not leak test network"
        );
        assert!(
            !prod_yml.contains("test.x.com"),
            "prod must not leak test domain"
        );

        // Test-specific values present in test, absent from prod.
        assert!(test_yml.contains("environment: test"));
        assert!(test_yml.contains("--network app_test_net"));
        assert!(test_yml.contains("com.docker.compose.project=app_test"));
        assert!(test_yml.contains("branches: [ dev ]"));
        assert!(test_yml.contains("DOMAIN=test.x.com"));
        assert!(
            !test_yml.contains("app_prod_net"),
            "test must not leak prod network"
        );
        assert!(
            !test_yml.contains("DOMAIN=x.com\n"),
            "test must not leak prod domain"
        );
    }

    #[test]
    fn test_multi_env_go_runtime_secrets_per_env_isolation() {
        // Each env's runtime secrets must appear only in that env's deploy.yml.
        // T-000103 Task 3: APP_PORT lives in deploy_repo_config (seeded by setup()).
        let (db, repo_id, prod_id) = setup();
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m
                },
            })
            .unwrap();

        // Per-env runtime secrets: prod gets DATABASE_URL_PROD, test gets DATABASE_URL_TEST.
        db.upsert_deploy_secret(prod_id, "DATABASE_URL_PROD", Some("runtime"), true, true)
            .unwrap();
        db.upsert_deploy_secret(
            test_env.id,
            "DATABASE_URL_TEST",
            Some("runtime"),
            true,
            true,
        )
        .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        let prod_yml = &prod_files
            .iter()
            .find(|f| f.path.ends_with("deploy-prod.yml"))
            .unwrap()
            .content;
        let test_yml = &test_files
            .iter()
            .find(|f| f.path.ends_with("deploy-test.yml"))
            .unwrap()
            .content;

        assert!(
            prod_yml.contains("DATABASE_URL_PROD"),
            "prod must reference its own runtime secret"
        );
        assert!(
            !prod_yml.contains("DATABASE_URL_TEST"),
            "prod must not leak test runtime secret"
        );
        assert!(
            test_yml.contains("DATABASE_URL_TEST"),
            "test must reference its own runtime secret"
        );
        assert!(
            !test_yml.contains("DATABASE_URL_PROD"),
            "test must not leak prod runtime secret"
        );
    }

    #[test]
    fn test_multi_env_go_shared_dockerfile_identical_across_envs() {
        // Go's Dockerfile uses only repo-wide placeholders (GO_VERSION,
        // BINARY_NAME, ENTRY_POINT, APP_PORT) and NOT env-specific
        // DOCKERFILE_ARGS (that's a Flutter-specific concept — Go binaries are
        // statically linked, secrets are runtime-injected via docker --env).
        // So when those repo-wide values match, the rendered Dockerfile must
        // be byte-identical regardless of which env triggers the render.
        // T-000103 Task 3: repo-wide values now live in `deploy_repo_config`
        // (seeded once by setup() — shared by ALL envs of the repo by design,
        // so identical-rendered-Dockerfile becomes a structural guarantee, not
        // a coincidence-of-matching-extras).
        let (db, repo_id, prod_id) = setup();
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    // Env-specific values differ from prod — must NOT affect Dockerfile.
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m
                },
            })
            .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        let prod_dockerfile = &prod_files
            .iter()
            .find(|f| f.path == "Dockerfile")
            .unwrap()
            .content;
        let test_dockerfile = &test_files
            .iter()
            .find(|f| f.path == "Dockerfile")
            .unwrap()
            .content;
        assert_eq!(
            prod_dockerfile, test_dockerfile,
            "Go Dockerfile is repo-wide; identical extras must render identical Dockerfile"
        );
    }

    #[test]
    fn test_validate_env_name_accepts_valid() {
        assert!(super::validate_env_name("prod").is_ok());
        assert!(super::validate_env_name("test-1").is_ok());
        assert!(super::validate_env_name("staging_v2").is_ok());
    }

    #[test]
    fn test_validate_env_name_rejects_invalid() {
        assert!(super::validate_env_name("").is_err());
        assert!(super::validate_env_name("has space").is_err());
        assert!(super::validate_env_name("dot.name").is_err());
        assert!(super::validate_env_name("slash/name").is_err());
        assert!(super::validate_env_name(&"x".repeat(256)).is_err());
    }
}
