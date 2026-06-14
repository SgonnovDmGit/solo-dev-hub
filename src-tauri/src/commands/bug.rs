use crate::db::AppDb;
use crate::models::*;
use crate::{db, export, sync};
use chrono;
use tauri::State;

// ── File-based bug read/write ─────────────────────────────────────────────────

#[tauri::command]
pub fn read_bugs_from_file(file_path: String) -> Result<ReadBugsResult, String> {
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
pub fn write_bugs_to_file(
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

// ── Bugs (v0.16.0, SQLite SoT) ───────────────────────────────────────────────

/// Idempotent lazy MD→DB migration for a repo. Call BEFORE reconcile.
/// Returns report with imported/archived counts; `already=true` means it
/// was already migrated on a prior call (no-op).
#[tauri::command]
pub fn ensure_bugs_migrated(db: State<AppDb>, repo_id: i64) -> Result<MigrationReport, String> {
    sync::migrate_bugs_for_repo(&db, repo_id)
}

/// 2-way sync MD ↔ DB: ingest LLM-edited status/comment from MD, silently
/// correct protected-field mismatches and restore deleted rows via regen.
/// Caller must have migrated the repo first — returns Err otherwise.
#[tauri::command]
pub fn reconcile_bugs_for_repo(db: State<AppDb>, repo_id: i64) -> Result<(), String> {
    sync::reconcile_bugs_for_repo(&db, repo_id)
}

#[derive(serde::Serialize)]
pub struct ReconcileAllReport {
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
pub fn reconcile_all_projects(db: State<AppDb>) -> Result<ReconcileAllReport, String> {
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
pub fn read_bugs_from_db(
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
pub fn count_confirmed_bugs(db: State<AppDb>, repo_id: i64) -> Result<i64, String> {
    db.count_confirmed_bugs(repo_id).map_err(|e| e.to_string())
}

/// Create a new bug via app UI (+ Add button). Starts in `created` status
/// with `fix_attempts=0`. `numeric_id` auto-allocated as max+1 per-repo.
/// Regenerates `docs/bug-reports.md` from DB so the new row is visible to LLM.
#[tauri::command]
pub fn create_bug(
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
pub fn resolve_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<BugView, String> {
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
pub fn update_bug_fields(
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
pub fn delete_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<(), String> {
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
pub fn reject_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<BugView, String> {
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
pub fn reopen_bug(db: State<AppDb>, repo_id: i64, display_id: String) -> Result<BugView, String> {
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
