// T-000094: db.rs split into per-domain sub-modules. Each child file adds
// its own `impl AppDb { ... }` block — Rust allows multiple impls in
// different files of the same module.
//
// `AppDb` struct + Mutex<Connection> + ctor live here. Shared row mappers
// (`bug_from_row`, `row_to_repo`) and the `utc_now_rfc3339()` helper are
// also here so child modules can `use super::*;`.

use crate::models::*;
use rusqlite::{Connection, Result as SqlResult, Row};
use std::path::PathBuf;
use std::sync::Mutex;

pub mod bugs;
pub mod bundle;
pub mod dashboard;
pub mod deploy;
pub mod graph;
pub mod migrations;
pub mod projects;
pub mod repos;
pub mod stats;
pub mod tasks_events;
pub mod timeline;

pub struct AppDb {
    pub conn: Mutex<Connection>,
}

/// v0.16.0: UTC timestamp in RFC 3339 format ("2026-04-24T12:34:56.789+00:00").
/// Used uniformly for bug `created_at`, `confirmed_at`, and `bugs_migrated_at`.
/// Migration path also uses this for consistency (old `YYYY-MM-DD` dates get
/// `T00:00:00+00:00` suffix).
pub fn utc_now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// v0.16.0: shared row→Bug mapper. Column order must match all SELECTs that
/// map into `Bug` (id, repository_id, numeric_id, display_id, created_at,
/// description, severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at).
pub(super) fn bug_from_row(row: &Row) -> SqlResult<Bug> {
    Ok(Bug {
        id: row.get(0)?,
        repository_id: row.get(1)?,
        numeric_id: row.get(2)?,
        display_id: row.get(3)?,
        created_at: row.get(4)?,
        description: row.get(5)?,
        severity: row.get(6)?,
        category: row.get(7)?,
        status: row.get(8)?,
        fix_attempts: row.get(9)?,
        comment: row.get(10)?,
        confirmed_at: row.get(11)?,
        archived_from_md_at: row.get(12)?,
    })
}

pub fn row_to_repo(row: &Row) -> SqlResult<Repository> {
    Ok(Repository {
        id: row.get(0)?,
        project_id: row.get(1)?,
        github_name: row.get(2)?,
        github_url: row.get(3)?,
        role: row.get(4)?,
        description: row.get(5)?,
        language: row.get(6)?,
        last_pushed_at: row.get(7)?,
        added_at: row.get(8)?,
        updated_at: row.get(9)?,
        local_path: row.get(10)?,
        github_id: row.get(11)?,
        deploy_target: row.get(12)?,
    })
}

impl AppDb {
    pub fn new(db_path: PathBuf) -> SqlResult<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let db = AppDb {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        // v0.17.0: synthesize bug_events for pre-v19 bugs on v18→v19 upgrade.
        // Idempotent guard inside: no-op if bug_events already has rows.
        db.backfill_bug_events_for_existing()?;
        Ok(db)
    }
}
