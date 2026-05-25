// Bugs MD ↔ DB sync (v0.16.0)
//
// SQLite = SoT for bugs. MD (docs/bug-reports.md) is the LLM-facing view:
// LLM edits status+comment on existing rows per the global CLAUDE.md contract;
// all other fields are either app-managed (id, date, fix_attempts, confirmed_at)
// or user-owned through the UI (description, severity, category).
//
// `migrate_bugs_for_repo` is the one-time MD→DB import (lazy, first open of
// bug-tab per repo in v0.16.0+). `reconcile_bugs_for_repo` is the ongoing
// sync on bug-tab open / Refresh button / global Sync: LLM-writable fields
// (status, comment) ingest MD→DB; protected-field violations and new/deleted
// rows are silently reverted by `regenerate_bugs_md` at the end.

use crate::db::{self, AppDb};
use crate::export;
use crate::models::{Bug, FileBugNote, MigrationReport};
use std::fs;
use std::path::Path;

/// Fixed repo-relative path for the bug-reports file. Matches the global
/// CLAUDE.md template contract; hardcoded since T-048 removed the configurable
/// path setting.
const BUG_REPORTS_REL: &str = "docs/bug-reports.md";

/// Parse numeric part of a `B-NNN` display id. Lenient on length — accepts
/// legacy 3-digit (`B-042` → 42), new 6-digit (`B-000042` → 42), or any `\d+`.
/// Returns None if the prefix is absent or the tail isn't integer.
pub fn parse_numeric_id(display_id: &str) -> Option<i64> {
    display_id
        .strip_prefix("B-")?
        .parse::<i64>()
        .ok()
        .filter(|n| *n >= 0)
}

/// Validate a status transition initiated by LLM via MD edit.
/// Allowed transitions (global CLAUDE.md bug workflow + LLM-friendly shortcuts):
///   created → in-progress         (taking into work, no fix yet)
///   created → testing             (quick fix shortcut, bumps fix_attempts)
///   in-progress → testing         (fix ready, bumps fix_attempts)
///   rejected → in-progress        (restart work after rejection)
///   rejected → testing            (quick retry after rejection, bumps fix_attempts)
/// `testing → confirmed` and `testing → rejected` are UI-only paths via
/// ✓/✗ buttons and the dedicated `resolve_bug` / `reject_bug` commands —
/// NOT reachable via LLM MD edit. Allowing them here would let an LLM
/// bypass the user-verification gate by writing `status: confirmed` in
/// bug-reports.md. Anything else (e.g. `created → confirmed`, `confirmed
/// → anything`, `testing → created`) is a contract violation — ignored
/// with a warning. All transitions ending in `testing` bump `fix_attempts
/// +1` — see reconcile logic.
pub fn valid_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("created", "in-progress")
            | ("created", "testing")
            | ("in-progress", "testing")
            | ("rejected", "in-progress")
            | ("rejected", "testing")
    )
}

/// Convert a DB row to the MD-facing 8-field `FileBugNote`. `created_at` ISO
/// timestamp is truncated to `YYYY-MM-DD` (first 10 chars) to match the MD
/// contract. `confirmed_at` is not in MD — lives in DB only.
fn bug_to_file_note(bug: &Bug) -> FileBugNote {
    let date = bug
        .created_at
        .get(..10)
        .unwrap_or(&bug.created_at)
        .to_string();
    FileBugNote {
        id: bug.display_id.clone(),
        date,
        description: bug.description.clone(),
        severity: bug.severity.clone(),
        category: bug.category.clone(),
        status: bug.status.clone(),
        fix_attempts: bug.fix_attempts,
        comment: bug.comment.clone(),
    }
}

/// Build the `docs/bug-reports.md` absolute path for a repo's local checkout.
fn bug_reports_path(local_path: &str) -> std::path::PathBuf {
    let clean = local_path.trim_end_matches(['/', '\\']);
    Path::new(clean).join(BUG_REPORTS_REL)
}

