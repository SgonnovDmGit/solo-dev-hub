// v0.20.0: todo.md / done.md ↔ DB sync for the `tasks` table.
//
// Algorithm overview lives on `sync_tasks_for_repo`. SoT distinction is
// reversed from bugs: for tasks, the **MD files are canonical**, the DB
// is a mirror that powers Timeline / Stats. The mirror is rebuilt by
// re-parsing MD each sync, with task_events emitted only on observed
// status transitions (and suppressed on the first migration).

use crate::db::AppDb;
use crate::export;
use std::path::Path;

#[derive(Debug, serde::Serialize)]
pub struct SyncTasksReport {
    pub imported: u32,
    pub events_emitted: u32,
}

/// Sync todo.md + done.md from disk into the `tasks` DB table for the given repo.
///
/// Algorithm:
/// 1. Parse todo.md and done.md from disk.
/// 2. Compare against existing `tasks` rows in DB (keyed by task_id string).
/// 3. New tasks → INSERT. Status changes → UPDATE + event. todo→done move → UPDATE source + event.
/// 4. First-sync semantics: if `tasks_migrated_at IS NULL` for this repo, suppress all
///    "created" events (silent backfill of legacy data). Mark migrated after.
///
/// Returns `SyncTasksReport` with counts of imported rows and emitted events.
pub fn sync_tasks_for_repo(db: &AppDb, repo_id: i64) -> Result<SyncTasksReport, String> {
    use std::collections::{HashMap, HashSet};

    /// Drop repeated ids within one MD file, keeping the LAST occurrence.
    ///
    /// T-000157: the DB maps below are built once, before the loops, and are
    /// never refreshed with rows inserted during the same pass. So a duplicate
    /// id inside a single file took the INSERT branch twice and hit
    /// `UNIQUE(repository_id, task_id, source)`, aborting the whole repo's
    /// sync (and leaving `tasks_migrated_at` unset, so it re-failed on every ↻).
    /// Real-world trigger: the same task listed under two `## vX.Y.Z` headers
    /// after a re-plan that didn't delete the old row. Last wins — the later
    /// header is the more recent intent, mirroring the done-beats-todo rule below.
    fn dedupe_by_id<T>(
        items: Vec<T>,
        id_of: impl Fn(&T) -> &str,
        what: &str,
        repo_id: i64,
    ) -> Vec<T> {
        let mut last_index: HashMap<String, usize> = HashMap::new();
        for (i, it) in items.iter().enumerate() {
            last_index.insert(id_of(it).to_string(), i);
        }
        if last_index.len() == items.len() {
            return items;
        }
        items
            .into_iter()
            .enumerate()
            .filter(|(i, it)| {
                let keep = last_index.get(id_of(it)) == Some(i);
                if !keep {
                    eprintln!(
                        "[sync_tasks repo={}] duplicate id {} in {} — keeping the last occurrence",
                        repo_id,
                        id_of(it),
                        what
                    );
                }
                keep
            })
            .map(|(_, it)| it)
            .collect()
    }

    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;

    let local_path = match repo.local_path.clone() {
        Some(p) => p,
        None => {
            db.mark_tasks_migrated(repo_id, &chrono::Utc::now().to_rfc3339())
                .map_err(|e| e.to_string())?;
            return Ok(SyncTasksReport {
                imported: 0,
                events_emitted: 0,
            });
        }
    };

    let todo_path = Path::new(&local_path).join("docs").join("todo.md");
    let done_path = Path::new(&local_path).join("docs").join("done.md");

    // Determine whether this is a first sync (suppress created events for legacy backfill)
    let was_migrated = db
        .get_tasks_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_some();
    let suppress_created_events = !was_migrated;

    // Read and parse todo.md
    let (todo_tasks, todo_mtime) = if todo_path.exists() {
        let content =
            std::fs::read_to_string(&todo_path).map_err(|e| format!("read todo.md: {}", e))?;
        let mtime = std::fs::metadata(&todo_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
        let (tasks, _warnings) = export::parse_todo_tasks(&content);
        (tasks, mtime)
    } else {
        (Vec::new(), String::new())
    };

    // Read and parse done.md. `done_mtime` is used as the fallback date for
    // historical done entries whose `## YYYY-MM-DD` section header is
    // missing — `todo_mtime` was wrong here because the entry came from
    // done.md, not todo.md (review H5).
    let (done_tasks, done_mtime) = if done_path.exists() {
        let content =
            std::fs::read_to_string(&done_path).map_err(|e| format!("read done.md: {}", e))?;
        let mtime = std::fs::metadata(&done_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                chrono::DateTime::<chrono::Utc>::from(t)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
        let (tasks, _warnings) = export::parse_done_tasks(&content);
        (tasks, mtime)
    } else {
        (Vec::new(), String::new())
    };

    // T-000157: collapse duplicate ids before diffing against the DB.
    let todo_tasks = dedupe_by_id(todo_tasks, |t| t.id.as_str(), "todo.md", repo_id);
    let done_tasks = dedupe_by_id(done_tasks, |t| t.id.as_str(), "done.md", repo_id);

    // Load existing DB rows keyed by task_id string
    let db_todos = db
        .list_tasks_by_repo(repo_id, "todo")
        .map_err(|e| e.to_string())?;
    let db_dones = db
        .list_tasks_by_repo(repo_id, "done")
        .map_err(|e| e.to_string())?;
    let db_todo_by_id: HashMap<String, _> = db_todos
        .iter()
        .map(|t| (t.task_id.clone(), t.clone()))
        .collect();
    let db_done_by_id: HashMap<String, _> = db_dones
        .iter()
        .map(|t| (t.task_id.clone(), t.clone()))
        .collect();

    let mut imported = 0u32;
    let mut events_emitted = 0u32;

    // ── Process TODO entries ─────────────────────────────────────────────────
    for tt in &todo_tasks {
        let prefix = if tt.id.starts_with("T-") {
            "T"
        } else if tt.id.starts_with("F-") {
            "F"
        } else {
            continue; // Skip unknown prefixes
        };

        let created_at = if tt.created_at.is_empty() {
            todo_mtime.clone()
        } else {
            tt.created_at.clone()
        };

        if let Some(existing) = db_todo_by_id.get(&tt.id) {
            // T-000109: keep the DB `version` column synced with the current
            // `## vX.Y.Z` section header above this task in todo.md. User may
            // move a task between version sections — that's a regular flow.
            let new_version_opt = if tt.version.is_empty() {
                None
            } else {
                Some(tt.version.as_str())
            };
            if new_version_opt != existing.version.as_deref() {
                db.update_task_version(existing.id, new_version_opt)
                    .map_err(|e| e.to_string())?;
            }
            // Row exists in DB as todo — check for status change
            let new_status = if tt.status.is_empty() {
                None
            } else {
                Some(tt.status.as_str())
            };
            let old_status = existing.status.as_deref();
            if new_status != old_status {
                let event_type = match (old_status, new_status) {
                    (Some("open"), Some("in-progress")) => "taken",
                    (Some("in-progress"), Some("review")) => "review",
                    (Some("review"), Some("open")) | (Some("done"), Some("in-progress")) => {
                        "reopened"
                    }
                    _ => {
                        // Unusual transition — update status but emit no event
                        eprintln!(
                            "[sync_tasks repo={}] unusual status transition: {:?} -> {:?} for {}",
                            repo_id, old_status, new_status, tt.id
                        );
                        ""
                    }
                };
                db.update_task_status(existing.id, new_status)
                    .map_err(|e| e.to_string())?;
                if !event_type.is_empty() {
                    db.insert_task_event(
                        existing.id,
                        event_type,
                        &chrono::Utc::now().to_rfc3339(),
                        old_status,
                        new_status,
                    )
                    .map_err(|e| e.to_string())?;
                    events_emitted += 1;
                }
            }
        } else if let Some(existing_done) = db_done_by_id.get(&tt.id) {
            // Was in done in DB but reappeared in todo — reopened
            db.update_task_source(existing_done.id, "todo")
                .map_err(|e| e.to_string())?;
            db.update_task_status(existing_done.id, Some(tt.status.as_str()))
                .map_err(|e| e.to_string())?;
            db.insert_task_event(
                existing_done.id,
                "reopened",
                &chrono::Utc::now().to_rfc3339(),
                None,
                Some(tt.status.as_str()),
            )
            .map_err(|e| e.to_string())?;
            events_emitted += 1;
        } else {
            // New task — insert
            let effort = tt.effort.parse::<f64>().ok();
            // T-000109: `## vX.Y.Z` section-header version inherited by the parser.
            let version_opt = if tt.version.is_empty() {
                None
            } else {
                Some(tt.version.as_str())
            };
            let row = db
                .insert_task(
                    repo_id,
                    &tt.id,
                    prefix,
                    &tt.description,
                    effort,
                    if tt.priority.is_empty() {
                        None
                    } else {
                        Some(tt.priority.as_str())
                    },
                    if tt.status.is_empty() {
                        None
                    } else {
                        Some(tt.status.as_str())
                    },
                    version_opt,
                    "todo",
                    &created_at,
                )
                .map_err(|e| e.to_string())?;
            imported += 1;

            if !suppress_created_events {
                let to_status = if tt.status.is_empty() {
                    None
                } else {
                    Some(tt.status.as_str())
                };
                db.insert_task_event(
                    row.id,
                    "created",
                    &chrono::Utc::now().to_rfc3339(),
                    None,
                    to_status,
                )
                .map_err(|e| e.to_string())?;
                events_emitted += 1;
            }
        }
    }

    // ── Process DONE entries ─────────────────────────────────────────────────
    // Build set of todo ids seen in MD (for detecting todo→done moves)
    let _md_todo_ids: HashSet<&str> = todo_tasks.iter().map(|t| t.id.as_str()).collect();

    for dt in &done_tasks {
        let prefix = if dt.id.starts_with("T-") {
            "T"
        } else if dt.id.starts_with("F-") {
            "F"
        } else if dt.id.starts_with("D-") {
            "D"
        } else {
            continue;
        };

        if db_done_by_id.contains_key(&dt.id) {
            // Already in done — skip (idempotent)
        } else if let Some(was_in_todo) = db_todo_by_id.get(&dt.id) {
            // Was in todo in DB, now in done in MD — task completed
            db.update_task_source(was_in_todo.id, "done")
                .map_err(|e| e.to_string())?;
            db.update_task_status(was_in_todo.id, None)
                .map_err(|e| e.to_string())?;
            db.insert_task_event(
                was_in_todo.id,
                "done",
                &chrono::Utc::now().to_rfc3339(),
                was_in_todo.status.as_deref(),
                None,
            )
            .map_err(|e| e.to_string())?;
            events_emitted += 1;
        } else {
            // Brand new done entry (historical task, never seen before in DB).
            // Use done.md mtime as the fallback — the entry originated there,
            // not in todo.md (which may have been touched much later).
            let fallback_date = if done_mtime.is_empty() {
                chrono::Utc::now().format("%Y-%m-%d").to_string()
            } else {
                done_mtime.clone()
            };
            let row = db
                .insert_task(
                    repo_id,
                    &dt.id,
                    prefix,
                    &dt.description,
                    None, // no effort for done tasks
                    None, // no priority
                    None, // no active status for done tasks
                    Some(dt.version.as_str()),
                    "done",
                    if dt.date.is_empty() {
                        &fallback_date
                    } else {
                        &dt.date
                    },
                )
                .map_err(|e| e.to_string())?;
            imported += 1;

            if !suppress_created_events {
                db.insert_task_event(row.id, "done", &chrono::Utc::now().to_rfc3339(), None, None)
                    .map_err(|e| e.to_string())?;
                events_emitted += 1;
            }
        }
    }

    // ── Cleanup orphan todo rows (in DB but absent from MD) ──────────────────
    // Fixes B-000004: when LLM normalises an ID in todo.md (e.g. T-034 → T-000034
    // or placeholder "F-NNN" → real "F-000035"), the old DB row used to stick
    // around as a duplicate forever. todo.md is canonical for tasks, so any DB
    // row whose task_id is no longer in MD is an orphan and gets dropped here.
    // task_events cascade via FK. Done rows are append-only and untouched.
    let md_todo_ids: HashSet<&str> = todo_tasks.iter().map(|t| t.id.as_str()).collect();
    let db_todos_now = db
        .list_tasks_by_repo(repo_id, "todo")
        .map_err(|e| e.to_string())?;
    for t in &db_todos_now {
        if !md_todo_ids.contains(t.task_id.as_str()) {
            db.delete_task(t.id).map_err(|e| e.to_string())?;
        }
    }

    // H6 review-fix: resolve split-state where the same task_id exists in
    // both `todo` and `done` source for the same repo. The UNIQUE constraint
    // is `(repository_id, task_id, source)` (not `task_id` alone), so this
    // state is reachable after a crash mid-transition or a manual MD edit
    // listing the same id in both files. Done is the more recent intent —
    // drop the todo duplicate. Without this the user would see the task
    // simultaneously in both Tasks and Done tabs.
    let db_todos_now = db
        .list_tasks_by_repo(repo_id, "todo")
        .map_err(|e| e.to_string())?;
    let db_dones_now = db
        .list_tasks_by_repo(repo_id, "done")
        .map_err(|e| e.to_string())?;
    let done_ids: HashSet<&str> = db_dones_now.iter().map(|t| t.task_id.as_str()).collect();
    for t in &db_todos_now {
        if done_ids.contains(t.task_id.as_str()) {
            db.delete_task(t.id).map_err(|e| e.to_string())?;
        }
    }

    db.mark_tasks_migrated(repo_id, &chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;

    Ok(SyncTasksReport {
        imported,
        events_emitted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db_for_sync_tests() -> AppDb {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        std::mem::forget(tmp);
        AppDb::new(path).unwrap()
    }

    #[test]
    fn test_sync_tasks_first_run_inserts_rows_no_events() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task A | 2 | high | open\n- [ ] T-002 | Task B | 4 | medium | in-progress\n",
        ).unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "test_repo", None, None)
            .unwrap();

        let report = sync_tasks_for_repo(&db, repo.id).unwrap();

        assert_eq!(report.imported, 2);
        assert_eq!(
            report.events_emitted, 0,
            "first sync must not emit 'created' events"
        );

        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos.len(), 2);
        let events = db.list_task_events_by_task(todos[0].id).unwrap();
        assert!(events.is_empty());
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_idempotent_no_changes_no_events() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task A | 2 | high | open\n",
        )
        .unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "test_repo", None, None)
            .unwrap();

        sync_tasks_for_repo(&db, repo.id).unwrap(); // first
        let r2 = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.events_emitted, 0);
        std::mem::forget(tmp);
    }

    /// T-000157: the same id under two `## vX.Y.Z` headers used to blow up on
    /// `UNIQUE(repository_id, task_id, source)` and abort the repo's sync.
    /// Now the last occurrence wins and the sync completes.
    #[test]
    fn test_sync_tasks_duplicate_id_in_todo_keeps_last() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "## v1.0.0\n\n- [ ] T-000083 | Task | 4 | medium | open | 2026-07-14\n\n\
             ## v1.1.0\n\n- [ ] T-000083 | Task | 4 | medium | open | 2026-07-14\n\
             - [ ] T-000088 | Other | 2 | low | open | 2026-07-14\n",
        )
        .unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "dup_repo", None, None)
            .unwrap();

        let report = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(
            report.imported, 2,
            "duplicate id must be collapsed to one row"
        );

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos.len(), 2);
        let dup = todos.iter().find(|t| t.task_id == "T-000083").unwrap();
        assert_eq!(
            dup.version.as_deref(),
            Some("v1.1.0"),
            "last occurrence wins — the later version header"
        );
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_no_todo_md_marks_migrated() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = db
            .insert_local_repository(tmp.path().to_str().unwrap(), "test_repo", None, None)
            .unwrap();

        let report = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(report.imported, 0);
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_open_to_inprogress_emits_taken() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | open\n",
        )
        .unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None)
            .unwrap();

        sync_tasks_for_repo(&db, repo.id).unwrap();

        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | in-progress\n",
        )
        .unwrap();

        let r = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r.events_emitted, 1);

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        let events = db.list_task_events_by_task(todos[0].id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "taken");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_todo_to_done_emits_done() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | review\n",
        )
        .unwrap();
        std::fs::write(repo_path.join("docs/done.md"), "").unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None)
            .unwrap();

        sync_tasks_for_repo(&db, repo.id).unwrap();

        std::fs::write(repo_path.join("docs/todo.md"), "").unwrap();
        std::fs::write(
            repo_path.join("docs/done.md"),
            "## 2026-04-26\n- T-001 | Task | v0.20.0\n",
        )
        .unwrap();

        let r = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r.events_emitted, 1);

        let dones = db.list_tasks_by_repo(repo.id, "done").unwrap();
        assert_eq!(dones.len(), 1);
        let events = db.list_task_events_by_task(dones[0].id).unwrap();
        let last = events.last().unwrap();
        assert_eq!(last.event_type, "done");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_unusual_transition_no_event() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | open\n",
        )
        .unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None)
            .unwrap();

        sync_tasks_for_repo(&db, repo.id).unwrap();

        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | review\n",
        )
        .unwrap();

        let r = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(
            r.events_emitted, 0,
            "unusual transitions log warn but emit no event"
        );

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos[0].status.as_deref(), Some("review"));
        std::mem::forget(tmp);
    }

    /// B-000004: when an ID in todo.md is rewritten (3-digit T-034 → 6-digit
    /// T-000034, or placeholder "F-NNN" → real "F-000035"), the old DB row
    /// must be dropped on the next sync — otherwise the same task shows up
    /// twice in the Tasks tab forever.
    #[test]
    fn test_sync_tasks_cleans_up_orphan_todo_rows() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-034 | Old format task | 2 | high | open\n- [ ] F-NNN | Placeholder feature | 4 | medium | open\n",
        ).unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None)
            .unwrap();

        // First sync: 2 rows imported with the original (legacy / placeholder) ids.
        sync_tasks_for_repo(&db, repo.id).unwrap();
        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos.len(), 2);

        // Rewrite todo.md with normalised ids (LLM did the cleanup).
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-000034 | Old format task | 2 | high | open\n- [ ] F-000035 | Placeholder feature | 4 | medium | open\n",
        ).unwrap();

        sync_tasks_for_repo(&db, repo.id).unwrap();

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(
            todos.len(),
            2,
            "orphan rows with old ids must be cleaned up"
        );
        let ids: std::collections::HashSet<&str> =
            todos.iter().map(|t| t.task_id.as_str()).collect();
        assert!(ids.contains("T-000034"));
        assert!(ids.contains("F-000035"));
        assert!(!ids.contains("T-034"), "old 3-digit row must be gone");
        assert!(!ids.contains("F-NNN"), "placeholder row must be gone");
        std::mem::forget(tmp);
    }

    /// H6 review-fix: split-state where the same task_id ended up in both
    /// `todo` and `done` source (e.g. after a crash mid-transition or a
    /// manual MD edit listing it in both files) is resolved by dropping the
    /// `todo` duplicate. The user would otherwise see the task in both tabs.
    #[test]
    fn test_sync_tasks_resolves_todo_done_split_state() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        // Initial state: task in todo only.
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-000042 | Test task | 1 | high | open\n",
        )
        .unwrap();
        std::fs::write(repo_path.join("docs/done.md"), "").unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None)
            .unwrap();
        sync_tasks_for_repo(&db, repo.id).unwrap();

        // Simulate split-state: task_id present in both DB sources. Direct
        // INSERT bypasses normal update_task_source to mimic the post-crash
        // pathological state.
        let now = chrono::Utc::now().to_rfc3339();
        db.insert_task(
            repo.id,
            "T-000042",
            "T",
            "Test task done",
            None,
            None,
            None,
            None,
            "done",
            &now,
        )
        .unwrap();
        assert_eq!(db.list_tasks_by_repo(repo.id, "todo").unwrap().len(), 1);
        assert_eq!(db.list_tasks_by_repo(repo.id, "done").unwrap().len(), 1);

        // Next sync should drop the todo duplicate.
        sync_tasks_for_repo(&db, repo.id).unwrap();

        assert_eq!(
            db.list_tasks_by_repo(repo.id, "todo").unwrap().len(),
            0,
            "todo duplicate must be removed when done row exists"
        );
        assert_eq!(
            db.list_tasks_by_repo(repo.id, "done").unwrap().len(),
            1,
            "done row must survive"
        );
        std::mem::forget(tmp);
    }
}
