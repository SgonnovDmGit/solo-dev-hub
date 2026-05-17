// T-000094: dashboard counter queries + KPI helpers + per-day flow + top-hot
// + category efficiency. Moved from db.rs.
//
// T-000096: extracted `run_count_with_project_filter` to remove the repeated
// "build fragment + thread params + lock + query_row" boilerplate from the
// simple-counter call-sites (count_active_bugs, count_active_bugs_with_severity,
// count_closed_bugs_in_period, count_opened_bugs_in_period). The more complex
// queries (avg_attempts, top_hot, bugs_per_day, category_efficiency) keep
// their own SQL because they return non-i64 shapes or bind the project filter
// in multiple places.

use super::*;
use rusqlite::ToSql;

impl AppDb {
    // ── Dashboard KPI helpers ──────────────────────────────────────────────────

    /// Build an optional project-filter SQL fragment + its bindings.
    /// `None` or empty slice → no filter (all repos).
    pub(super) fn project_filter_fragment(project_ids: Option<&[i64]>) -> (String, Vec<i64>) {
        match project_ids {
            None => (String::new(), vec![]),
            Some(ids) if ids.is_empty() => (String::new(), vec![]),
            Some(ids) => {
                let placeholders = vec!["?"; ids.len()].join(",");
                (
                    format!(
                        " AND repository_id IN (SELECT id FROM repositories WHERE project_id IN ({}))",
                        placeholders
                    ),
                    ids.to_vec(),
                )
            }
        }
    }

