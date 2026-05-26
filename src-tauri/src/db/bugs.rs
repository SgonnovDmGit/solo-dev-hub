// T-000094: bugs CRUD + bug_events + reconcile/migrate.
// Moved from db.rs.

use super::*;

impl AppDb {
    // ── Bugs (v0.16.0, SQLite = SoT) ──────────────────────────────────────────

    /// Next numeric_id for a new bug in `repo_id`. Starts at 1 for empty repos.
    /// Uses `MAX(numeric_id) + 1` — per-repo counter, NOT global autoincrement.
    pub fn next_numeric_id(&self, repo_id: i64) -> SqlResult<i64> {
        let conn = self.conn.lock().unwrap();
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(numeric_id), 0) + 1 FROM bugs WHERE repository_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )?;
        Ok(next)
    }

    /// Insert a new bug row. `numeric_id` is pre-computed by caller via `next_numeric_id`
    /// to keep the id-allocation logic explicit and testable.
    /// `display_id` is formatted as `B-{:06}` from `numeric_id`.
    /// `created_at` is set to UTC now if not provided (migration path passes explicit value).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_bug(
        &self,
        repo_id: i64,
        numeric_id: i64,
        created_at: &str,
        description: &str,
        severity: &str,
        category: &str,
        status: &str,
        fix_attempts: i32,
        comment: Option<&str>,
        confirmed_at: Option<&str>,
    ) -> SqlResult<Bug> {
        let conn = self.conn.lock().unwrap();
        let display_id = format!("B-{:06}", numeric_id);
        conn.execute(
            "INSERT INTO bugs (repository_id, numeric_id, display_id, created_at,
                               description, severity, category, status, fix_attempts,
                               comment, confirmed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                repo_id,
                numeric_id,
                display_id,
                created_at,
                description,
                severity,
                category,
                status,
                fix_attempts,
                comment,
                confirmed_at,
            ],
        )?;
        let id = conn.last_insert_rowid();
        conn.query_row(
            "SELECT id, repository_id, numeric_id, display_id, created_at, description,
                    severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at
             FROM bugs WHERE id = ?1",
            rusqlite::params![id],
            bug_from_row,
        )
    }

    /// Update status on an existing bug (by internal id). Caller is responsible
    /// for transition validity — `valid_transition()` check lives in `sync.rs`.
    /// If `new_fix_attempts` is `Some`, overrides the current value (used when
    /// entering `testing` status bumps attempts).
    /// If `new_confirmed_at` is `Some`, overrides (set on `confirmed`, leave None otherwise).
    pub fn update_bug_status(
        &self,
        bug_id: i64,
        new_status: &str,
        new_fix_attempts: Option<i32>,
        new_confirmed_at: Option<&str>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        match (new_fix_attempts, new_confirmed_at) {
            (Some(fa), Some(ca)) => {
                conn.execute(
                    "UPDATE bugs SET status = ?1, fix_attempts = ?2, confirmed_at = ?3 WHERE id = ?4",
                    rusqlite::params![new_status, fa, ca, bug_id],
                )?;
            }
            (Some(fa), None) => {
                conn.execute(
                    "UPDATE bugs SET status = ?1, fix_attempts = ?2 WHERE id = ?3",
                    rusqlite::params![new_status, fa, bug_id],
                )?;
            }
            (None, Some(ca)) => {
                conn.execute(
                    "UPDATE bugs SET status = ?1, confirmed_at = ?2 WHERE id = ?3",
                    rusqlite::params![new_status, ca, bug_id],
                )?;
            }
            (None, None) => {
                conn.execute(
                    "UPDATE bugs SET status = ?1 WHERE id = ?2",
                    rusqlite::params![new_status, bug_id],
                )?;
            }
        }
        Ok(())
    }

    /// Update comment on an existing bug (by internal id). Passing `None` sets comment to NULL.
    pub fn update_bug_comment(&self, bug_id: i64, comment: Option<&str>) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE bugs SET comment = ?1 WHERE id = ?2",
            rusqlite::params![comment, bug_id],
        )?;
        Ok(())
    }

    /// Update user-owned fields (description/severity/category) and/or comment on
    /// an existing bug (by internal id). Each `Some(_)` arg sets that field; `None`
    /// leaves the DB value unchanged. Comment `Some(None)` explicitly clears the
    /// field — caller distinguishes via the outer Option.
    pub fn update_bug_fields(
        &self,
        bug_id: i64,
        description: Option<&str>,
        severity: Option<&str>,
        category: Option<&str>,
        comment: Option<Option<&str>>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        if let Some(d) = description {
            conn.execute(
                "UPDATE bugs SET description = ?1 WHERE id = ?2",
                rusqlite::params![d, bug_id],
            )?;
        }
        if let Some(s) = severity {
            conn.execute(
                "UPDATE bugs SET severity = ?1 WHERE id = ?2",
                rusqlite::params![s, bug_id],
            )?;
        }
        if let Some(c) = category {
            conn.execute(
                "UPDATE bugs SET category = ?1 WHERE id = ?2",
                rusqlite::params![c, bug_id],
            )?;
        }
        if let Some(c) = comment {
            conn.execute(
                "UPDATE bugs SET comment = ?1 WHERE id = ?2",
                rusqlite::params![c, bug_id],
            )?;
        }
        Ok(())
    }

    /// T-000130: reopen a confirmed-or-rejected bug back to `testing`.
    /// Atomically: status → 'testing', confirmed_at → NULL, archived_from_md_at
    /// → NULL. `fix_attempts` left unchanged (reopen is the undo of a verdict,
    /// not a new fix attempt — the invariant `COUNT(entered_testing) ==
    /// fix_attempts` must hold). Caller is responsible for inserting the
    /// `reopened` bug_event and gating the source status (confirmed/rejected
    /// only); see `reopen_bug` Tauri command in `lib.rs`.
    pub fn reopen_bug(&self, bug_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE bugs
             SET status = 'testing',
                 confirmed_at = NULL,
                 archived_from_md_at = NULL
             WHERE id = ?1",
            rusqlite::params![bug_id],
        )?;
        Ok(())
    }

    /// Hard-delete a bug row. Used only for "accidental creation" cleanup
    /// (UI gates to `status='created'`). Normal close flow is resolve_bug
    /// (→ status='confirmed', row stays for history).
    pub fn delete_bug(&self, bug_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM bugs WHERE id = ?1", rusqlite::params![bug_id])?;
        Ok(())
    }

    /// List bugs for a repo. If `include_confirmed=false`, rows with status='confirmed' are filtered out.
    /// Ordered by `numeric_id` ascending (stable user-facing order).
    pub fn list_bugs_by_repo(&self, repo_id: i64, include_confirmed: bool) -> SqlResult<Vec<Bug>> {
        let conn = self.conn.lock().unwrap();
        let sql = if include_confirmed {
            "SELECT id, repository_id, numeric_id, display_id, created_at, description,
                    severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at
             FROM bugs WHERE repository_id = ?1 ORDER BY numeric_id ASC"
        } else {
            "SELECT id, repository_id, numeric_id, display_id, created_at, description,
                    severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at
             FROM bugs WHERE repository_id = ?1 AND status != 'confirmed' ORDER BY numeric_id ASC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params![repo_id], bug_from_row)?;
        rows.collect()
    }

    /// v0.21.1: Bugs visible in MD. Returns active rows (not confirmed) PLUS
    /// confirmed rows that haven't been LLM-acknowledged yet (archived_from_md_at IS NULL).
    /// Used by `regenerate_bugs_md` so LLM sees confirmation in MD until the next
    /// session edit, after which reconcile sets archived_from_md_at and the row
    /// drops from MD permanently. DB-side history is preserved (row stays).
    pub fn list_bugs_for_md(&self, repo_id: i64) -> SqlResult<Vec<Bug>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, numeric_id, display_id, created_at, description,
                    severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at
             FROM bugs
             WHERE repository_id = ?1
               AND (status != 'confirmed' OR archived_from_md_at IS NULL)
             ORDER BY numeric_id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![repo_id], bug_from_row)?;
        rows.collect()
    }

    /// v0.21.1: Mark a confirmed bug as LLM-acknowledged (LLM removed it from MD).
    /// Subsequent `regenerate_bugs_md` calls won't re-add this row.
    /// Idempotent — re-acknowledging is a no-op (timestamp not overwritten).
    pub fn mark_bug_archived_from_md(&self, bug_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE bugs SET archived_from_md_at = ?1
             WHERE id = ?2 AND archived_from_md_at IS NULL",
            rusqlite::params![chrono::Utc::now().to_rfc3339(), bug_id],
        )?;
        Ok(())
    }

    /// Count of `status='confirmed'` bugs for a repo. Used by "Показать закрытые (N)" toggle label.
    pub fn count_confirmed_bugs(&self, repo_id: i64) -> SqlResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM bugs WHERE repository_id = ?1 AND status = 'confirmed'",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
    }

    /// Find a bug by (repo, display_id). Returns `None` if not found.
    pub fn get_bug_by_display_id(&self, repo_id: i64, display_id: &str) -> SqlResult<Option<Bug>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT id, repository_id, numeric_id, display_id, created_at, description,
                    severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at
             FROM bugs WHERE repository_id = ?1 AND display_id = ?2",
            rusqlite::params![repo_id, display_id],
            bug_from_row,
        ) {
            Ok(bug) => Ok(Some(bug)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Get a bug by internal (auto-increment) id.
    pub fn get_bug_by_id(&self, bug_id: i64) -> SqlResult<Option<Bug>> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT id, repository_id, numeric_id, display_id, created_at, description,
                    severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at
             FROM bugs WHERE id = ?1",
            rusqlite::params![bug_id],
            bug_from_row,
        ) {
            Ok(bug) => Ok(Some(bug)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// v0.16.0: marker for lazy per-repo bug migration. NULL = not yet migrated.
    /// Migration skipped on subsequent calls once set.
    pub fn get_bugs_migrated_at(&self, repo_id: i64) -> SqlResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT bugs_migrated_at FROM repositories WHERE id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
    }

    pub fn set_bugs_migrated_at(&self, repo_id: i64, ts: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET bugs_migrated_at = ?1 WHERE id = ?2",
            rusqlite::params![ts, repo_id],
        )?;
        Ok(())
    }

    /// v0.16.0: atomically import bugs parsed from MD into the `bugs` table and
    /// set the per-repo `bugs_migrated_at` marker. Rolls back on any UNIQUE
    /// violation (duplicate numeric_id within the same repo).
    ///
    /// Input: `rows` = (numeric_id, FileBugNote) tuples. `numeric_id` must be
    /// pre-extracted from the MD display_id by the caller (via `sync::parse_numeric_id`),
    /// so malformed ids fail early (before the transaction starts).
    ///
    /// `created_at` is built from `row.date` as `{date}T00:00:00Z`.
    /// `confirmed_at` is set to `now` if `row.status=='confirmed'`, else None.
    /// MD file write happens outside this transaction, on success — see
    /// `sync::migrate_bugs_for_repo` for the surrounding flow.
    ///
    /// T-000091: also synthesizes `bug_events` rows per imported bug
    /// (`created` + N×`entered_testing` + optional `confirmed`) so that
    /// Dashboard KPI5 (reads bug_events) stays aligned with StatsSummary
    /// (reads bugs.fix_attempts). Mirrors `backfill_bug_events_for_existing`.
    pub fn migrate_bugs_transactional(
        &self,
        repo_id: i64,
        rows: &[(i64, FileBugNote)],
        now: &str,
    ) -> SqlResult<MigrationReport> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        let mut imported = 0u32;
        let mut confirmed_archived = 0u32;
        for (numeric_id, row) in rows {
            let display_id = format!("B-{:06}", numeric_id);
            let created_at = format!("{}T00:00:00Z", row.date);
            let confirmed_at = if row.status == "confirmed" {
                Some(now)
            } else {
                None
            };
            // v0.21.1: legacy migrated confirmed-bugs are treated as already
            // LLM-acknowledged (archived_from_md_at = NOW) — preserves legacy
            // "confirmed → drops from MD" UX expectation. Fresh confirmations
            // post-v0.21.1 instead get archived NULL until reconcile sees the
            // LLM-removal in MD.
            let archived_from_md_at = if row.status == "confirmed" {
                Some(now)
            } else {
                None
            };
            tx.execute(
                "INSERT INTO bugs (repository_id, numeric_id, display_id, created_at,
                                   description, severity, category, status, fix_attempts,
                                   comment, confirmed_at, archived_from_md_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    repo_id,
                    numeric_id,
                    display_id,
                    created_at,
                    row.description,
                    row.severity,
                    row.category,
                    row.status,
                    row.fix_attempts,
                    row.comment,
                    confirmed_at,
                    archived_from_md_at,
                ],
            )?;
            let bug_id = tx.last_insert_rowid();
            imported += 1;
            if row.status == "confirmed" {
                confirmed_archived += 1;
            }

            // T-000091: synthesize bug_events for the imported bug so that
            // Dashboard KPI5 (reads bug_events.entered_testing count) stays
            // aligned with stats_summary.avg_attempts (reads bugs.fix_attempts).
            // Same logic as `backfill_bug_events_for_existing`, scoped to the
            // single row we just inserted.
            let mut synth_attempts = row.fix_attempts as i64;
            if row.status == "confirmed" && synth_attempts < 1 {
                synth_attempts = 1;
            }

            tx.execute(
                "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                 VALUES (?1, 'created', ?2, NULL, 'created')",
                rusqlite::params![bug_id, created_at],
            )?;

            if synth_attempts > 0 {
                let end_ts = confirmed_at.unwrap_or(now);
                let start_dt = chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let end_dt = chrono::DateTime::parse_from_rfc3339(end_ts)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let span = (end_dt - start_dt).num_seconds().max(1);

                for i in 0..synth_attempts {
                    let t = start_dt
                        + chrono::Duration::seconds(((i + 1) * span) / (synth_attempts + 1));
                    tx.execute(
                        "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                         VALUES (?1, 'entered_testing', ?2, 'in-progress', 'testing')",
                        rusqlite::params![bug_id, t.to_rfc3339()],
                    )?;
                }
            }

            if row.status == "confirmed" {
                if let Some(cat) = confirmed_at {
                    tx.execute(
                        "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                         VALUES (?1, 'confirmed', ?2, 'testing', 'confirmed')",
                        rusqlite::params![bug_id, cat],
                    )?;
                }
            }
        }

        tx.execute(
            "UPDATE repositories SET bugs_migrated_at = ?1 WHERE id = ?2",
            rusqlite::params![now, repo_id],
        )?;

        tx.commit()?;
        Ok(MigrationReport {
            imported,
            confirmed_archived,
            already: false,
        })
    }

    // ── Bug events (v0.17.0) ──────────────────────────────────────────────────

    /// Insert a new event row into bug_events. `ts` is RFC3339 UTC.
    /// `from_status=None, to_status=Some("created")` for creation events.
    pub fn insert_bug_event(
        &self,
        bug_id: i64,
        event_type: &str,
        from_status: Option<&str>,
        to_status: Option<&str>,
        ts: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![bug_id, event_type, ts, from_status, to_status],
        )?;
        Ok(())
    }

    /// Back-fill bug_events for bugs inserted BEFORE migration v19.
    /// No-op if any rows already exist in bug_events (idempotent guard).
    /// Invariant preserved: COUNT(entered_testing events) == bugs.fix_attempts
    /// (or at least 1 for corrupt legacy confirmed bugs with fix_attempts=0).
    pub fn backfill_bug_events_for_existing(&self) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();

        let existing: i64 = conn.query_row("SELECT COUNT(*) FROM bug_events", [], |r| r.get(0))?;
        if existing > 0 {
            return Ok(());
        }

        let mut stmt = conn.prepare(
            "SELECT id, created_at, status, fix_attempts, confirmed_at FROM bugs ORDER BY id",
        )?;
        let rows: Vec<(i64, String, String, i64, Option<String>)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        drop(stmt);

        let now = crate::db::utc_now_rfc3339();
        for (bug_id, created_at, status, mut fix_attempts, confirmed_at) in rows {
            if status == "confirmed" && fix_attempts < 1 {
                eprintln!(
                    "[backfill] bug_id={} status='confirmed' but fix_attempts=0 — forcing 1 synthetic attempt",
                    bug_id
                );
                fix_attempts = 1;
            }

            // 1. created event
            conn.execute(
                "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                 VALUES (?1, 'created', ?2, NULL, 'created')",
                rusqlite::params![bug_id, created_at],
            )?;

            // 2. entered_testing events (N = fix_attempts), evenly spaced
            if fix_attempts > 0 {
                let end_ts = confirmed_at.as_deref().unwrap_or(&now);
                let start = chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let end = chrono::DateTime::parse_from_rfc3339(end_ts)
                    .map(|t| t.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let span = (end - start).num_seconds().max(1);

                for i in 0..fix_attempts {
                    let t =
                        start + chrono::Duration::seconds(((i + 1) * span) / (fix_attempts + 1));
                    conn.execute(
                        "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                         VALUES (?1, 'entered_testing', ?2, 'in-progress', 'testing')",
                        rusqlite::params![bug_id, t.to_rfc3339()],
                    )?;
                }
            }

            // 3. confirmed event
            if status == "confirmed" {
                if let Some(ref cat) = confirmed_at {
                    conn.execute(
                        "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                         VALUES (?1, 'confirmed', ?2, 'testing', 'confirmed')",
                        rusqlite::params![bug_id, cat],
                    )?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    fn make_repo(db: &AppDb) -> i64 {
        let r = db
            .upsert_repository("owner/x", None, None, None, None, None)
            .unwrap();
        r.id
    }

    // ── Bug CRUD tests (v0.16.0; bug_stats VIEW dropped in v23) ──────────────

    /// Helper: seed a bug via new API. Timestamp `YYYY-MM-DD` expands to
    /// `YYYY-MM-DDT00:00:00Z` to match `date(created_at)` aggregation in stats queries.
    fn seed_bug(
        db: &AppDb,
        repo_id: i64,
        date: &str,
        severity: &str,
        category: &str,
        fix_attempts: i32,
        status: &str,
    ) -> Bug {
        let nid = db.next_numeric_id(repo_id).unwrap();
        let created = format!("{}T00:00:00Z", date);
        let confirmed = if status == "confirmed" {
            Some("2026-04-24T12:00:00Z".to_string())
        } else {
            None
        };
        db.insert_bug(
            repo_id,
            nid,
            &created,
            "seed",
            severity,
            category,
            status,
            fix_attempts,
            None,
            confirmed.as_deref(),
        )
        .unwrap()
    }

    #[test]
    fn test_insert_bug_assigns_display_id() {
        let db = make_db();
        let rid = make_repo(&db);
        let b1 = seed_bug(&db, rid, "2026-03-29", "critical", "database", 0, "created");
        assert_eq!(b1.numeric_id, 1);
        assert_eq!(b1.display_id, "B-000001");
        let b2 = seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "created");
        assert_eq!(b2.numeric_id, 2);
        assert_eq!(b2.display_id, "B-000002");
    }

    #[test]
    fn test_next_numeric_id_empty_repo_returns_one() {
        let db = make_db();
        let rid = make_repo(&db);
        assert_eq!(db.next_numeric_id(rid).unwrap(), 1);
    }

    #[test]
    fn test_next_numeric_id_per_repo_independent() {
        let db = make_db();
        let r1 = make_repo(&db);
        let r2 = db
            .upsert_repository("owner/other-repo", None, None, None, None, None)
            .unwrap()
            .id;
        seed_bug(&db, r1, "2026-03-29", "critical", "database", 0, "created");
        seed_bug(&db, r1, "2026-03-29", "minor", "ui_ux", 0, "created");
        assert_eq!(db.next_numeric_id(r2).unwrap(), 1);
        assert_eq!(db.next_numeric_id(r1).unwrap(), 3);
    }

    #[test]
    fn test_insert_bug_duplicate_numeric_id_fails() {
        let db = make_db();
        let rid = make_repo(&db);
        db.insert_bug(
            rid,
            42,
            "2026-03-29T00:00:00Z",
            "first",
            "minor",
            "other",
            "created",
            0,
            None,
            None,
        )
        .unwrap();
        let err = db
            .insert_bug(
                rid,
                42,
                "2026-03-29T00:00:00Z",
                "dup",
                "minor",
                "other",
                "created",
                0,
                None,
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("UNIQUE"), "got: {}", err);
    }

    #[test]
    fn test_update_bug_status_overrides_attempts_and_confirmed_at() {
        let db = make_db();
        let rid = make_repo(&db);
        let b = seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "created");
        db.update_bug_status(b.id, "in-progress", None, None)
            .unwrap();
        db.update_bug_status(b.id, "testing", Some(1), None)
            .unwrap();
        let refreshed = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert_eq!(refreshed.status, "testing");
        assert_eq!(refreshed.fix_attempts, 1);
        assert!(refreshed.confirmed_at.is_none());

        db.update_bug_status(b.id, "confirmed", None, Some("2026-04-24T10:00:00Z"))
            .unwrap();
        let refreshed = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert_eq!(refreshed.status, "confirmed");
        assert_eq!(
            refreshed.confirmed_at.as_deref(),
            Some("2026-04-24T10:00:00Z")
        );
    }

    // T-000130: reopen_bug — undo a confirmed/rejected verdict back to testing.
    #[test]
    fn test_reopen_bug_from_confirmed_clears_confirmed_at_keeps_attempts() {
        let db = make_db();
        let rid = make_repo(&db);
        let b = seed_bug(&db, rid, "2026-05-10", "major", "logic", 2, "testing");
        db.update_bug_status(b.id, "confirmed", None, Some("2026-05-20T10:00:00Z"))
            .unwrap();
        let pre = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert_eq!(pre.status, "confirmed");
        assert_eq!(pre.confirmed_at.as_deref(), Some("2026-05-20T10:00:00Z"));
        assert_eq!(pre.fix_attempts, 2);

        db.reopen_bug(b.id).unwrap();
        let post = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert_eq!(post.status, "testing", "status → testing");
        assert!(post.confirmed_at.is_none(), "confirmed_at cleared");
        assert_eq!(post.fix_attempts, 2, "fix_attempts unchanged");
    }

    #[test]
    fn test_reopen_bug_from_rejected_keeps_attempts() {
        let db = make_db();
        let rid = make_repo(&db);
        let b = seed_bug(&db, rid, "2026-05-10", "minor", "ui_ux", 1, "rejected");
        db.reopen_bug(b.id).unwrap();
        let post = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert_eq!(post.status, "testing");
        assert!(post.confirmed_at.is_none());
        assert_eq!(post.fix_attempts, 1, "fix_attempts unchanged from rejected");
    }

    #[test]
    fn test_reopen_preserves_entered_testing_invariant() {
        // The bug_events invariant is `COUNT(entered_testing) ==
        // bugs.fix_attempts`. Reopen logs a `reopened` event (NOT
        // `entered_testing`) and leaves fix_attempts unchanged — so both
        // sides of the equality stay in sync.
        let db = make_db();
        let rid = make_repo(&db);
        let b = seed_bug(&db, rid, "2026-05-10", "major", "logic", 0, "created");
        // 2 testing transitions = 2 attempts.
        db.update_bug_status(b.id, "in-progress", None, None).unwrap();
        db.update_bug_status(b.id, "testing", Some(1), None).unwrap();
        db.insert_bug_event(
            b.id, "entered_testing",
            Some("in-progress"), Some("testing"), "2026-05-12T10:00:00Z",
        ).unwrap();
        db.update_bug_status(b.id, "rejected", None, None).unwrap();
        db.update_bug_status(b.id, "testing", Some(2), None).unwrap();
        db.insert_bug_event(
            b.id, "entered_testing",
            Some("rejected"), Some("testing"), "2026-05-14T10:00:00Z",
        ).unwrap();
        db.update_bug_status(b.id, "confirmed", None, Some("2026-05-20T10:00:00Z")).unwrap();

        // Reopen: status → testing, fix_attempts UNCHANGED at 2.
        db.reopen_bug(b.id).unwrap();
        db.insert_bug_event(b.id, "reopened", Some("confirmed"), Some("testing"), "2026-05-21T10:00:00Z").unwrap();

        // Invariant: COUNT(entered_testing) == fix_attempts.
        let bug = db.get_bug_by_id(b.id).unwrap().unwrap();
        let count: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id = ?1 AND event_type = 'entered_testing'",
                rusqlite::params![b.id],
                |r| r.get(0),
            ).unwrap()
        };
        assert_eq!(count, 2, "two entered_testing events");
        assert_eq!(bug.fix_attempts, 2, "fix_attempts matches");
        assert_eq!(count, bug.fix_attempts as i64, "invariant holds after reopen");

        // The reopened event exists but doesn't count toward attempts.
        let reopened_count: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id = ?1 AND event_type = 'reopened'",
                rusqlite::params![b.id],
                |r| r.get(0),
            ).unwrap()
        };
        assert_eq!(reopened_count, 1, "one reopened event logged");
    }

    #[test]
    fn test_reopen_bug_clears_archived_from_md_at() {
        // Confirmed bug after LLM acknowledgement cleanup has archived_from_md_at
        // set. Reopen must clear it so the bug reappears in MD on next regen.
        let db = make_db();
        let rid = make_repo(&db);
        let b = seed_bug(&db, rid, "2026-05-10", "minor", "ui_ux", 1, "testing");
        db.update_bug_status(b.id, "confirmed", None, Some("2026-05-20T10:00:00Z"))
            .unwrap();
        // Simulate LLM cleanup having archived this bug from MD.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE bugs SET archived_from_md_at = '2026-05-21T11:00:00Z' WHERE id = ?1",
                rusqlite::params![b.id],
            )
            .unwrap();
        }

        db.reopen_bug(b.id).unwrap();
        let post = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert_eq!(post.status, "testing");
        assert!(post.confirmed_at.is_none());
        assert!(
            post.archived_from_md_at.is_none(),
            "archived_from_md_at cleared so bug reappears in MD"
        );
    }

    #[test]
    fn test_update_bug_comment_roundtrip() {
        let db = make_db();
        let rid = make_repo(&db);
        let b = seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "created");
        db.update_bug_comment(b.id, Some("fixed by X")).unwrap();
        let b2 = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert_eq!(b2.comment.as_deref(), Some("fixed by X"));
        db.update_bug_comment(b.id, None).unwrap();
        let b3 = db.get_bug_by_id(b.id).unwrap().unwrap();
        assert!(b3.comment.is_none());
    }

    #[test]
    fn test_list_bugs_by_repo_excludes_confirmed_by_default() {
        let db = make_db();
        let rid = make_repo(&db);
        seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "created");
        seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "confirmed");

        let active = db.list_bugs_by_repo(rid, false).unwrap();
        assert_eq!(active.len(), 1);
        assert_ne!(active[0].status, "confirmed");

        let all = db.list_bugs_by_repo(rid, true).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_count_confirmed_bugs() {
        let db = make_db();
        let rid = make_repo(&db);
        seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "created");
        assert_eq!(db.count_confirmed_bugs(rid).unwrap(), 0);
        seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "confirmed");
        seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "confirmed");
        assert_eq!(db.count_confirmed_bugs(rid).unwrap(), 2);
    }

    #[test]
    fn test_get_bug_by_display_id() {
        let db = make_db();
        let rid = make_repo(&db);
        let b = seed_bug(&db, rid, "2026-03-29", "minor", "ui_ux", 0, "created");
        assert_eq!(b.display_id, "B-000001");
        let found = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(found.id, b.id);
        assert!(db.get_bug_by_display_id(rid, "B-999999").unwrap().is_none());
    }

    #[test]
    fn test_bugs_migrated_at_marker() {
        let db = make_db();
        let rid = make_repo(&db);
        assert!(db.get_bugs_migrated_at(rid).unwrap().is_none());
        db.set_bugs_migrated_at(rid, "2026-04-24T10:00:00Z")
            .unwrap();
        assert_eq!(
            db.get_bugs_migrated_at(rid).unwrap().as_deref(),
            Some("2026-04-24T10:00:00Z")
        );
    }

    #[test]
    fn test_utc_now_rfc3339_format() {
        let ts = utc_now_rfc3339();
        assert!(ts.len() >= 20, "got: {}", ts);
        assert!(ts.contains('T'), "got: {}", ts);
        let db = make_db();
        let rid = make_repo(&db);
        db.insert_bug(
            rid, 1, &ts, "seed", "minor", "other", "created", 0, None, None,
        )
        .unwrap();
        let parsed: Option<String> = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT date(created_at) FROM bugs WHERE repository_id = ?1",
                rusqlite::params![rid],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            parsed.is_some(),
            "SQLite date() should parse rfc3339 timestamp"
        );
    }

    // ── Bug events (A3, v0.17.0) ─────────────────────────────────────────────

    #[test]
    fn test_insert_bug_event_writes_row() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r", "r", None, None)
            .unwrap();
        let bug = db
            .insert_bug(
                repo.id,
                1,
                "2026-04-24T00:00:00Z",
                "desc",
                "minor",
                "other",
                "created",
                0,
                None,
                None,
            )
            .unwrap();

        db.insert_bug_event(
            bug.id,
            "entered_testing",
            Some("in-progress"),
            Some("testing"),
            "2026-04-24T12:00:00Z",
        )
        .unwrap();

        let conn = db.conn.lock().unwrap();
        let (typ, from_s, to_s): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT event_type, from_status, to_status FROM bug_events WHERE bug_id = ?1",
                [bug.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(typ, "entered_testing");
        assert_eq!(from_s.as_deref(), Some("in-progress"));
        assert_eq!(to_s.as_deref(), Some("testing"));
    }

    #[test]
    fn test_migrate_bugs_transactional_synthesizes_events() {
        // T-000091: migrate_bugs_for_repo imports fix_attempts from MD without
        // creating bug_events. Subsequent reads from `bug_events.entered_testing`
        // (Dashboard KPI5) would diverge from `bugs.fix_attempts` (StatsSummary).
        // Fix: synthesize per-row events inside the transaction.
        let db = make_db();
        let rid = make_repo(&db);
        let rows = vec![
            (
                1i64,
                FileBugNote {
                    id: "B-000001".to_string(),
                    date: "2026-05-01".to_string(),
                    description: "active bug, 2 attempts".to_string(),
                    severity: "major".to_string(),
                    category: "logic".to_string(),
                    status: "testing".to_string(),
                    fix_attempts: 2,
                    comment: Some("retried twice".to_string()),
                },
            ),
            (
                2i64,
                FileBugNote {
                    id: "B-000002".to_string(),
                    date: "2026-05-02".to_string(),
                    description: "confirmed, 3 attempts".to_string(),
                    severity: "minor".to_string(),
                    category: "ui_ux".to_string(),
                    status: "confirmed".to_string(),
                    fix_attempts: 3,
                    comment: None,
                },
            ),
            (
                3i64,
                FileBugNote {
                    id: "B-000003".to_string(),
                    date: "2026-05-03".to_string(),
                    description: "legacy confirmed, attempts=0".to_string(),
                    severity: "minor".to_string(),
                    category: "other".to_string(),
                    status: "confirmed".to_string(),
                    fix_attempts: 0,
                    comment: None,
                },
            ),
            (
                4i64,
                FileBugNote {
                    id: "B-000004".to_string(),
                    date: "2026-05-04".to_string(),
                    description: "fresh, 0 attempts".to_string(),
                    severity: "minor".to_string(),
                    category: "other".to_string(),
                    status: "created".to_string(),
                    fix_attempts: 0,
                    comment: None,
                },
            ),
        ];
        let report = db
            .migrate_bugs_transactional(rid, &rows, "2026-05-13T10:00:00Z")
            .unwrap();
        assert_eq!(report.imported, 4);
        assert_eq!(report.confirmed_archived, 2);

        let conn = db.conn.lock().unwrap();

        let n1: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id = (SELECT id FROM bugs WHERE display_id = 'B-000001')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n1, 3, "bug 1: created + 2 entered_testing");
        let t1: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE event_type='entered_testing' AND bug_id=(SELECT id FROM bugs WHERE display_id='B-000001')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(t1, 2);

        let n2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id = (SELECT id FROM bugs WHERE display_id = 'B-000002')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n2, 5, "bug 2: created + 3 entered_testing + confirmed");

        let n3: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id = (SELECT id FROM bugs WHERE display_id = 'B-000003')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n3, 3,
            "legacy confirmed/0-attempts gets 1 synthetic entered_testing"
        );

        let n4: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id = (SELECT id FROM bugs WHERE display_id = 'B-000004')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n4, 1, "fresh created bug: only 'created' event");
    }

    #[test]
    fn test_backfill_with_legacy_bugs() {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        let repo = db
            .insert_local_repository("/tmp/legacy", "legacy", None, None)
            .unwrap();

        {
            let c = db.conn.lock().unwrap();
            c.execute(
                "INSERT INTO bugs (repository_id, numeric_id, display_id, created_at,
                    description, severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at)
                 VALUES (?1, 1, 'B-000001', '2026-04-01T00:00:00Z', 'legacy', 'minor', 'other',
                         'confirmed', 3, NULL, '2026-04-10T00:00:00Z', NULL)",
                [repo.id],
            )
            .unwrap();
        }

        db.backfill_bug_events_for_existing().unwrap();

        let conn = db.conn.lock().unwrap();
        let n_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id = (SELECT id FROM bugs WHERE display_id='B-000001')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n_events, 5,
            "back-fill must synthesize all events for legacy confirmed bug"
        );

        let n_attempts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE event_type='entered_testing' AND bug_id=(SELECT id FROM bugs WHERE display_id='B-000001')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n_attempts, 3,
            "entered_testing count must match fix_attempts"
        );
    }

    #[test]
    fn test_backfill_guards_invalid_legacy_state() {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        let repo = db
            .insert_local_repository("/tmp/bad", "bad", None, None)
            .unwrap();

        {
            let c = db.conn.lock().unwrap();
            c.execute(
                "INSERT INTO bugs (repository_id, numeric_id, display_id, created_at,
                    description, severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at)
                 VALUES (?1, 1, 'B-000002', '2026-04-01T00:00:00Z', 'corrupt', 'minor', 'other',
                         'confirmed', 0, NULL, '2026-04-10T00:00:00Z', NULL)",
                [repo.id],
            )
            .unwrap();
        }
        db.backfill_bug_events_for_existing().unwrap();

        let conn = db.conn.lock().unwrap();
        let n_attempts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE event_type='entered_testing'
                 AND bug_id=(SELECT id FROM bugs WHERE display_id='B-000002')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n_attempts, 1,
            "guard must force 1 synthetic attempt for corrupt legacy"
        );
    }

    #[test]
    fn test_backfill_is_idempotent() {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        let repo = db
            .insert_local_repository("/tmp/idem", "idem", None, None)
            .unwrap();
        {
            let c = db.conn.lock().unwrap();
            c.execute(
                "INSERT INTO bugs (repository_id, numeric_id, display_id, created_at,
                    description, severity, category, status, fix_attempts, comment, confirmed_at, archived_from_md_at)
                 VALUES (?1, 1, 'B-000003', '2026-04-01T00:00:00Z', 'idem', 'minor', 'other',
                         'created', 0, NULL, NULL, NULL)",
                [repo.id],
            )
            .unwrap();
        }
        db.backfill_bug_events_for_existing().unwrap();
        db.backfill_bug_events_for_existing().unwrap();

        let conn = db.conn.lock().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id=(SELECT id FROM bugs WHERE display_id='B-000003')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "second backfill must be a no-op");
    }

    #[test]
    fn test_create_then_resolve_writes_events() {
        // Smoke test: insert_bug + insert_bug_event walk through a typical
        // bug lifecycle, verifying bug_events accumulate without violating
        // the CHECK or FK constraints.
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r", "r", None, None)
            .unwrap();

        // Simulate create_bug flow
        let bug = db
            .insert_bug(
                repo.id,
                1,
                "2026-04-24T10:00:00Z",
                "desc",
                "minor",
                "other",
                "created",
                0,
                None,
                None,
            )
            .unwrap();
        db.insert_bug_event(bug.id, "created", None, Some("created"), &bug.created_at)
            .unwrap();

        // Simulate in-progress → testing transition
        let ts1 = "2026-04-24T11:00:00Z";
        db.update_bug_status(bug.id, "testing", None, None).unwrap();
        db.insert_bug_event(
            bug.id,
            "entered_testing",
            Some("in-progress"),
            Some("testing"),
            ts1,
        )
        .unwrap();

        // Simulate resolve_bug
        let ts2 = "2026-04-24T12:00:00Z";
        db.update_bug_status(bug.id, "confirmed", None, Some(ts2))
            .unwrap();
        db.insert_bug_event(bug.id, "confirmed", Some("testing"), Some("confirmed"), ts2)
            .unwrap();

        let conn = db.conn.lock().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM bug_events WHERE bug_id=?1",
                [bug.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 3,
            "expected 3 events: created + entered_testing + confirmed"
        );
    }
}
