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

/// Timestamp for a `done` task event, derived from the completion day in
/// done.md's `## YYYY-MM-DD` section header (`date`).
///
/// T-000202: the event used to carry `Utc::now()` at sync time. Measured on the
/// owner's DB (2026-10-19): of 2266 `done` events in 30 days, 1527 came from the
/// "brand new done row" branch and 1369 of those were 1-30+ days later than the
/// real completion day, so Timeline / Dashboard showed completion spikes on sync
/// days. Rules: a past date -> `<date>T12:00:00+00:00` (midday, so no timezone
/// shifts it to a neighbouring day); today's date (exact time is better), an
/// empty / unparseable date or a future date -> `now`.
///
/// Affects new events only: existing `task_events` rows are not rewritten.
fn done_event_ts(date: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    match chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d") {
        Ok(day) if day < now.date_naive() => format!("{}T12:00:00+00:00", day.format("%Y-%m-%d")),
        _ => now.to_rfc3339(),
    }
}

#[derive(Debug, serde::Serialize)]
pub struct SyncTasksReport {
    pub imported: u32,
    pub events_emitted: u32,
}

/// Who an event belongs to: given the id of a task, the GitHub login of the
/// participant whose commit caused it (`None` = this install).
type AuthorOf<'a> = &'a dyn Fn(&str) -> Option<String>;

struct CoreOpts<'a> {
    author: AuthorOf<'a>,
    /// Only these task ids are looked at (incoming-delta runs); `None` = all.
    only_ids: Option<&'a std::collections::HashSet<String>>,
    /// Drop DB rows that are absent from the MD (B-000004) and resolve the
    /// todo/done split state (H6). A full run does it; an incoming-delta run
    /// over the committed files must not: it would delete rows that exist only
    /// in the work tree, and the next full run would re-create them.
    cleanup: bool,
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
/// Events carry no author (they are this install's). Returns `SyncTasksReport`
/// with counts of imported rows and emitted events.
pub fn sync_tasks_for_repo(db: &AppDb, repo_id: i64) -> Result<SyncTasksReport, String> {
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
    let (todo_text, todo_mtime) = read_with_mtime(&todo_path, "todo.md")?;
    let (done_text, done_mtime) = read_with_mtime(&done_path, "done.md")?;

    let none = |_: &str| None;
    sync_tasks_core(
        db,
        repo_id,
        todo_text.as_deref(),
        &todo_mtime,
        done_text.as_deref(),
        &done_mtime,
        &CoreOpts {
            author: &none,
            only_ids: None,
            cleanup: true,
        },
    )
}

/// Text of `path` and its modification day (`YYYY-MM-DD`); `(None, "")` when the
/// file is absent.
fn read_with_mtime(path: &Path, what: &str) -> Result<(Option<String>, String), String> {
    if !path.exists() {
        return Ok((None, String::new()));
    }
    let content =
        crate::export::read_to_string_lossy(path).map_err(|e| format!("read {}: {}", what, e))?;
    let mtime = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
    Ok((Some(content), mtime))
}

/// The same mirror over the given file CONTENTS instead of the disk (`None` =
/// the file does not exist), with every emitted event attributed to `author`
/// (`None` = this install). Semantics are those of `sync_tasks_for_repo`.
#[cfg_attr(not(test), allow(dead_code))] // the contents-taking core; production callers use the disk wrapper / the incoming delta
pub fn sync_tasks_from_contents(
    db: &AppDb,
    repo_id: i64,
    todo: Option<&str>,
    done: Option<&str>,
    author: Option<&str>,
) -> Result<SyncTasksReport, String> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let fixed = |_: &str| author.map(String::from);
    sync_tasks_core(
        db,
        repo_id,
        todo,
        &today,
        done,
        &today,
        &CoreOpts {
            author: &fixed,
            only_ids: None,
            cleanup: true,
        },
    )
}

