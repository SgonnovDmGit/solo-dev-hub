// T-000094: lifetime stats summaries for the per-repo and per-project Stats
// tabs (v0.22.0 redesign). KPI + categories + lifetime span; project-scope
// additionally returns `top_hot_repos` (calls into `dashboard::top_hot_repos_in_project`)
// and `repo_count`.

use super::*;

impl AppDb {
    /// v0.22.0 (T-000054): one-shot lifetime summary for the redesigned per-repo
    /// Stats tab. Returns KPI + categories + lifetime span. `top_hot_repos` and
    /// `repo_count` are always None for repo-scope.
    pub fn stats_summary_for_repo(&self, repo_id: i64) -> SqlResult<StatsSummary> {
        let conn = self.conn.lock().unwrap();
        // KPI
        // B-000013: "active" = anything not yet closed → status != 'confirmed'
        // includes rejected (rejected means user disagreed with last fix attempt
        // and the bug is back in flight, NOT closed). Earlier strict whitelist
        // ('created','in-progress','testing') silently dropped rejected from
        // the KPI even though rejected bugs are very much active work.
        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM bugs WHERE repository_id = ?1 AND status != 'confirmed'",
            rusqlite::params![repo_id],
            |r| r.get(0),
        )?;
        let active_critical: i64 = conn.query_row(
            "SELECT COUNT(*) FROM bugs WHERE repository_id = ?1 AND status != 'confirmed' AND severity = 'critical'",
            rusqlite::params![repo_id],
            |r| r.get(0),
        )?;
        let closed_total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM bugs WHERE repository_id = ?1 AND status = 'confirmed'",
            rusqlite::params![repo_id],
            |r| r.get(0),
        )?;
        let created_total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM bugs WHERE repository_id = ?1",
            rusqlite::params![repo_id],
            |r| r.get(0),
        )?;
        let avg_attempts: f64 = conn.query_row(
            "SELECT COALESCE(AVG(fix_attempts), 0) FROM bugs WHERE repository_id = ?1 AND status = 'confirmed'",
            rusqlite::params![repo_id],
            |r| r.get(0),
        )?;
        // Median attempts via ORDER BY + LIMIT 1 OFFSET (count/2). For even counts
        // we just take the upper-mid (no averaging) — acceptable for usability.
        let median_attempts: f64 = if closed_total == 0 {
            0.0
        } else {
            let offset = closed_total / 2;
            conn.query_row(
                "SELECT fix_attempts FROM bugs WHERE repository_id = ?1 AND status = 'confirmed' ORDER BY fix_attempts LIMIT 1 OFFSET ?2",
                rusqlite::params![repo_id, offset],
                |r| {
                    let v: i64 = r.get(0)?;
                    Ok(v as f64)
                },
            )?
        };
        let fix_rate: f64 = if created_total == 0 {
            0.0
        } else {
            closed_total as f64 / created_total as f64
        };
        let kpi = StatsKpi {
            active,
            active_critical,
            closed_total,
            avg_attempts,
            median_attempts,
            fix_rate,
            created_total,
        };

        // Lifetime since: MIN(bugs.created_at) → fallback repositories.added_at
        let lifetime_since: Option<String> = match conn.query_row(
            "SELECT date(MIN(created_at)) FROM bugs WHERE repository_id = ?1",
            rusqlite::params![repo_id],
            |r| r.get::<_, Option<String>>(0),
        )? {
            Some(d) => Some(d),
            None => conn.query_row(
                "SELECT date(added_at) FROM repositories WHERE id = ?1",
                rusqlite::params![repo_id],
                |r| r.get::<_, Option<String>>(0),
            )?,
        };
        let days_history: i64 = match &lifetime_since {
            Some(d) => conn.query_row(
                "SELECT CAST(julianday(date('now')) - julianday(?1) AS INTEGER)",
                rusqlite::params![d],
                |r| r.get(0),
            ).unwrap_or(0),
            None => 0,
        };

        // Categories
        let mut stmt = conn.prepare(
            "SELECT category, COUNT(*) AS total,
                    SUM(CASE WHEN status='confirmed' THEN 1 ELSE 0 END) AS closed
               FROM bugs WHERE repository_id = ?1
              GROUP BY category
              ORDER BY (CAST(SUM(CASE WHEN status='confirmed' THEN 1 ELSE 0 END) AS REAL) /
                        NULLIF(COUNT(*), 0)) DESC, category ASC",
        )?;
        let categories: Vec<CategoryBar> = stmt
            .query_map(rusqlite::params![repo_id], |r| {
                let category: String = r.get(0)?;
                let total: i64 = r.get(1)?;
                let closed: i64 = r.get(2)?;
                let percent = if total == 0 { 0.0 } else { (closed as f64 / total as f64) * 100.0 };
                Ok(CategoryBar { category, total, closed, percent })
            })?
            .collect::<SqlResult<Vec<_>>>()?;

        Ok(StatsSummary {
            kpi,
            categories,
            top_hot_repos: None,
            lifetime_since,
            days_history,
            repo_count: None,
        })
    }

    /// v0.22.0 (T-000054): one-shot lifetime summary for the redesigned per-project
    /// Stats tab. Aggregates across all repos in the project via JOIN repositories.
    /// Always populates `top_hot_repos` (Some, possibly empty) and `repo_count` (Some).
    pub fn stats_summary_for_project(&self, project_id: i64) -> SqlResult<StatsSummary> {
        let conn = self.conn.lock().unwrap();
        let scope_filter = " WHERE r.project_id = ?1";

        // KPI via JOIN repositories. B-000013: same fix as repo-level — include
        // rejected in active (status != 'confirmed' is the canonical "not closed").
        let active: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM bugs b JOIN repositories r ON b.repository_id = r.id{} AND b.status != 'confirmed'", scope_filter),
            rusqlite::params![project_id],
            |r| r.get(0),
        )?;
        let active_critical: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM bugs b JOIN repositories r ON b.repository_id = r.id{} AND b.status != 'confirmed' AND b.severity = 'critical'", scope_filter),
            rusqlite::params![project_id],
            |r| r.get(0),
        )?;
        let closed_total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM bugs b JOIN repositories r ON b.repository_id = r.id{} AND b.status = 'confirmed'", scope_filter),
            rusqlite::params![project_id],
            |r| r.get(0),
        )?;
        let created_total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM bugs b JOIN repositories r ON b.repository_id = r.id{}", scope_filter),
            rusqlite::params![project_id],
            |r| r.get(0),
        )?;
        let avg_attempts: f64 = conn.query_row(
            &format!("SELECT COALESCE(AVG(b.fix_attempts), 0) FROM bugs b JOIN repositories r ON b.repository_id = r.id{} AND b.status = 'confirmed'", scope_filter),
            rusqlite::params![project_id],
            |r| r.get(0),
        )?;
        let median_attempts: f64 = if closed_total == 0 {
            0.0
        } else {
            let offset = closed_total / 2;
            conn.query_row(
                &format!("SELECT b.fix_attempts FROM bugs b JOIN repositories r ON b.repository_id = r.id{} AND b.status = 'confirmed' ORDER BY b.fix_attempts LIMIT 1 OFFSET ?2", scope_filter),
                rusqlite::params![project_id, offset],
                |r| {
                    let v: i64 = r.get(0)?;
                    Ok(v as f64)
                },
            )?
        };
        let fix_rate: f64 = if created_total == 0 {
            0.0
        } else {
            closed_total as f64 / created_total as f64
        };
        let kpi = StatsKpi {
            active,
            active_critical,
            closed_total,
            avg_attempts,
            median_attempts,
            fix_rate,
            created_total,
        };

        // Lifetime since: MIN(bugs.created_at) → MIN(repositories.added_at) → projects.created_at
        let lifetime_since: Option<String> = match conn.query_row(
            "SELECT date(MIN(b.created_at)) FROM bugs b JOIN repositories r ON b.repository_id = r.id WHERE r.project_id = ?1",
            rusqlite::params![project_id],
            |r| r.get::<_, Option<String>>(0),
        )? {
            Some(d) => Some(d),
            None => match conn.query_row(
                "SELECT date(MIN(added_at)) FROM repositories WHERE project_id = ?1",
                rusqlite::params![project_id],
                |r| r.get::<_, Option<String>>(0),
            )? {
                Some(d) => Some(d),
                None => conn.query_row(
                    "SELECT date(created_at) FROM projects WHERE id = ?1",
                    rusqlite::params![project_id],
                    |r| r.get::<_, Option<String>>(0),
                )?,
            },
        };
        let days_history: i64 = match &lifetime_since {
            Some(d) => conn.query_row(
                "SELECT CAST(julianday(date('now')) - julianday(?1) AS INTEGER)",
                rusqlite::params![d],
                |r| r.get(0),
            ).unwrap_or(0),
            None => 0,
        };

        let repo_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM repositories WHERE project_id = ?1",
            rusqlite::params![project_id],
            |r| r.get(0),
        )?;

        // Categories — same pattern, JOIN'd
        let mut stmt = conn.prepare(
            "SELECT b.category, COUNT(*) AS total,
                    SUM(CASE WHEN b.status='confirmed' THEN 1 ELSE 0 END) AS closed
               FROM bugs b JOIN repositories r ON b.repository_id = r.id
              WHERE r.project_id = ?1
              GROUP BY b.category
              ORDER BY (CAST(SUM(CASE WHEN b.status='confirmed' THEN 1 ELSE 0 END) AS REAL) /
                        NULLIF(COUNT(*), 0)) DESC, b.category ASC",
        )?;
        let categories: Vec<CategoryBar> = stmt
            .query_map(rusqlite::params![project_id], |r| {
                let category: String = r.get(0)?;
                let total: i64 = r.get(1)?;
                let closed: i64 = r.get(2)?;
                let percent = if total == 0 { 0.0 } else { (closed as f64 / total as f64) * 100.0 };
                Ok(CategoryBar { category, total, closed, percent })
            })?
            .collect::<SqlResult<Vec<_>>>()?;

        // Drop conn lock before calling top_hot_repos_in_project (it acquires its own)
        drop(stmt);
        drop(conn);
        let top_hot_repos = Some(self.top_hot_repos_in_project(project_id, 3)?);

        Ok(StatsSummary {
            kpi,
            categories,
            top_hot_repos,
            lifetime_since,
            days_history,
            repo_count: Some(repo_count),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

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
    fn test_stats_summary_for_repo_basic() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let r = db.insert_local_repository("/tmp/r", "r", Some(p.id), Some("server")).unwrap();
        seed_bug(&db, r.id, "2026-04-01", "critical", "logic", 2, "confirmed");
        seed_bug(&db, r.id, "2026-04-02", "major", "ui_ux", 0, "in-progress");
        seed_bug(&db, r.id, "2026-04-03", "minor", "ui_ux", 1, "testing");

        let s = db.stats_summary_for_repo(r.id).unwrap();
        assert_eq!(s.kpi.active, 2);
        assert_eq!(s.kpi.closed_total, 1);
        assert_eq!(s.kpi.created_total, 3);
        assert_eq!(s.kpi.active_critical, 0); // critical one was confirmed
        assert!(s.lifetime_since.is_some());
        assert!(s.top_hot_repos.is_none(), "repo-scope must not include top_hot");
        assert!(s.repo_count.is_none());
        assert!(!s.categories.is_empty());
    }

    #[test]
    fn test_stats_summary_for_repo_empty_falls_back_to_added_at() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let r = db.insert_local_repository("/tmp/r", "r", Some(p.id), Some("server")).unwrap();
        // No bugs → lifetime_since falls back to repositories.added_at
        let s = db.stats_summary_for_repo(r.id).unwrap();
        assert_eq!(s.kpi.active, 0);
        assert_eq!(s.kpi.closed_total, 0);
        assert!(s.lifetime_since.is_some(), "fallback to added_at must yield a date");
    }

    #[test]
    fn test_stats_summary_for_repo_categories_sorted_by_percent_closed_desc() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let r = db.insert_local_repository("/tmp/r", "r", Some(p.id), Some("server")).unwrap();
        // logic: 2 bugs, both confirmed → 100%
        seed_bug(&db, r.id, "2026-04-01", "critical", "logic", 2, "confirmed");
        seed_bug(&db, r.id, "2026-04-02", "critical", "logic", 1, "confirmed");
        // ui_ux: 2 bugs, 1 confirmed → 50%
        seed_bug(&db, r.id, "2026-04-03", "minor", "ui_ux", 0, "confirmed");
        seed_bug(&db, r.id, "2026-04-04", "minor", "ui_ux", 0, "in-progress");
        // database: 1 bug, 0 confirmed → 0%
        seed_bug(&db, r.id, "2026-04-05", "major", "database", 0, "in-progress");

        let s = db.stats_summary_for_repo(r.id).unwrap();
        assert_eq!(s.categories.len(), 3);
        // Order: logic (100%), ui_ux (50%), database (0%)
        assert_eq!(s.categories[0].category, "logic");
        assert_eq!(s.categories[1].category, "ui_ux");
        assert_eq!(s.categories[2].category, "database");
    }

    #[test]
    fn test_stats_summary_for_repo_avg_and_median_attempts() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let r = db.insert_local_repository("/tmp/r", "r", Some(p.id), Some("server")).unwrap();
        // Confirmed bugs with attempts: 1, 2, 3, 5. avg=2.75, median=upper-mid=3.
        seed_bug(&db, r.id, "2026-04-01", "minor", "ui_ux", 1, "confirmed");
        seed_bug(&db, r.id, "2026-04-02", "minor", "ui_ux", 2, "confirmed");
        seed_bug(&db, r.id, "2026-04-03", "minor", "ui_ux", 3, "confirmed");
        seed_bug(&db, r.id, "2026-04-04", "minor", "ui_ux", 5, "confirmed");
        let s = db.stats_summary_for_repo(r.id).unwrap();
        assert_eq!(s.kpi.avg_attempts, 2.75);
        assert_eq!(s.kpi.median_attempts, 3.0);
    }

    #[test]
    fn test_stats_summary_includes_rejected_in_active() {
        // B-000013: rejected bugs must count as active (not closed).
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let r = db.insert_local_repository("/tmp/r", "r", Some(p.id), Some("server")).unwrap();
        seed_bug(&db, r.id, "2026-04-01", "critical", "logic", 2, "rejected");
        seed_bug(&db, r.id, "2026-04-02", "minor", "ui_ux", 0, "created");
        seed_bug(&db, r.id, "2026-04-03", "minor", "ui_ux", 1, "confirmed");
        let s = db.stats_summary_for_repo(r.id).unwrap();
        assert_eq!(s.kpi.active, 2, "rejected + created must both count as active");
        assert_eq!(s.kpi.active_critical, 1, "rejected critical must count");
        assert_eq!(s.kpi.closed_total, 1);
    }

    #[test]
    fn test_stats_summary_for_project_aggregates_across_repos_with_top_hot() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/tmp/r1", "r1", Some(p.id), Some("server")).unwrap();
        let r2 = db.insert_local_repository("/tmp/r2", "r2", Some(p.id), Some("client")).unwrap();
        seed_bug(&db, r1.id, "2026-04-01", "critical", "logic", 1, "in-progress");
        seed_bug(&db, r2.id, "2026-04-02", "major", "ui_ux", 0, "in-progress");
        seed_bug(&db, r2.id, "2026-04-03", "minor", "ui_ux", 0, "confirmed");

        let s = db.stats_summary_for_project(p.id).unwrap();
        assert_eq!(s.kpi.active, 2);
        assert_eq!(s.kpi.closed_total, 1);
        assert_eq!(s.kpi.created_total, 3);
        assert!(s.top_hot_repos.is_some());
        assert_eq!(s.repo_count, Some(2));
    }

    #[test]
    fn test_stats_summary_for_project_empty() {
        let db = make_db();
        let p = db.create_project("Empty", None, "standard").unwrap();
        let s = db.stats_summary_for_project(p.id).unwrap();
        assert_eq!(s.kpi.active, 0);
        assert_eq!(s.kpi.closed_total, 0);
        assert_eq!(s.kpi.created_total, 0);
        assert!(s.top_hot_repos.is_some());
        assert_eq!(s.top_hot_repos.as_ref().unwrap().len(), 0);
        assert_eq!(s.repo_count, Some(0));
        // Fallback to projects.created_at
        assert!(s.lifetime_since.is_some());
    }

    #[test]
    fn test_stats_summary_for_project_repos_no_bugs() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        db.insert_local_repository("/tmp/r1", "r1", Some(p.id), Some("server")).unwrap();
        db.insert_local_repository("/tmp/r2", "r2", Some(p.id), Some("client")).unwrap();
        // No bugs → lifetime_since falls back to MIN(repos.added_at).
        let s = db.stats_summary_for_project(p.id).unwrap();
        assert_eq!(s.repo_count, Some(2));
        assert!(s.lifetime_since.is_some());
    }
}