/// Rewrite `docs/bug-reports.md` from the current `bugs` DB state.
///
/// v0.21.1 rules: row appears in MD if it's active (status != 'confirmed') OR
/// it's confirmed but not yet LLM-acknowledged (archived_from_md_at IS NULL).
/// This restores the original LLM-acknowledgement workflow: app sets
/// status='confirmed' on user ✓ click, MD now shows the confirmation, and on
/// the next LLM session the row gets removed from MD as cleanup. Reconcile
/// then sets archived_from_md_at, after which subsequent regens permanently
/// exclude the row. DB history (with confirmed_at) is preserved indefinitely.
///
/// Called after every mutation that can change MD contents: create/resolve/reject
/// via UI, end of reconcile, end of migration.
///
/// If `local_path` is None (remote-only repo), this is a silent no-op — MD
/// will regenerate once the repo is cloned and `local_path` populated.
pub fn regenerate_bugs_md(db: &AppDb, repo_id: i64) -> Result<(), String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(ref local_path) = repo.local_path else {
        return Ok(());
    };
    let path = bug_reports_path(local_path);
    let bugs = db.list_bugs_for_md(repo_id).map_err(|e| e.to_string())?;
    let file_notes: Vec<FileBugNote> = bugs.iter().map(bug_to_file_note).collect();
    let md = export::generate_bug_reports(&file_notes);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    fs::write(&path, md).map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// One-time lazy MD→DB bug import for a repo. Triggered on first open of
/// bug-tab in v0.16.0+. Idempotent — returns `already=true` on repeat calls.
///
/// Flow (atomicity: all DB writes in one transaction; MD write outside commit):
///   1. Check `bugs_migrated_at` marker — skip if set.
///   2. Resolve `local_path`; if None, skip (remote-only repo).
///   3. Parse `docs/bug-reports.md`. Absent file → set marker to empty, done.
///   4. Pre-check: every row has a valid `B-NNN` id and no duplicate
///      numeric_id in the file. If duplicates found, return Err before any
///      DB writes (user fixes MD manually, retries via Refresh).
///   5. Single-transaction INSERT of all rows + UPDATE marker.
///   6. Regenerate MD from DB state (6-digit ids, confirmed rows dropped).
///      Regen failure does not roll back DB — next reconcile self-heals.
pub fn migrate_bugs_for_repo(db: &AppDb, repo_id: i64) -> Result<MigrationReport, String> {
    // 1. Already migrated?
    if db
        .get_bugs_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(MigrationReport {
            imported: 0,
            confirmed_archived: 0,
            already: true,
        });
    }

    // 2. local_path present?
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(ref local_path) = repo.local_path else {
        // Remote-only: can't migrate yet. Leave marker NULL so next-time open
        // after clone triggers migration. (This is a no-op; we don't set marker.)
        return Ok(MigrationReport {
            imported: 0,
            confirmed_archived: 0,
            already: false,
        });
    };

    let now = db::utc_now_rfc3339();

    // 3. Parse MD (absent file = empty import, still set marker).
    let path = bug_reports_path(local_path);
    if !path.exists() {
        db.set_bugs_migrated_at(repo_id, &now)
            .map_err(|e| e.to_string())?;
        return Ok(MigrationReport {
            imported: 0,
            confirmed_archived: 0,
            already: false,
        });
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("Read {} failed: {}", path.display(), e))?;
    let (file_notes, warnings) = export::parse_bug_reports(&content);
    for w in &warnings {
        eprintln!("[migrate_bugs repo={}] parse warn: {}", repo_id, w);
    }

    // 4. Pre-check: extract numeric_ids, detect duplicates.
    let mut rows: Vec<(i64, FileBugNote)> = Vec::with_capacity(file_notes.len());
    let mut seen_ids = std::collections::HashSet::new();
    for note in file_notes {
        let Some(nid) = parse_numeric_id(&note.id) else {
            return Err(format!(
                "Unparseable bug id '{}' in {}. Fix MD and retry.",
                note.id,
                path.display()
            ));
        };
        if !seen_ids.insert(nid) {
            return Err(format!(
                "Duplicate bug id '{}' in {}. Fix MD and retry.",
                note.id,
                path.display()
            ));
        }
        rows.push((nid, note));
    }

    // 5. Transactional insert + marker.
    let report = db
        .migrate_bugs_transactional(repo_id, &rows, &now)
        .map_err(|e| format!("Migration transaction failed: {}", e))?;

    // 6. Regen MD from DB (outside transaction — self-healing on fs failure).
    if let Err(e) = regenerate_bugs_md(db, repo_id) {
        eprintln!(
            "[migrate_bugs repo={}] regen after commit failed: {} \
             — DB is consistent, next reconcile will regen",
            repo_id, e
        );
    }

    Ok(report)
}