/// F-000044 / T-000221 (spec R11 step 1): apply what arrived BY COMMIT to the
/// mirror. `prev_*` is what this install last imported / committed, `head_*`
/// what HEAD holds now; only the tasks whose entry differs between the two are
/// touched, so local edits that are not committed yet stay as they are for the
/// ordinary disk run that follows. No orphan cleanup. `author_of` names the
/// author of each task's events.
pub fn sync_tasks_incoming(
    db: &AppDb,
    repo_id: i64,
    prev: (&str, &str),
    head: (Option<&str>, Option<&str>),
    author_of: AuthorOf,
) -> Result<SyncTasksReport, String> {
    let mut changed: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(text) = head.0 {
        let (now, _) = export::parse_todo_tasks(text);
        let (before, _) = export::parse_todo_tasks(prev.0);
        let before: std::collections::HashMap<String, String> = before
            .into_iter()
            .map(|t| (t.id.clone(), format!("{:?}", t)))
            .collect();
        for t in now {
            if before.get(&t.id) != Some(&format!("{:?}", t)) {
                changed.insert(t.id);
            }
        }
    }
    if let Some(text) = head.1 {
        let (now, _) = export::parse_done_tasks(text);
        let (before, _) = export::parse_done_tasks(prev.1);
        let before: std::collections::HashMap<String, String> = before
            .into_iter()
            .map(|t| (t.id.clone(), format!("{:?}", t)))
            .collect();
        for t in now {
            if before.get(&t.id) != Some(&format!("{:?}", t)) {
                changed.insert(t.id);
            }
        }
    }
    if changed.is_empty() {
        return Ok(SyncTasksReport {
            imported: 0,
            events_emitted: 0,
        });
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    sync_tasks_core(
        db,
        repo_id,
        head.0,
        &today,
        head.1,
        &today,
        &CoreOpts {
            author: author_of,
            only_ids: Some(&changed),
            cleanup: false,
        },
    )
}

fn sync_tasks_core(
    db: &AppDb,
    repo_id: i64,
    todo: Option<&str>,
    todo_mtime: &str,
    done: Option<&str>,
    done_mtime: &str,
    opts: &CoreOpts,
) -> Result<SyncTasksReport, String> {
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

    // Determine whether this is a first sync (suppress created events for legacy backfill)
    let was_migrated = db
        .get_tasks_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_some();
    let suppress_created_events = !was_migrated;

    // Parse todo.md. `todo_mtime` / `done_mtime` are the fallback dates for
    // entries without one; `done_mtime` is used for historical done entries
    // whose `## YYYY-MM-DD` section header is missing - `todo_mtime` was wrong
    // here because the entry came from done.md, not todo.md (review H5).
    let todo_tasks = match todo {
        Some(content) => export::parse_todo_tasks(content).0,
        None => Vec::new(),
    };
    let done_tasks = match done {
        Some(content) => export::parse_done_tasks(content).0,
        None => Vec::new(),
    };
    let todo_mtime = todo_mtime.to_string();
    let done_mtime = done_mtime.to_string();

    // T-000157: collapse duplicate ids before diffing against the DB.
    let todo_tasks = dedupe_by_id(todo_tasks, |t| t.id.as_str(), "todo.md", repo_id);
    let done_tasks = dedupe_by_id(done_tasks, |t| t.id.as_str(), "done.md", repo_id);

    // T-000183: a task standing in the id slot of BOTH files used to flap between
    // sources forever, one phantom event per sync. The TODO loop saw the DB row as
    // done and emitted `reopened`; on the next pass the DONE loop saw it as todo and
    // emitted `done`. The H6 block at the end of this function already resolves the
    // split state - but only after the events have been written, so it never stopped
    // the leak. Done is the more recent intent (the same rule H6 applies), so subtract
    // ids present in done.md from the todo side *before* the loops run: the todo
    // branch then never touches such a row at all.
    let done_ids_in_md: HashSet<&str> = done_tasks.iter().map(|t| t.id.as_str()).collect();
    let todo_tasks: Vec<_> = todo_tasks
        .into_iter()
        .filter(|t| {
            let in_done = done_ids_in_md.contains(t.id.as_str());
            if in_done {
                eprintln!(
                    "[sync_tasks repo={}] {} is in both todo.md and done.md - done wins, skipping the todo side",
                    repo_id, t.id
                );
            }
            !in_done
        })
        .collect();

    // Incoming-delta runs look at the changed tasks only.
    let (todo_tasks, done_tasks) = match opts.only_ids {
        Some(only) => (
            todo_tasks
                .into_iter()
                .filter(|t| only.contains(&t.id))
                .collect::<Vec<_>>(),
            done_tasks
                .into_iter()
                .filter(|t| only.contains(&t.id))
                .collect::<Vec<_>>(),
        ),
        None => (todo_tasks, done_tasks),
    };

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
        // T-000179: O-/M- operations classes (technical / marketing) sync like
        // any todo row. Their version is force-emptied by the parser.
        let prefix = if tt.id.starts_with("T-") {
            "T"
        } else if tt.id.starts_with("F-") {
            "F"
        } else if tt.id.starts_with("O-") {
            "O"
        } else if tt.id.starts_with("M-") {
            "M"
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
                    db.insert_task_event_by(
                        existing.id,
                        event_type,
                        &chrono::Utc::now().to_rfc3339(),
                        old_status,
                        new_status,
                        (opts.author)(&tt.id).as_deref(),
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
            db.insert_task_event_by(
                existing_done.id,
                "reopened",
                &chrono::Utc::now().to_rfc3339(),
                None,
                Some(tt.status.as_str()),
                (opts.author)(&tt.id).as_deref(),
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
                db.insert_task_event_by(
                    row.id,
                    "created",
                    &chrono::Utc::now().to_rfc3339(),
                    None,
                    to_status,
                    (opts.author)(&tt.id).as_deref(),
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
        // T-000179: O-/M- operations classes also land in done.md when closed.
        let prefix = if dt.id.starts_with("T-") {
            "T"
        } else if dt.id.starts_with("F-") {
            "F"
        } else if dt.id.starts_with("O-") {
            "O"
        } else if dt.id.starts_with("M-") {
            "M"
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
            db.insert_task_event_by(
                was_in_todo.id,
                "done",
                &done_event_ts(&dt.date, chrono::Utc::now()),
                was_in_todo.status.as_deref(),
                None,
                (opts.author)(&dt.id).as_deref(),
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
                db.insert_task_event_by(
                    row.id,
                    "done",
                    &done_event_ts(&dt.date, chrono::Utc::now()),
                    None,
                    None,
                    (opts.author)(&dt.id).as_deref(),
                )
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
    if opts.cleanup {
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
    }

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
    fn test_sync_tasks_same_id_in_both_files_emits_no_events_ever() {
        // T-000183 regression: the id sits in the slot of both todo.md and done.md.
        // Before the fix each sync wrote one phantom event (reopened, then done, then
        // reopened...), forever. Three consecutive syncs must produce zero.
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-000086 | Ghost task | 2 | high | open | 2026-08-01\n",
        )
        .unwrap();
        std::fs::write(
            repo_path.join("docs/done.md"),
            "## 2026-08-01\n- T-000086 | Ghost task | v1.0.0\n",
        )
        .unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "flap_repo", None, None)
            .unwrap();

        sync_tasks_for_repo(&db, repo.id).unwrap();
        for pass in 2..=3 {
            let r = sync_tasks_for_repo(&db, repo.id).unwrap();
            assert_eq!(
                r.events_emitted, 0,
                "sync pass {} emitted phantom events for a task listed in both files",
                pass
            );
        }

        // The task lives on the done side only - never both, never flapping back.
        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        let dones = db.list_tasks_by_repo(repo.id, "done").unwrap();
        assert!(
            todos.iter().all(|t| t.task_id != "T-000086"),
            "todo side must not carry a task that done.md already claims"
        );
        assert_eq!(dones.len(), 1);
        assert_eq!(dones[0].task_id, "T-000086");

        let events = db.list_task_events_by_task(dones[0].id).unwrap();
        assert!(
            events.is_empty(),
            "no events at all: the task was imported as done on the first pass"
        );
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_dedupes_duplicate_ids_within_done_md() {
        // T-000157 covered duplicate ids inside todo.md; done.md had no such test.
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/done.md"),
            "## 2026-08-01\n- T-000010 | First wording | v1.0.0\n\n## 2026-08-02\n- T-000010 | Later wording | v1.1.0\n",
        )
        .unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "dup_done_repo", None, None)
            .unwrap();

        sync_tasks_for_repo(&db, repo.id).unwrap();

        let dones = db.list_tasks_by_repo(repo.id, "done").unwrap();
        assert_eq!(
            dones.len(),
            1,
            "duplicate id inside done.md must collapse to one row"
        );
        assert_eq!(
            dones[0].description, "Later wording",
            "last occurrence wins, mirroring the todo.md rule"
        );

        let r = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r.events_emitted, 0, "a re-sync must stay quiet");
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

    /// T-000179: O-/M- operations tasks sync into the `tasks` table with the
    /// correct prefix (`"O"` / `"M"`) and carry no release version (the parser
    /// force-empties it, and they never sit under a release header anyway).
    #[test]
    fn test_sync_tasks_operations_classes_prefix_and_no_version() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "## v1.14.0 — release with ops mixed in\n\n\
             - [ ] O-000001 | Setup signing | 2 | high | open | 2026-08-12\n\
             - [ ] M-000001 | Write announcement | 1 | medium | open | 2026-08-12\n\
             - [ ] T-000200 | Code task | 3 | high | open | 2026-08-12\n",
        )
        .unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "ops_repo", None, None)
            .unwrap();

        let report = sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(report.imported, 3);

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos.len(), 3);

        let o = todos.iter().find(|t| t.task_id == "O-000001").unwrap();
        assert_eq!(o.prefix, "O");
        assert_eq!(o.version, None, "O- task must carry no release version");

        let m = todos.iter().find(|t| t.task_id == "M-000001").unwrap();
        assert_eq!(m.prefix, "M");
        assert_eq!(m.version, None, "M- task must carry no release version");

        // Sanity: the normal T- task under the same header still gets the version.
        let t = todos.iter().find(|t| t.task_id == "T-000200").unwrap();
        assert_eq!(t.prefix, "T");
        assert_eq!(t.version.as_deref(), Some("v1.14.0"));

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

    // ── T-000202: done events are dated by the done.md day ──────────────────

    fn utc(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    #[test]
    fn test_done_event_ts_past_date_is_midday_utc() {
        let now = utc("2026-10-19T08:30:00+00:00");
        assert_eq!(
            done_event_ts("2026-01-15", now),
            "2026-01-15T12:00:00+00:00"
        );
        assert_eq!(
            done_event_ts("2026-10-18", now),
            "2026-10-18T12:00:00+00:00"
        );
    }

    #[test]
    fn test_done_event_ts_today_keeps_exact_now() {
        let now = utc("2026-10-19T08:30:00+00:00");
        assert_eq!(done_event_ts("2026-10-19", now), now.to_rfc3339());
    }

    #[test]
    fn test_done_event_ts_future_date_falls_back_to_now() {
        let now = utc("2026-10-19T08:30:00+00:00");
        assert_eq!(done_event_ts("2026-10-20", now), now.to_rfc3339());
    }

    #[test]
    fn test_done_event_ts_empty_or_garbage_falls_back_to_now() {
        let now = utc("2026-10-19T08:30:00+00:00");
        for bad in ["", "   ", "not-a-date", "2026-13-40", "15.01.2026"] {
            assert_eq!(done_event_ts(bad, now), now.to_rfc3339(), "input {:?}", bad);
        }
    }

    /// Repo set up as already migrated (an empty first sync done), so `done`
    /// events are not suppressed.
    fn migrated_repo(db: &AppDb, repo_path: &Path) -> i64 {
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(repo_path.join("docs/todo.md"), "").unwrap();
        std::fs::write(repo_path.join("docs/done.md"), "").unwrap();
        let repo = db
            .insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None)
            .unwrap();
        sync_tasks_for_repo(db, repo.id).unwrap();
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());
        repo.id
    }

    fn done_event_ts_of(db: &AppDb, repo_id: i64, task_id: &str) -> String {
        let dones = db.list_tasks_by_repo(repo_id, "done").unwrap();
        let row = dones.iter().find(|t| t.task_id == task_id).unwrap();
        let events = db.list_task_events_by_task(row.id).unwrap();
        let ev = events
            .iter()
            .rev()
            .find(|e| e.event_type == "done")
            .expect("a done event");
        ev.ts.clone()
    }

    #[test]
    fn test_sync_tasks_brand_new_done_entry_dated_by_section_header() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        let repo_id = migrated_repo(&db, &repo_path);

        std::fs::write(
            repo_path.join("docs/done.md"),
            "## 2026-01-15
- T-000001 | Created and closed between syncs | v1.0.0
",
        )
        .unwrap();
        sync_tasks_for_repo(&db, repo_id).unwrap();

        let ts = done_event_ts_of(&db, repo_id, "T-000001");
        assert!(ts.starts_with("2026-01-15T12:00"), "got {}", ts);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_todo_to_done_move_dated_by_section_header() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        let repo_id = migrated_repo(&db, &repo_path);

        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-000002 | Task | 2 | high | review
",
        )
        .unwrap();
        sync_tasks_for_repo(&db, repo_id).unwrap();

        std::fs::write(repo_path.join("docs/todo.md"), "").unwrap();
        std::fs::write(
            repo_path.join("docs/done.md"),
            "## 2026-01-15
- T-000002 | Task | v1.0.0
",
        )
        .unwrap();
        sync_tasks_for_repo(&db, repo_id).unwrap();

        let ts = done_event_ts_of(&db, repo_id, "T-000002");
        assert!(ts.starts_with("2026-01-15T12:00"), "got {}", ts);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_done_under_todays_header_keeps_today() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        let repo_id = migrated_repo(&db, &repo_path);

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        std::fs::write(
            repo_path.join("docs/done.md"),
            format!(
                "## {}
- T-000003 | Closed today | v1.0.0
",
                today
            ),
        )
        .unwrap();
        sync_tasks_for_repo(&db, repo_id).unwrap();

        let ts = done_event_ts_of(&db, repo_id, "T-000003");
        assert!(ts.starts_with(&today), "got {}", ts);
        assert_ne!(
            ts,
            format!("{}T12:00:00+00:00", today),
            "today's event keeps the exact sync time, not the midday placeholder"
        );
        std::mem::forget(tmp);
    }

    // ----- T-000221: contents-taking core, authors, incoming delta ---------------

    fn repo_without_files(db: &AppDb, name: &str) -> i64 {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = tmp.path().to_path_buf();
        std::mem::forget(tmp);
        db.insert_local_repository(p.to_str().unwrap(), name, None, None)
            .unwrap()
            .id
    }

    fn task_event_authors(
        db: &AppDb,
        repo_id: i64,
        task_id: &str,
    ) -> Vec<(String, Option<String>)> {
        let filter = crate::models::TimelineFilter {
            start_date: "1970-01-01".into(),
            end_date: "2100-01-01".into(),
            event_kinds: None,
            project_ids: None,
            repo_ids: Some(vec![repo_id]),
            search: None,
            author: None,
            author_is_me: false,
        };
        let mut evs: Vec<_> = db
            .read_timeline_filtered(&filter, 0, 1000)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "task_event" && e.task_display_id.as_deref() == Some(task_id))
            .collect();
        evs.sort_by(|a, b| a.ts.cmp(&b.ts));
        evs.into_iter().map(|e| (e.event_type, e.author)).collect()
    }

    #[test]
    fn contents_core_attributes_its_events_to_the_given_author() {
        let db = make_db_for_sync_tests();
        let rid = repo_without_files(&db, "authored");
        let v0 = "- [ ] T-000001 | Task A | 2 | high | open | 2026-10-01\n";
        sync_tasks_from_contents(&db, rid, Some(v0), None, None).unwrap();
        let v1 = "- [ ] T-000001 | Task A | 2 | high | in-progress | 2026-10-01\n";
        let rep = sync_tasks_from_contents(&db, rid, Some(v1), None, Some("helper")).unwrap();
        assert_eq!(rep.events_emitted, 1);
        let evs = task_event_authors(&db, rid, "T-000001");
        assert_eq!(evs, vec![("taken".to_string(), Some("helper".to_string()))]);
    }

    #[test]
    fn incoming_delta_touches_only_changed_tasks_and_never_cleans_up() {
        let db = make_db_for_sync_tests();
        let rid = repo_without_files(&db, "delta");
        let prev = "- [ ] T-000001 | A | 1 | low | open | 2026-10-01\n- [ ] T-000002 | B | 1 | low | open | 2026-10-01\n";
        sync_tasks_from_contents(&db, rid, Some(prev), None, None).unwrap();
        // A local, uncommitted task that only the DB mirror knows about.
        let local = format!("{}- [ ] T-000003 | C | 1 | low | open | 2026-10-02\n", prev);
        sync_tasks_from_contents(&db, rid, Some(&local), None, None).unwrap();
        // The local edit also moved T-000002 to in-progress (uncommitted).
        let local2 = local.replace(
            "T-000002 | B | 1 | low | open",
            "T-000002 | B | 1 | low | in-progress",
        );
        sync_tasks_from_contents(&db, rid, Some(&local2), None, None).unwrap();

        // HEAD: T-000001 started by the other install.
        let head = prev.replace(
            "T-000001 | A | 1 | low | open",
            "T-000001 | A | 1 | low | in-progress",
        );
        let by_helper = |_: &str| Some("helper".to_string());
        sync_tasks_incoming(&db, rid, (prev, ""), (Some(&head), None), &by_helper).unwrap();

        let todos = db.list_tasks_by_repo(rid, "todo").unwrap();
        let status = |id: &str| {
            todos
                .iter()
                .find(|t| t.task_id == id)
                .map(|t| t.status.clone())
        };
        assert_eq!(status("T-000001"), Some(Some("in-progress".to_string())));
        // untouched: the local rows keep their state
        assert_eq!(status("T-000002"), Some(Some("in-progress".to_string())));
        assert!(status("T-000003").is_some(), "no orphan cleanup");
        let evs = task_event_authors(&db, rid, "T-000001");
        assert_eq!(evs, vec![("taken".to_string(), Some("helper".to_string()))]);
        // nothing changed -> nothing done
        let again =
            sync_tasks_incoming(&db, rid, (&head, ""), (Some(&head), None), &by_helper).unwrap();
        assert_eq!(again.events_emitted, 0);
    }
}