    /// T-000096: run a `SELECT COUNT(*)` query with an optional project filter.
    ///
    /// `base_sql` ends with the static portion of the WHERE clause. The project
    /// filter fragment is appended (`AND repository_id IN (...)` style). Any
    /// fixed (pre-filter) parameters bind first, then the project ids — order
    /// the placeholders in `base_sql` accordingly.
    fn run_count_with_project_filter(
        &self,
        base_sql: &str,
        fixed_params: &[&dyn ToSql],
        project_ids: Option<&[i64]>,
    ) -> SqlResult<i64> {
        let (filter, ids) = Self::project_filter_fragment(project_ids);
        let sql = format!("{}{}", base_sql, filter);
        let conn = self.conn.lock().unwrap();
        let mut params: Vec<&dyn ToSql> = fixed_params.to_vec();
        let ids_refs: Vec<&dyn ToSql> = ids.iter().map(|v| v as &dyn ToSql).collect();
        params.extend(ids_refs);
        conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| r.get(0))
    }

    /// Count bugs with status != 'confirmed' (optionally scoped to projects).
    pub fn count_active_bugs(&self, project_ids: Option<&[i64]>) -> SqlResult<i64> {
        self.run_count_with_project_filter(
            "SELECT COUNT(*) FROM bugs WHERE status != 'confirmed'",
            &[],
            project_ids,
        )
    }

    /// Count active bugs filtered by severity (optionally scoped to projects).
    pub fn count_active_bugs_with_severity(
        &self,
        project_ids: Option<&[i64]>,
        severity: &str,
    ) -> SqlResult<i64> {
        self.run_count_with_project_filter(
            "SELECT COUNT(*) FROM bugs WHERE status != 'confirmed' AND severity = ?1",
            &[&severity],
            project_ids,
        )
    }

    /// Count confirmed bugs whose `confirmed_at` date falls within [start, end] (YYYY-MM-DD).
    pub fn count_closed_bugs_in_period(
        &self,
        project_ids: Option<&[i64]>,
        start: &str,
        end: &str,
    ) -> SqlResult<i64> {
        self.run_count_with_project_filter(
            "SELECT COUNT(*) FROM bugs \
             WHERE status = 'confirmed' \
               AND date(confirmed_at) BETWEEN ?1 AND ?2",
            &[&start, &end],
            project_ids,
        )
    }

    /// Count bugs whose `created_at` date falls within [start, end] (YYYY-MM-DD),
    /// regardless of current status.
    pub fn count_opened_bugs_in_period(
        &self,
        project_ids: Option<&[i64]>,
        start: &str,
        end: &str,
    ) -> SqlResult<i64> {
        self.run_count_with_project_filter(
            "SELECT COUNT(*) FROM bugs \
             WHERE date(created_at) BETWEEN ?1 AND ?2",
            &[&start, &end],
            project_ids,
        )
    }

    /// KPI 5: AVG(fix_attempts) over bugs closed in period.
    /// Returns None if no closed bugs in period (AVG of empty set = NULL).
    pub fn avg_attempts_per_closed_in_period(
        &self,
        project_ids: Option<&[i64]>,
        start: &str,
        end: &str,
    ) -> SqlResult<Option<f64>> {
        let (filter, ids) = Self::project_filter_fragment(project_ids);
        let sql = format!(
            "SELECT AVG(attempts) FROM (
                 SELECT COUNT(*) AS attempts
                 FROM bug_events
                 WHERE event_type = 'entered_testing'
                   AND bug_id IN (
                     SELECT id FROM bugs
                     WHERE status = 'confirmed'
                       AND date(confirmed_at) BETWEEN ?1 AND ?2
                       {}
                   )
                 GROUP BY bug_id
             )",
            filter
        );
        let conn = self.conn.lock().unwrap();
        let mut all: Vec<&dyn ToSql> = vec![&start, &end];
        let ids_refs: Vec<&dyn ToSql> = ids.iter().map(|v| v as &dyn ToSql).collect();
        all.extend(ids_refs);
        conn.query_row(&sql, rusqlite::params_from_iter(all.iter()), |r| {
            r.get::<_, Option<f64>>(0)
        })
    }

    /// Top-N projects by weighted heat-score combining bug burden + recent activity.
    ///
    /// Score = critical_active × 50 + major_active × 15 + active_total × 1
    ///       + bugs_closed_in_period × 2 + tasks_done_in_period × 1
    ///
    /// Threshold: `score > 0`. Sort: score DESC, name ASC (deterministic tiebreaker).
    ///
    /// Period semantics: `period = Some((start, end))` uses dashboard-window filtering
    /// for `bugs_closed` and `tasks_done` terms; `None` → lifetime mode (all-time counts,
    /// implemented via 0001..9999 date sentinels — SQLite `date()` handles these cleanly).
    ///
    /// Severity counts (critical / major / active) are always **current state** — not
    /// period-bounded — because they represent present pressure, not history.
    pub fn top_hot_projects(
        &self,
        project_ids: Option<&[i64]>,
        period: Option<(&str, &str)>,
        limit: i64,
    ) -> SqlResult<Vec<TopHotProject>> {
        let (period_start, period_end) = period.unwrap_or(("0001-01-01", "9999-12-31"));
        let (proj_filter, proj_ids) = match project_ids {
            None => (String::new(), vec![]),
            Some(ids) if ids.is_empty() => (String::new(), vec![]),
            Some(ids) => {
                let p = vec!["?"; ids.len()].join(",");
                (format!(" AND p.id IN ({})", p), ids.to_vec())
            }
        };
        // Period params bind in 2 places (bugs_closed + tasks_done subquery).
        // Order of binding: start, end, start, end, [proj_ids...], limit.
        let sql = format!(
            "SELECT * FROM (
                SELECT p.id AS project_id, p.name AS name,
                       COALESCE(SUM(CASE WHEN b.severity='critical' AND b.status != 'confirmed' THEN 1 ELSE 0 END), 0) AS critical,
                       COALESCE(SUM(CASE WHEN b.severity='major' AND b.status != 'confirmed' THEN 1 ELSE 0 END), 0) AS major,
                       COALESCE(SUM(CASE WHEN b.id IS NOT NULL AND b.status != 'confirmed' THEN 1 ELSE 0 END), 0) AS active,
                       COALESCE(SUM(CASE WHEN b.status = 'confirmed' AND date(b.confirmed_at) BETWEEN ? AND ? THEN 1 ELSE 0 END), 0) AS bugs_closed,
                       (SELECT COUNT(*) FROM task_events te
                          JOIN tasks t ON t.id = te.task_id
                          JOIN repositories r2 ON r2.id = t.repository_id
                         WHERE r2.project_id = p.id
                           AND te.event_type = 'done'
                           AND date(te.ts) BETWEEN ? AND ?
                       ) AS tasks_done
                FROM projects p
                LEFT JOIN repositories r ON r.project_id = p.id
                LEFT JOIN bugs b ON b.repository_id = r.id
                WHERE 1=1{}
                GROUP BY p.id, p.name
            ) AS scored
            WHERE (critical * 50 + major * 15 + active + bugs_closed * 2 + tasks_done) > 0
            ORDER BY (critical * 50 + major * 15 + active + bugs_closed * 2 + tasks_done) DESC, name ASC
            LIMIT ?",
            proj_filter
        );
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;

        let mut all: Vec<&dyn ToSql> = Vec::with_capacity(4 + proj_ids.len() + 1);
        all.push(&period_start);
        all.push(&period_end);
        all.push(&period_start);
        all.push(&period_end);
        let ids_refs: Vec<&dyn ToSql> = proj_ids.iter().map(|v| v as &dyn ToSql).collect();
        all.extend(ids_refs);
        all.push(&limit);

        let rows = stmt
            .query_map(rusqlite::params_from_iter(all.iter()), |r| {
                Ok(TopHotProject {
                    project_id: r.get(0)?,
                    name: r.get(1)?,
                    critical: r.get(2)?,
                    major: r.get(3)?,
                    active: r.get(4)?,
                    bugs_closed: r.get(5)?,
                    tasks_done: r.get(6)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    /// v0.22.0 (T-000054): top-N hot repos within a single project.
    /// Mirror of `top_hot_projects` but ranked at repo level, scoped to one project.
    /// Same weighted heat-score (critical×50 + major×15 + active + closed×2 + tasks×1).
    /// `period = None` → lifetime mode (Stats tab default). Used by per-project Stats tab.
    pub fn top_hot_repos_in_project(
        &self,
        project_id: i64,
        period: Option<(&str, &str)>,
        limit: i64,
    ) -> SqlResult<Vec<HotRepo>> {
        let (period_start, period_end) = period.unwrap_or(("0001-01-01", "9999-12-31"));
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM (
                SELECT r.id AS repo_id, r.github_name, r.description,
                       COALESCE(SUM(CASE WHEN b.severity='critical' AND b.status != 'confirmed' THEN 1 ELSE 0 END), 0) AS critical,
                       COALESCE(SUM(CASE WHEN b.severity='major' AND b.status != 'confirmed' THEN 1 ELSE 0 END), 0) AS major,
                       COALESCE(SUM(CASE WHEN b.id IS NOT NULL AND b.status != 'confirmed' THEN 1 ELSE 0 END), 0) AS active,
                       COALESCE(SUM(CASE WHEN b.status = 'confirmed' AND date(b.confirmed_at) BETWEEN ?1 AND ?2 THEN 1 ELSE 0 END), 0) AS bugs_closed,
                       (SELECT COUNT(*) FROM task_events te
                          JOIN tasks t ON t.id = te.task_id
                         WHERE t.repository_id = r.id
                           AND te.event_type = 'done'
                           AND date(te.ts) BETWEEN ?1 AND ?2
                       ) AS tasks_done
                  FROM repositories r
                  LEFT JOIN bugs b ON b.repository_id = r.id
                 WHERE r.project_id = ?3
                 GROUP BY r.id, r.github_name, r.description
            ) AS scored
            WHERE (critical * 50 + major * 15 + active + bugs_closed * 2 + tasks_done) > 0
            ORDER BY (critical * 50 + major * 15 + active + bugs_closed * 2 + tasks_done) DESC, github_name ASC
            LIMIT ?4",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![period_start, period_end, project_id, limit], |r| {
                Ok(HotRepo {
                    repo_id: r.get(0)?,
                    github_name: r.get(1)?,
                    // description in DB is nullable; pass through as Option<String>
                    description: r.get(2)?,
                    critical: r.get(3)?,
                    major: r.get(4)?,
                    active: r.get(5)?,
                    bugs_closed: r.get(6)?,
                    tasks_done: r.get(7)?,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }

    /// Per-day bug counts (opened + closed). Missing days filled with zeros.
    pub fn bugs_per_day(
        &self,
        project_ids: Option<&[i64]>,
        start: &str,
        end: &str,
    ) -> SqlResult<Vec<DailyFlowDay>> {
        let (filter, ids) = Self::project_filter_fragment(project_ids);

        let sql_opened = format!(
            "SELECT date(created_at) AS d, COUNT(*) \
             FROM bugs \
             WHERE date(created_at) BETWEEN ?1 AND ?2{}\
             GROUP BY d",
            filter
        );
        let sql_closed = format!(
            "SELECT date(confirmed_at) AS d, COUNT(*) \
             FROM bugs \
             WHERE status='confirmed' \
               AND date(confirmed_at) BETWEEN ?1 AND ?2{}\
             GROUP BY d",
            filter
        );
        let conn = self.conn.lock().unwrap();
        let mut params: Vec<&dyn ToSql> = vec![&start, &end];
        let ids_refs: Vec<&dyn ToSql> = ids.iter().map(|v| v as &dyn ToSql).collect();
        params.extend(ids_refs);

        use std::collections::BTreeMap;
        let mut opened_map: BTreeMap<String, i64> = BTreeMap::new();
        let mut closed_map: BTreeMap<String, i64> = BTreeMap::new();

        let mut s = conn.prepare(&sql_opened)?;
        let rows = s.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (d, n) = row?;
            opened_map.insert(d, n);
        }
        drop(s);

        let mut s2 = conn.prepare(&sql_closed)?;
        let rows2 = s2.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows2 {
            let (d, n) = row?;
            closed_map.insert(d, n);
        }
        drop(s2);

        let start_d = chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
        let end_d = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d")
            .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))?;
        let today = chrono::Local::now().date_naive();

        let mut out = Vec::new();
        let mut d = start_d;
        while d <= end_d {
            let key = d.format("%Y-%m-%d").to_string();
            out.push(DailyFlowDay {
                date: key.clone(),
                opened: Some(*opened_map.get(&key).unwrap_or(&0)),
                closed: Some(*closed_map.get(&key).unwrap_or(&0)),
                done: None,
                is_future: d > today,
            });
            match d.succ_opt() {
                Some(next) => d = next,
                None => break,
            }
        }
        Ok(out)
    }

    /// v0.17.0: list repos that have a non-null local_path.
    /// If `project_ids` is None or empty, returns ALL repos with a local_path.
    /// If `project_ids` is Some with values, filters to those project_ids only.
    pub fn list_repos_with_local_path(
        &self,
        project_ids: Option<&[i64]>,
    ) -> SqlResult<Vec<Repository>> {
        let sql = match project_ids {
            None => "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target FROM repositories WHERE local_path IS NOT NULL".to_string(),
            Some(ids) if ids.is_empty() => {
                "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target FROM repositories WHERE local_path IS NOT NULL".to_string()
            }
            Some(ids) => {
                let p = vec!["?"; ids.len()].join(",");
                format!(
                    "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target FROM repositories WHERE local_path IS NOT NULL AND project_id IN ({})",
                    p
                )
            }
        };
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;
        let ids_vec: Vec<i64> = project_ids.unwrap_or(&[]).to_vec();
        let rows = stmt.query_map(
            rusqlite::params_from_iter(ids_vec.iter()),
            row_to_repo,
        )?;
        rows.collect()
    }

    /// Category efficiency bars data. Returns rows for all categories that have touched>0.
    pub fn category_efficiency(
        &self,
        project_ids: Option<&[i64]>,
        start: &str,
        end: &str,
    ) -> SqlResult<Vec<CategoryEfficiencyRow>> {
        let (filter, ids) = Self::project_filter_fragment(project_ids);
        let sql = format!(
            "SELECT category,
                    COUNT(*) AS touched,
                    SUM(CASE WHEN status='confirmed' \
                          AND date(confirmed_at) BETWEEN ?1 AND ?2 THEN 1 ELSE 0 END) AS closed,
                    COALESCE((
                        SELECT COUNT(*) FROM bug_events e
                        WHERE e.event_type='entered_testing'
                          AND date(e.ts) BETWEEN ?1 AND ?2
                          AND e.bug_id IN (
                            SELECT id FROM bugs b2 WHERE b2.category = bugs.category{}
                              AND (date(b2.created_at) BETWEEN ?1 AND ?2
                                   OR date(b2.confirmed_at) BETWEEN ?1 AND ?2)
                          )
                    ), 0) AS attempts
             FROM bugs
             WHERE (date(created_at) BETWEEN ?1 AND ?2
                    OR date(confirmed_at) BETWEEN ?1 AND ?2)
                   {}
             GROUP BY category",
            filter, filter
        );
        let conn = self.conn.lock().unwrap();
        // filter appears TWICE — ids bound twice
        let mut params: Vec<&dyn ToSql> = vec![&start, &end];
        for _pass in 0..2 {
            let ids_refs: Vec<&dyn ToSql> = ids.iter().map(|v| v as &dyn ToSql).collect();
            params.extend(ids_refs);
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                let touched: i64 = r.get(1)?;
                let closed: i64 = r.get(2)?;
                let attempts: i64 = r.get(3)?;
                let rate = if touched > 0 {
                    Some((closed as f64 / touched as f64) * 100.0)
                } else {
                    None
                };
                Ok(CategoryEfficiencyRow {
                    category: r.get::<_, String>(0)?,
                    touched_in_period: touched,
                    closed_in_period: closed,
                    attempts_in_period: attempts,
                    resolution_rate: rate,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    // ── Dashboard KPI query helpers (A5) ──────────────────────────────────────

    fn setup_fixture_bugs(db: &AppDb) -> (i64, i64, Vec<i64>) {
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let r1 = db
            .insert_local_repository("/tmp/r1", "r1", Some(p1.id), None)
            .unwrap();
        let r2 = db
            .insert_local_repository("/tmp/r2", "r2", Some(p1.id), None)
            .unwrap();

        // Active bug: created in period (Apr 22), open
        let b_open = db
            .insert_bug(
                r1.id,
                1,
                "2026-04-22T10:00:00Z",
                "open bug",
                "critical",
                "ui_ux",
                "created",
                0,
                None,
                None,
            )
            .unwrap();

        // Closed in period: created Apr 20, confirmed Apr 23
        let b_closed = db
            .insert_bug(
                r2.id,
                1,
                "2026-04-20T10:00:00Z",
                "closed bug",
                "major",
                "logic",
                "confirmed",
                2,
                None,
                Some("2026-04-23T12:00:00Z"),
            )
            .unwrap();

        // Closed OUTSIDE period: created Apr 1, confirmed Apr 10
        let b_old = db
            .insert_bug(
                r2.id,
                2,
                "2026-04-01T00:00:00Z",
                "old bug",
                "minor",
                "other",
                "confirmed",
                1,
                None,
                Some("2026-04-10T00:00:00Z"),
            )
            .unwrap();

        (p1.id, r1.id, vec![b_open.id, b_closed.id, b_old.id])
    }

    #[test]
    fn test_closed_in_period_excludes_old_and_open() {
        let db = make_db();
        let (p1, _, _) = setup_fixture_bugs(&db);
        let n = db
            .count_closed_bugs_in_period(Some(&[p1]), "2026-04-21", "2026-04-24")
            .unwrap();
        assert_eq!(n, 1, "only b_closed (confirmed 2026-04-23) fits");
    }

    #[test]
    fn test_opened_in_period_counts_all_created() {
        let db = make_db();
        let (p1, _, _) = setup_fixture_bugs(&db);
        let n = db
            .count_opened_bugs_in_period(Some(&[p1]), "2026-04-21", "2026-04-24")
            .unwrap();
        assert_eq!(n, 1, "only b_open (created 2026-04-22) fits");
    }

    #[test]
    fn test_count_active_bugs_by_severity() {
        let db = make_db();
        let (p1, _, _) = setup_fixture_bugs(&db);
        let total = db.count_active_bugs(Some(&[p1])).unwrap();
        assert_eq!(total, 1);
        let critical = db
            .count_active_bugs_with_severity(Some(&[p1]), "critical")
            .unwrap();
        assert_eq!(critical, 1);
        let major = db
            .count_active_bugs_with_severity(Some(&[p1]), "major")
            .unwrap();
        assert_eq!(major, 0);
    }

    #[test]
    fn test_queries_with_project_ids_none_scope_all_repos() {
        let db = make_db();
        let (_, _, _) = setup_fixture_bugs(&db);
        // None means "all repos", not "no repos"
        let n_closed = db
            .count_closed_bugs_in_period(None, "2026-04-21", "2026-04-24")
            .unwrap();
        assert_eq!(n_closed, 1);
    }

    #[test]
    fn test_attempts_per_closed_avg() {
        let db = make_db();
        let (_, _, _) = setup_fixture_bugs(&db);
        db.backfill_bug_events_for_existing().unwrap();

        // b_closed had fix_attempts=2 and is in period; b_old=1 but outside period.
        // avg for 2026-04-21..24 = 2.0 (only 1 closed bug in window)
        let avg = db
            .avg_attempts_per_closed_in_period(None, "2026-04-21", "2026-04-24")
            .unwrap();
        assert_eq!(avg, Some(2.0));
    }

    #[test]
    fn test_attempts_per_closed_empty_returns_none() {
        let db = make_db();
        let (_, _, _) = setup_fixture_bugs(&db);
        db.backfill_bug_events_for_existing().unwrap();
        // Period with no closed bugs
        let avg = db
            .avg_attempts_per_closed_in_period(None, "2025-01-01", "2025-01-07")
            .unwrap();
        assert_eq!(avg, None, "empty period -> None (UI shows '—')");
    }

    #[test]
    fn test_top_hot_projects_critical_dominates() {
        // T-000115: weighted-score formula (critical×50 + major×15 + active + closed×2 + tasks×1).
        // P1 has 2 critical = score 102. P2 has 1 critical + 2 major = 50+30+3 = 83.
        // Critical dominance preserved at modest major counts.
        let db = make_db();
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let p2 = db.create_project("P2", None, "standard").unwrap();
        let r1 = db
            .insert_local_repository("/tmp/p1r", "p1r", Some(p1.id), None)
            .unwrap();
        let r2 = db
            .insert_local_repository("/tmp/p2r", "p2r", Some(p2.id), None)
            .unwrap();

        // P1: 2 critical → score 2*50 + 2 = 102
        db.insert_bug(r1.id, 1, "2026-04-01T00:00:00Z", "p1 crit1", "critical", "logic", "created", 0, None, None).unwrap();
        db.insert_bug(r1.id, 2, "2026-04-01T00:00:00Z", "p1 crit2", "critical", "logic", "created", 0, None, None).unwrap();
        // P2: 1 critical + 2 major → score 1*50 + 2*15 + 3 = 83
        db.insert_bug(r2.id, 1, "2026-04-01T00:00:00Z", "p2 crit", "critical", "logic", "created", 0, None, None).unwrap();
        db.insert_bug(r2.id, 2, "2026-04-01T00:00:00Z", "p2 maj1", "major", "logic", "created", 0, None, None).unwrap();
        db.insert_bug(r2.id, 3, "2026-04-01T00:00:00Z", "p2 maj2", "major", "logic", "created", 0, None, None).unwrap();

        let top = db.top_hot_projects(None, None, 3).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "P1", "P1 (score 102) ranks above P2 (score 83)");
        assert_eq!(top[0].critical, 2);
        assert_eq!(top[0].bugs_closed, 0);
        assert_eq!(top[0].tasks_done, 0);
        assert_eq!(top[1].name, "P2");
        assert_eq!(top[1].critical, 1);
        assert_eq!(top[1].major, 2);
    }

    #[test]
    fn test_top_hot_excludes_zero_everything() {
        // T-000115: project with closed bugs in period contributes to score
        // (formula change from "active only" to "any signal"). To verify exclusion,
        // pass a period that EXCLUDES the closure date so no signal applies.
        let db = make_db();
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let r = db
            .insert_local_repository("/tmp/r", "r", Some(p1.id), None)
            .unwrap();
        // Confirmed bug closed 2026-04-24 (outside the queried 2027 period)
        db.insert_bug(r.id, 1, "2026-04-20T00:00:00Z", "done", "minor", "other", "confirmed", 1, None, Some("2026-04-24T00:00:00Z")).unwrap();

        // Period that excludes the closure → no active, no closed-in-period, no tasks → score 0
        let top = db.top_hot_projects(None, Some(("2027-01-01", "2027-12-31")), 5).unwrap();
        assert!(top.is_empty(), "project with zero contribution in period must be excluded");
    }

    #[test]
    fn test_top_hot_projects_closed_in_period_contributes() {
        // T-000115: bugs closed within the period contribute weight 2 each to score.
        let db = make_db();
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let r = db
            .insert_local_repository("/tmp/r", "r", Some(p1.id), None)
            .unwrap();
        // 3 bugs closed 2026-04-24 (inside period)
        for i in 1..=3 {
            db.insert_bug(r.id, i, "2026-04-20T00:00:00Z", "d", "minor", "logic", "confirmed", 1, None, Some("2026-04-24T00:00:00Z")).unwrap();
        }

        let top = db.top_hot_projects(None, Some(("2026-04-20", "2026-04-30")), 5).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].bugs_closed, 3, "3 closures inside period");
        assert_eq!(top[0].active, 0);
        // score = 0*50 + 0*15 + 0 + 3*2 + 0 = 6 → qualifies (>0)
    }

    #[test]
    fn test_top_hot_projects_tasks_done_contributes() {
        // T-000115: task_events with event_type='done' inside the period contribute weight 1 each.
        // Verifies task-only projects (no bugs at all) can surface via task activity.
        let db = make_db();
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let r = db
            .insert_local_repository("/tmp/r", "r", Some(p1.id), None)
            .unwrap();
        // 5 tasks completed in period — no bugs at all
        for i in 1..=5 {
            let t = db.insert_task(r.id, &format!("T-{:06}", i), "T", "desc", None, None, None, None, "done", "2026-04-20T00:00:00Z").unwrap();
            db.insert_task_event(t.id, "done", "2026-04-22T10:00:00Z", Some("in-progress"), None).unwrap();
        }

        let top = db.top_hot_projects(None, Some(("2026-04-20", "2026-04-30")), 5).unwrap();
        assert_eq!(top.len(), 1, "task-only project (no bugs) qualifies via task activity");
        assert_eq!(top[0].tasks_done, 5);
        assert_eq!(top[0].active, 0);
        // score = 0 + 0 + 0 + 0 + 5 = 5 → qualifies
    }

    #[test]
    fn test_top_hot_projects_one_critical_dominates_50_tasks() {
        // T-000115: weight ratio verification — 1 critical bug (score 50) edges out
        // 49 tasks done (score 49), but 51 tasks done (score 51) narrowly edges out
        // 1 critical. Documents the crossover point.
        let db = make_db();
        let pa = db.create_project("PA", None, "standard").unwrap();
        let pb = db.create_project("PB", None, "standard").unwrap();
        let ra = db.insert_local_repository("/tmp/pa", "pa", Some(pa.id), None).unwrap();
        let rb = db.insert_local_repository("/tmp/pb", "pb", Some(pb.id), None).unwrap();

        // PA: 1 critical → score 50 + 1 (active) = 51
        db.insert_bug(ra.id, 1, "2026-04-01T00:00:00Z", "crit", "critical", "logic", "created", 0, None, None).unwrap();
        // PB: 49 tasks done → score 49
        for i in 1..=49 {
            let t = db.insert_task(rb.id, &format!("T-{:06}", i), "T", "d", None, None, None, None, "done", "2026-04-20T00:00:00Z").unwrap();
            db.insert_task_event(t.id, "done", "2026-04-22T10:00:00Z", None, None).unwrap();
        }

        let top = db.top_hot_projects(None, Some(("2026-04-20", "2026-04-30")), 5).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "PA", "1 critical (51) outranks 49 tasks (49)");
        assert_eq!(top[1].name, "PB");
    }

    // ── Dashboard flow + efficiency queries (A7) ──────────────────────────────

    #[test]
    fn test_bugs_per_day_returns_opened_and_closed() {
        let db = make_db();
        let (_, _, _) = setup_fixture_bugs(&db);
        let days = db.bugs_per_day(None, "2026-04-20", "2026-04-24").unwrap();
        // 5 days: Apr 20, 21, 22, 23, 24
        assert_eq!(days.len(), 5);

        // Apr 20: b_closed opened on Apr 20 — opened=1, closed=0
        assert_eq!(days[0].date, "2026-04-20");
        assert_eq!(days[0].opened, Some(1));
        assert_eq!(days[0].closed, Some(0));

        // Apr 22: b_open created Apr 22 — opened=1
        assert_eq!(days[2].date, "2026-04-22");
        assert_eq!(days[2].opened, Some(1));

        // Apr 23: b_closed confirmed — closed=1
        assert_eq!(days[3].closed, Some(1));
    }

    #[test]
    fn test_category_efficiency_rows() {
        let db = make_db();
        let (_, _, _) = setup_fixture_bugs(&db);
        db.backfill_bug_events_for_existing().unwrap();

        let rows = db.category_efficiency(None, "2026-04-20", "2026-04-24").unwrap();

        // b_open (critical, ui_ux) created Apr 22 — in period, touched=1, not closed
        let ui = rows.iter().find(|r| r.category == "ui_ux").expect("ui_ux row");
        assert_eq!(ui.touched_in_period, 1);
        assert_eq!(ui.closed_in_period, 0);
        assert_eq!(ui.resolution_rate, Some(0.0));

        // b_closed (major, logic) — created Apr 20, confirmed Apr 23, fix_attempts=2 — in period
        let logic = rows.iter().find(|r| r.category == "logic").expect("logic row");
        assert_eq!(logic.touched_in_period, 1);
        assert_eq!(logic.closed_in_period, 1);
        assert_eq!(logic.attempts_in_period, 2);
        assert_eq!(logic.resolution_rate, Some(100.0));
    }

    // ── v0.17.0: list_repos_with_local_path ───────────────────────────────────

    #[test]
    fn test_list_repos_with_local_path_filters() {
        let db = make_db();
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let p2 = db.create_project("P2", None, "standard").unwrap();
        let _r1 = db
            .insert_local_repository("/tmp/r1", "r1", Some(p1.id), None)
            .unwrap();
        let _r2 = db
            .insert_local_repository("/tmp/r2", "r2", Some(p2.id), None)
            .unwrap();
        // unassigned repo with local_path
        let _r3 = db
            .insert_local_repository("/tmp/r3", "r3", None, None)
            .unwrap();
        // a repo with NO local path — should be excluded by helper
        db.upsert_repository("owner/repo-without-local", None, None, None, None, None)
            .unwrap();

        let all = db.list_repos_with_local_path(None).unwrap();
        assert_eq!(all.len(), 3);

        let only_p1 = db.list_repos_with_local_path(Some(&[p1.id])).unwrap();
        assert_eq!(only_p1.len(), 1);
        assert!(only_p1[0].local_path.as_deref() == Some("/tmp/r1"));
    }

    #[test]
    fn test_top_hot_repos_in_project_basic_ordering() {
        let db = make_db();
        let p = db.create_project("proj", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/tmp/r1", "r1", Some(p.id), Some("server")).unwrap();
        let r2 = db.insert_local_repository("/tmp/r2", "r2", Some(p.id), Some("client")).unwrap();
        let r3 = db.insert_local_repository("/tmp/r3", "r3", Some(p.id), Some("tool")).unwrap();
        // r1: 0 critical, 1 major, 1 active → score 0*50 + 1*15 + 1 = 16
        db.insert_bug(r1.id, 1, "2026-01-01T00:00:00Z", "d1", "major", "logic", "created", 0, None, None).unwrap();
        // r2: 2 critical, 0 major, 2 active → score 2*50 + 0*15 + 2 = 102
        db.insert_bug(r2.id, 1, "2026-01-01T00:00:00Z", "d2", "critical", "logic", "created", 0, None, None).unwrap();
        db.insert_bug(r2.id, 2, "2026-01-01T00:00:00Z", "d3", "critical", "ui_ux", "in-progress", 0, None, None).unwrap();
        // r3: 0 critical, 0 major, 3 active (medium severity) → score 0 + 0 + 3 = 3
        db.insert_bug(r3.id, 1, "2026-01-01T00:00:00Z", "d4", "medium", "logic", "created", 0, None, None).unwrap();
        db.insert_bug(r3.id, 2, "2026-01-01T00:00:00Z", "d5", "medium", "logic", "created", 0, None, None).unwrap();
        db.insert_bug(r3.id, 3, "2026-01-01T00:00:00Z", "d6", "medium", "logic", "testing", 0, None, None).unwrap();

        let hot = db.top_hot_repos_in_project(p.id, None, 3).unwrap();
        assert_eq!(hot.len(), 3);
        // r2 (102) > r1 (16) > r3 (3)
        assert_eq!(hot[0].repo_id, r2.id);
        assert_eq!(hot[0].critical, 2);
        assert_eq!(hot[0].active, 2);
        assert_eq!(hot[1].repo_id, r1.id);
        assert_eq!(hot[1].major, 1);
        assert_eq!(hot[2].repo_id, r3.id);
        assert_eq!(hot[2].active, 3);
    }

    #[test]
    fn test_top_hot_repos_in_project_severity_counts_exclude_confirmed() {
        // T-000115: confirmed bugs don't contribute to critical/major/active counts,
        // but DO contribute to bugs_closed (in lifetime mode) — separate accumulators.
        let db = make_db();
        let p = db.create_project("proj", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/tmp/r1", "r1", Some(p.id), Some("server")).unwrap();
        db.insert_bug(r1.id, 1, "2026-01-01T00:00:00Z", "d1", "critical", "logic", "confirmed", 1, None, Some("2026-01-02T00:00:00Z")).unwrap();
        db.insert_bug(r1.id, 2, "2026-01-01T00:00:00Z", "d2", "minor", "logic", "created", 0, None, None).unwrap();

        let hot = db.top_hot_repos_in_project(p.id, None, 5).unwrap();
        assert_eq!(hot.len(), 1);
        assert_eq!(hot[0].critical, 0, "confirmed critical doesn't count in severity");
        assert_eq!(hot[0].active, 1, "only the created minor counts as active");
        assert_eq!(hot[0].bugs_closed, 1, "the confirmed critical counts as closed (lifetime mode)");
    }

    #[test]
    fn test_top_hot_repos_in_project_period_excludes_old_closures() {
        // T-000115: period-filtered mode — closures outside the window don't contribute.
        // Verifies repo with only out-of-period activity gets filtered.
        let db = make_db();
        let p = db.create_project("proj", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/tmp/r1", "r1", Some(p.id), Some("server")).unwrap();
        let r2 = db.insert_local_repository("/tmp/r2", "r2", Some(p.id), Some("client")).unwrap();
        // r1: confirmed Jan 2 (OUTSIDE 2027 query period) → 0 contribution in period
        db.insert_bug(r1.id, 1, "2026-01-01T00:00:00Z", "d1", "minor", "logic", "confirmed", 1, None, Some("2026-01-02T00:00:00Z")).unwrap();
        // r2: 1 active minor → score 1 in any window
        db.insert_bug(r2.id, 1, "2026-01-01T00:00:00Z", "d2", "minor", "logic", "created", 0, None, None).unwrap();

        let hot = db.top_hot_repos_in_project(p.id, Some(("2027-01-01", "2027-12-31")), 5).unwrap();
        assert_eq!(hot.len(), 1, "r1 (closure out of period) excluded; r2 (active bug) included");
        assert_eq!(hot[0].repo_id, r2.id);
    }

    #[test]
    fn test_top_hot_repos_in_project_lifetime_mode_includes_all() {
        // T-000115: lifetime mode (period=None) — closed bugs of any age contribute.
        // Stats tab default behavior.
        let db = make_db();
        let p = db.create_project("proj", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/tmp/r1", "r1", Some(p.id), Some("server")).unwrap();
        // Only confirmed bugs, all old (2026) — in lifetime mode they contribute via bugs_closed
        db.insert_bug(r1.id, 1, "2026-01-01T00:00:00Z", "d1", "minor", "logic", "confirmed", 1, None, Some("2026-01-02T00:00:00Z")).unwrap();
        db.insert_bug(r1.id, 2, "2026-01-01T00:00:00Z", "d2", "minor", "logic", "confirmed", 1, None, Some("2026-01-05T00:00:00Z")).unwrap();

        let hot = db.top_hot_repos_in_project(p.id, None, 5).unwrap();
        assert_eq!(hot.len(), 1, "lifetime mode includes repo with historical closed bugs");
        assert_eq!(hot[0].bugs_closed, 2);
        assert_eq!(hot[0].active, 0);
        // score = 0+0+0+2*2+0 = 4 → qualifies
    }
}