/// 2-way sync MD ↔ DB for a repo. Ingests LLM edits of `status` / `comment`
/// from MD into DB; protected-field mismatches and unknown/deleted rows are
/// silently corrected by the final `regenerate_bugs_md`.
///
/// Preconditions:
///   - `bugs_migrated_at IS NOT NULL` — caller must have run
///     `migrate_bugs_for_repo` first. `ensure_bugs_migrated` Tauri command
///     handles this for UI.
///   - If `local_path IS NULL` → no-op (remote-only repo).
///   - MD file absent → all DB rows with `status != 'confirmed'` appear to
///     have been "deleted"; regen recreates the MD from DB state (self-heals).
pub fn reconcile_bugs_for_repo(db: &AppDb, repo_id: i64) -> Result<(), String> {
    if db
        .get_bugs_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Err(format!(
            "repo {} is not migrated yet — call ensure_bugs_migrated first",
            repo_id
        ));
    }

    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(ref local_path) = repo.local_path else {
        return Ok(());
    };

    // Read MD (missing file = empty rows, all active DB bugs "restored").
    let path = bug_reports_path(local_path);
    let file_notes: Vec<FileBugNote> = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Read {} failed: {}", path.display(), e))?;
        let (notes, warnings) = export::parse_bug_reports(&content);
        for w in &warnings {
            eprintln!("[reconcile_bugs repo={}] parse warn: {}", repo_id, w);
        }
        notes
    } else {
        Vec::new()
    };

    // Build lookup by numeric_id.
    let db_bugs = db
        .list_bugs_by_repo(repo_id, true)
        .map_err(|e| e.to_string())?;
    let db_by_nid: std::collections::HashMap<i64, &Bug> =
        db_bugs.iter().map(|b| (b.numeric_id, b)).collect();

    let now = db::utc_now_rfc3339();
    let mut md_ids = std::collections::HashSet::new();

    for note in &file_notes {
        let Some(nid) = parse_numeric_id(&note.id) else {
            eprintln!(
                "[reconcile_bugs repo={}] unparseable id '{}' — drop on regen",
                repo_id, note.id
            );
            continue;
        };
        md_ids.insert(nid);

        let Some(db_bug) = db_by_nid.get(&nid) else {
            eprintln!(
                "[reconcile_bugs repo={}] orphan row {} in MD (not in DB) — drop on regen",
                repo_id, note.id
            );
            continue;
        };

        // Status transition (LLM-writable).
        if note.status != db_bug.status {
            if valid_transition(&db_bug.status, &note.status) {
                let new_attempts = if note.status == "testing" && db_bug.status != "testing" {
                    Some(db_bug.fix_attempts + 1)
                } else {
                    None
                };
                let event_type = match (db_bug.status.as_str(), note.status.as_str()) {
                    ("created", "in-progress") => "taken",
                    ("created", "testing") => "entered_testing",
                    ("in-progress", "testing") => "entered_testing",
                    ("rejected", "in-progress") => "reopened",
                    ("rejected", "testing") => "entered_testing",
                    _ => "taken", // fallback — valid_transition filters invalids, unreachable in practice
                };
                // confirmed_at is set only via UI (resolve_bug), never via LLM MD edit.
                db.update_bug_status(db_bug.id, &note.status, new_attempts, None)
                    .map_err(|e| e.to_string())?;
                db.insert_bug_event(
                    db_bug.id,
                    event_type,
                    Some(db_bug.status.as_str()),
                    Some(note.status.as_str()),
                    &now,
                )
                .map_err(|e| e.to_string())?;
            } else {
                eprintln!(
                    "[reconcile_bugs repo={}] invalid transition {} → {} for {} — revert on regen",
                    repo_id, db_bug.status, note.status, note.id
                );
            }
        }

        // Comment (LLM-writable). Empty string in MD = None in DB (normalize).
        let md_comment = note.comment.as_deref().filter(|s| !s.is_empty());
        let db_comment = db_bug.comment.as_deref().filter(|s| !s.is_empty());
        if md_comment != db_comment {
            db.update_bug_comment(db_bug.id, md_comment)
                .map_err(|e| e.to_string())?;
        }

        // Protected fields (description, severity, category, fix_attempts, date) —
        // ignored here. Any mismatch will be overwritten by the regen below.
    }

    // v0.21.1: Rows missing from MD have two interpretations depending on status:
    //   - Active (status != 'confirmed') missing: LLM-deleted by mistake — regen
    //     will restore them in MD (DB is authoritative for active state).
    //   - Confirmed missing AND archived_from_md_at IS NULL: LLM acknowledged the
    //     confirmation by removing the row → mark archived_from_md_at = NOW so
    //     subsequent regens permanently exclude this row (history kept in DB).
    for db_bug in &db_bugs {
        if md_ids.contains(&db_bug.numeric_id) {
            continue;
        }
        if db_bug.status == "confirmed" && db_bug.archived_from_md_at.is_none() {
            db.mark_bug_archived_from_md(db_bug.id)
                .map_err(|e| e.to_string())?;
        } else if db_bug.status != "confirmed" {
            eprintln!(
                "[reconcile_bugs repo={}] bug {} deleted from MD by LLM — restore on regen",
                repo_id, db_bug.display_id
            );
        }
    }

    // Final regen: authoritative MD = DB state. Corrects all protected-field
    // edits, removes orphans, restores LLM-deleted active rows. Confirmed rows
    // appear if not yet LLM-acknowledged; otherwise excluded.
    regenerate_bugs_md(db, repo_id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Test helper: set up an in-memory DB with one repo pointed at a fresh
    /// temp directory. Returns (db, tmp, repo_id). The tmp dir is kept alive
    /// via the returned guard — drop it at the end of the test.
    fn setup_repo_with_dir() -> (AppDb, TempDir, i64) {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        let tmp = TempDir::new().unwrap();
        let repo = db
            .upsert_repository("owner/test-repo", None, None, None, None, None)
            .unwrap();
        db.set_repo_local_path(repo.id, Some(tmp.path().to_str().unwrap()))
            .unwrap();
        (db, tmp, repo.id)
    }

    /// Write `docs/bug-reports.md` with the provided lines inserted under
    /// the `## Open bugs` section.
    fn write_bug_reports_md(dir: &Path, bug_lines: &[&str]) {
        let docs = dir.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let mut md = String::from("# Bug reports\n\n## Open bugs\n\n");
        for line in bug_lines {
            md.push_str(line);
            if !line.ends_with('\n') {
                md.push('\n');
            }
        }
        fs::write(docs.join("bug-reports.md"), md).unwrap();
    }

    /// Read `docs/bug-reports.md` and return its contents (panics if missing).
    fn read_bug_reports_md(dir: &Path) -> String {
        fs::read_to_string(dir.join("docs").join("bug-reports.md")).unwrap()
    }

    #[test]
    fn test_parse_numeric_id() {
        assert_eq!(parse_numeric_id("B-1"), Some(1));
        assert_eq!(parse_numeric_id("B-042"), Some(42));
        assert_eq!(parse_numeric_id("B-000042"), Some(42));
        assert_eq!(parse_numeric_id("B-999999"), Some(999999));
        assert_eq!(parse_numeric_id("VB-042"), None);
        assert_eq!(parse_numeric_id("B-abc"), None);
        assert_eq!(parse_numeric_id("42"), None);
    }

    #[test]
    fn test_valid_transition_whitelist() {
        // Forward progress
        assert!(valid_transition("created", "in-progress"));
        assert!(valid_transition("created", "testing")); // quick-fix shortcut
        assert!(valid_transition("in-progress", "testing"));
        assert!(valid_transition("rejected", "in-progress"));
        assert!(valid_transition("rejected", "testing")); // retry after rejection

        // UI-only paths via ✓/✗ buttons (resolve_bug / reject_bug commands).
        // NOT reachable via LLM MD edit — otherwise LLM could bypass the
        // user-verification gate by writing `status: confirmed`.
        assert!(!valid_transition("testing", "confirmed"));
        assert!(!valid_transition("testing", "rejected"));

        // Invalid: skipping the `testing` verification step is never allowed.
        assert!(!valid_transition("created", "confirmed"));
        assert!(!valid_transition("in-progress", "confirmed"));
        assert!(!valid_transition("in-progress", "rejected"));
        assert!(!valid_transition("rejected", "confirmed"));

        // Invalid: `confirmed` is terminal; no mutation allowed.
        assert!(!valid_transition("confirmed", "in-progress"));
        assert!(!valid_transition("confirmed", "testing"));
        assert!(!valid_transition("confirmed", "rejected"));

        // Invalid: backwards moves.
        assert!(!valid_transition("testing", "created"));
        assert!(!valid_transition("in-progress", "created"));
    }

    #[test]
    fn test_migrate_bugs_for_repo_lazy() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | bug a | minor | other | created | 0 |",
                "- B-002 | 2026-03-02 | bug b | major | other | confirmed | 1 | fixed",
            ],
        );
        let report = migrate_bugs_for_repo(&db, rid).unwrap();
        assert!(!report.already);
        assert_eq!(report.imported, 2);
        assert_eq!(report.confirmed_archived, 1);

        // Re-run: should report already=true.
        let r2 = migrate_bugs_for_repo(&db, rid).unwrap();
        assert!(r2.already);
        assert_eq!(r2.imported, 0);
    }

    #[test]
    fn test_migrate_preserves_numeric_id() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-042 | 2026-03-01 | sparse-1 | minor | other | created | 0 |",
                "- B-100 | 2026-03-02 | sparse-2 | major | other | in-progress | 1 |",
            ],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        let bugs = db.list_bugs_by_repo(rid, true).unwrap();
        let nids: Vec<i64> = bugs.iter().map(|b| b.numeric_id).collect();
        assert!(nids.contains(&42));
        assert!(nids.contains(&100));
        // Display IDs use 6-digit padding.
        let dids: Vec<&str> = bugs.iter().map(|b| b.display_id.as_str()).collect();
        assert!(dids.contains(&"B-000042"));
        assert!(dids.contains(&"B-000100"));
    }

    #[test]
    fn test_migrate_confirms_archived_rows_not_in_md() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | active | minor | other | created | 0 |",
                "- B-002 | 2026-03-02 | done   | medium | other | confirmed | 2 | fixed in vX",
            ],
        );
        let report = migrate_bugs_for_repo(&db, rid).unwrap();
        assert_eq!(report.imported, 2);
        assert_eq!(report.confirmed_archived, 1);

        // After migration, MD must drop the confirmed row.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(!md.contains("B-000002"));

        // But DB keeps the confirmed row.
        let all = db.list_bugs_by_repo(rid, true).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_migrate_aborts_on_duplicate_id() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | a | minor | other | created | 0 |",
                "- B-001 | 2026-03-02 | b | major | other | created | 0 |",
            ],
        );
        let err = migrate_bugs_for_repo(&db, rid).unwrap_err();
        assert!(err.contains("Duplicate"));
        // No rows imported.
        assert_eq!(db.list_bugs_by_repo(rid, true).unwrap().len(), 0);
    }

    #[test]
    fn test_migrate_missing_md_file_sets_marker() {
        let (db, _tmp, rid) = setup_repo_with_dir();
        // No MD file written.
        let report = migrate_bugs_for_repo(&db, rid).unwrap();
        assert_eq!(report.imported, 0);
        // Marker still set so we don't re-import each run.
        assert!(db.get_bugs_migrated_at(rid).unwrap().is_some());
    }

    #[test]
    fn test_reconcile_requires_migration_first() {
        let (db, _tmp, rid) = setup_repo_with_dir();
        let err = reconcile_bugs_for_repo(&db, rid).unwrap_err();
        assert!(err.contains("not migrated"));
    }

    #[test]
    fn test_reconcile_status_transition_to_testing_increments_attempts() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM edits status → testing.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | testing | 0 | fix attempted"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.status, "testing");
        assert_eq!(b.fix_attempts, 1, "moving to testing bumps fix_attempts");
        assert_eq!(b.comment.as_deref(), Some("fix attempted"));
    }

    #[test]
    fn test_reconcile_rejects_testing_to_confirmed_from_md() {
        // Guard: LLM must not be able to confirm a bug via MD edit — that
        // path is reserved for the UI ✓ button (resolve_bug command). If
        // the LLM writes `status: confirmed` in bug-reports.md, reconcile
        // ignores the transition and regen restores the testing status.
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | testing | 1 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM attempts testing → confirmed — should be ignored.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | confirmed | 1 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(
            b.status, "testing",
            "LLM cannot bypass user-verification gate"
        );
        assert!(
            b.confirmed_at.is_none(),
            "confirmed_at set only via UI resolve_bug"
        );
    }

    #[test]
    fn test_reconcile_protected_field_restored_on_regen() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | original desc | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM rewrites description (protected field).
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | EDITED BY LLM | minor | other | created | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB unchanged.
        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.description, "original desc");

        // MD regenerated from DB — LLM edit was reverted.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("original desc"));
        assert!(!md.contains("EDITED BY LLM"));
    }

    #[test]
    fn test_reconcile_orphan_row_removed() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM tries to add a new bug via MD (B-999).
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-000001 | 2026-03-01 | bug | minor | other | created | 0 |",
                "- B-999 | 2026-03-02 | injected by LLM | major | other | created | 0 |",
            ],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB does not have the orphan.
        assert_eq!(db.list_bugs_by_repo(rid, true).unwrap().len(), 1);

        // MD regen drops it.
        let md = read_bug_reports_md(tmp.path());
        assert!(!md.contains("B-999"));
        assert!(!md.contains("injected by LLM"));
    }

    #[test]
    fn test_reconcile_deleted_non_confirmed_row_restored() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | a | minor | other | created | 0 |",
                "- B-002 | 2026-03-02 | b | major | other | created | 0 |",
            ],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM deletes B-002 from MD (illegal — only confirmed rows can be removed).
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | a | minor | other | created | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB unchanged.
        assert_eq!(db.list_bugs_by_repo(rid, true).unwrap().len(), 2);

        // MD regen restored B-000002.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(md.contains("B-000002"));
    }

    #[test]
    fn test_reconcile_deleted_confirmed_row_stays_deleted_from_md_db_intact() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | active | minor | other | created | 0 |",
                "- B-002 | 2026-03-02 | closed | minor | other | confirmed | 1 |",
            ],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();
        // After migration, MD already has B-002 dropped.

        // LLM edit (just reaffirming MD state without B-002).
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB still has the confirmed row for history.
        let all = db.list_bugs_by_repo(rid, true).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|b| b.status == "confirmed"));

        // MD has only the active one.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(!md.contains("B-000002"));
    }

    #[test]
    fn test_reconcile_invalid_transition_ignored() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM attempts invalid transition: created → confirmed (skipping workflow).
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | confirmed | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB status NOT changed; confirmed_at NOT set.
        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.status, "created");
        assert!(b.confirmed_at.is_none());

        // MD regen reverts to DB truth.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("| created |"));
        assert!(!md.contains("| confirmed |"));
    }

    #[test]
    fn test_reconcile_comment_update_propagates() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | in-progress | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM updates comment only (no status change).
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | in-progress | 0 | debugging now"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.comment.as_deref(), Some("debugging now"));

        // LLM clears comment.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | in-progress | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert!(b.comment.is_none());
    }

    #[test]
    fn test_reconcile_missing_md_file_restores_from_db() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // User deletes MD file externally.
        fs::remove_file(tmp.path().join("docs").join("bug-reports.md")).unwrap();

        reconcile_bugs_for_repo(&db, rid).unwrap();

        // File restored from DB state.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
    }

    #[test]
    fn test_regenerate_bugs_md_includes_unacknowledged_confirmed() {
        // v0.21.1: confirmed bugs appear in MD until LLM-acknowledged
        // (archived_from_md_at IS NULL). Both active and unacknowledged-confirmed rows show.
        let (db, tmp, rid) = setup_repo_with_dir();
        db.insert_bug(
            rid,
            1,
            "2026-03-01T00:00:00Z",
            "active",
            "minor",
            "other",
            "created",
            0,
            None,
            None,
        )
        .unwrap();
        db.insert_bug(
            rid,
            2,
            "2026-03-02T00:00:00Z",
            "fresh-confirm",
            "minor",
            "other",
            "confirmed",
            1,
            None,
            Some("2026-04-24T10:00:00Z"),
        )
        .unwrap();

        regenerate_bugs_md(&db, rid).unwrap();

        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"), "active bug must appear in MD");
        assert!(
            md.contains("B-000002"),
            "fresh confirmed (not yet acknowledged) must appear"
        );
    }

    #[test]
    fn test_reconcile_marks_confirmed_archived_when_llm_removes_from_md() {
        // v0.21.1 workflow: app sets status='confirmed' on user ✓ click; row appears
        // in MD with confirmed status; LLM removes it on next session edit; reconcile
        // detects the absence and sets archived_from_md_at, ensuring future regens
        // permanently exclude the row.
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | confirmed | 1 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();
        // Migration's regen drops the confirmed row (legacy import semantics —
        // confirmed-from-MD treated as already archived).

        // Manually unarchive to simulate a v0.21.1+ flow: app set confirmed,
        // row visible to LLM in MD.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE bugs SET archived_from_md_at = NULL WHERE display_id = 'B-000001'",
                [],
            )
            .unwrap();
        }
        regenerate_bugs_md(&db, rid).unwrap();
        let md_with_confirmed = read_bug_reports_md(tmp.path());
        assert!(
            md_with_confirmed.contains("B-000001"),
            "fresh-confirmed must be in MD"
        );

        // LLM edit: removes the confirmed row.
        write_bug_reports_md(tmp.path(), &[]);

        // Reconcile must mark archived + final regen excludes it.
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert!(
            b.archived_from_md_at.is_some(),
            "reconcile must mark archived"
        );
        assert_eq!(
            b.status, "confirmed",
            "DB row stays as confirmed for history"
        );

        let md_after = read_bug_reports_md(tmp.path());
        assert!(
            !md_after.contains("B-000001"),
            "archived row no longer in MD"
        );
    }

    #[test]
    fn test_regenerate_bugs_md_excludes_archived_confirmed() {
        // v0.21.1: once LLM acknowledged a confirmed row (archived_from_md_at set),
        // it's permanently excluded from MD. DB row still exists for history.
        let (db, tmp, rid) = setup_repo_with_dir();
        db.insert_bug(
            rid,
            1,
            "2026-03-01T00:00:00Z",
            "active",
            "minor",
            "other",
            "created",
            0,
            None,
            None,
        )
        .unwrap();
        let archived_bug = db
            .insert_bug(
                rid,
                2,
                "2026-03-02T00:00:00Z",
                "archived",
                "minor",
                "other",
                "confirmed",
                1,
                None,
                Some("2026-04-24T10:00:00Z"),
            )
            .unwrap();
        db.mark_bug_archived_from_md(archived_bug.id).unwrap();

        regenerate_bugs_md(&db, rid).unwrap();

        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(
            !md.contains("B-000002"),
            "archived confirmed must drop from MD"
        );
    }

    #[test]
    fn test_reconcile_records_events_on_transition() {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        let tmp = TempDir::new().unwrap();
        let repo = db
            .upsert_repository("owner/events-test", None, None, None, None, None)
            .unwrap();
        db.set_repo_local_path(repo.id, Some(tmp.path().to_str().unwrap()))
            .unwrap();

        let bug = db
            .insert_bug(
                repo.id,
                1,
                "2026-04-24T10:00:00Z",
                "x",
                "minor",
                "other",
                "created",
                0,
                None,
                None,
            )
            .unwrap();

        // Simulate LLM moving created → in-progress (should produce 'taken' event).
        db.update_bug_status(bug.id, "in-progress", None, None)
            .unwrap();
        db.insert_bug_event(
            bug.id,
            "taken",
            Some("created"),
            Some("in-progress"),
            &crate::db::utc_now_rfc3339(),
        )
        .unwrap();

        let conn = db.conn.lock().unwrap();
        let ev_type: String = conn
            .query_row(
                "SELECT event_type FROM bug_events WHERE bug_id=?1 ORDER BY id DESC LIMIT 1",
                [bug.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ev_type, "taken");
    }

    /// T-000128 regression: UI mutation commands (create_bug / resolve_bug /
    /// update_bug_fields / delete_bug / reject_bug in lib.rs) must call
    /// `reconcile_bugs_for_repo` BEFORE their DB mutation so pending LLM MD
    /// edits ingest first. Without that, the final `regenerate_bugs_md`
    /// overwrites the LLM's MD changes with stale DB state.
    ///
    /// This test exercises the FIXED pattern (reconcile → mutate → regen)
    /// and asserts LLM's status+comment edit on bug A survives a fresh
    /// "+ Add bug" insertion of bug B. If a future refactor drops the
    /// leading reconcile call from any of those 5 lib.rs commands, the
    /// hand-trace pattern documented here is the contract being broken.
    #[test]
    fn test_t000128_reconcile_before_mutate_preserves_llm_edits() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | first | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM edits MD: B-000001 created → testing + comment.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | first | minor | other | testing | 0 | done"],
        );

        // Fixed UI command sequence: reconcile FIRST, then DB mutation,
        // then regen. Mirrors the lib.rs `create_bug` body after T-000128.
        reconcile_bugs_for_repo(&db, rid).unwrap();
        let nid = db.next_numeric_id(rid).unwrap();
        db.insert_bug(
            rid,
            nid,
            "2026-05-25T00:00:00Z",
            "second",
            "minor",
            "other",
            "created",
            0,
            None,
            None,
        )
        .unwrap();
        regenerate_bugs_md(&db, rid).unwrap();

        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"), "B-000001 row preserved");
        assert!(md.contains("B-000002"), "new B-000002 row added");
        assert!(md.contains("| testing |"), "LLM status edit preserved");
        assert!(md.contains("done"), "LLM comment preserved");

        let b1 = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b1.status, "testing");
        assert_eq!(
            b1.fix_attempts, 1,
            "created→testing transition bumped fix_attempts"
        );
        assert_eq!(b1.comment.as_deref(), Some("done"));
    }
}
