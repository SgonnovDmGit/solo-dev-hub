// T-000094: Timeline UNION ALL across 5 event sources.
// Moved from db.rs.

use super::*;

impl AppDb {
    // ── v0.20.0: Timeline UNION ALL across 5 event sources ────────────────────

    pub fn read_timeline_filtered(
        &self,
        filter: &crate::models::TimelineFilter,
        offset: u32,
        limit: u32,
    ) -> SqlResult<Vec<crate::models::ActivityEvent>> {
        // H3 review-fix: push kind/repo/project filters into SQL `WHERE`
        // before LIMIT/OFFSET. Previously these filters ran in Rust after
        // fetching `limit` rows — when most rows got filtered out the
        // caller saw `r.length < PAGE_SIZE` and assumed "no more events"
        // even though plenty matched on later pages. `search` stays in
        // Rust (substring match is cheap and the SQL form would be ugly
        // across kinds).
        // Build the dynamic WHERE additions from validated filters. Both
        // i64 IDs (numeric) and the event-kind enum are safe to inline.
        const VALID_KINDS: &[&str] = &[
            "bug_event", "task_event", "sync_event", "deploy_event", "repo_rename",
        ];
        let mut where_extra = String::new();
        if let Some(ref kinds) = filter.event_kinds {
            let filtered: Vec<&str> = kinds.iter()
                .filter_map(|k| VALID_KINDS.iter().find(|v| **v == k.as_str()).copied())
                .collect();
            if !filtered.is_empty() {
                let list = filtered.iter().map(|k| format!("'{}'", k)).collect::<Vec<_>>().join(", ");
                where_extra.push_str(&format!(" AND kind IN ({})", list));
            }
        }
        if let Some(ref repos) = filter.repo_ids {
            if !repos.is_empty() {
                let list = repos.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(", ");
                where_extra.push_str(&format!(" AND repo_id IN ({})", list));
            }
        }
        if let Some(ref projs) = filter.project_ids {
            if !projs.is_empty() {
                let list = projs.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                where_extra.push_str(&format!(" AND project_id IN ({})", list));
            }
        }

        // T-000103 Task 6: every UNION leg now projects a 14th `details` column
        // so sync_event rows can carry their `details` JSON to the frontend
        // (currently only `sync_type='migration'` populates this; other legs
        // emit NULL).
        let sql_body = r#"
            SELECT * FROM (
              SELECT
                'bug_event' AS kind,
                be.event_type AS event_type,
                be.ts AS ts,
                b.repository_id AS repo_id,
                CASE WHEN r.github_name IS NOT NULL THEN
                  CASE WHEN instr(r.github_name, '/') > 0
                       THEN substr(r.github_name, instr(r.github_name, '/') + 1)
                       ELSE r.github_name END
                ELSE r.description END AS repo_display_name,
                b.display_id AS bug_display_id,
                NULL AS task_display_id,
                NULL AS old_canonical, NULL AS new_canonical,
                NULL AS sync_type, NULL AS deploy_action, NULL AS deploy_env_name,
                NULL AS change_count,
                NULL AS details,
                r.project_id AS project_id
              FROM bug_events be
              JOIN bugs b ON b.id = be.bug_id
              LEFT JOIN repositories r ON r.id = b.repository_id

              UNION ALL

              SELECT
                'repo_rename', 'renamed', rr.renamed_at, rr.repository_id,
                CASE WHEN r.github_name IS NOT NULL THEN
                  CASE WHEN instr(r.github_name, '/') > 0
                       THEN substr(r.github_name, instr(r.github_name, '/') + 1)
                       ELSE r.github_name END
                ELSE r.description END,
                NULL, NULL,
                rr.old_canonical, rr.new_canonical,
                NULL, NULL, NULL, NULL,
                NULL,
                r.project_id
              FROM repo_renames rr
              LEFT JOIN repositories r ON r.id = rr.repository_id

              UNION ALL

              SELECT
                'task_event', te.event_type, te.ts, t.repository_id,
                CASE WHEN r.github_name IS NOT NULL THEN
                  CASE WHEN instr(r.github_name, '/') > 0
                       THEN substr(r.github_name, instr(r.github_name, '/') + 1)
                       ELSE r.github_name END
                ELSE r.description END,
                NULL, t.task_id,
                NULL, NULL,
                NULL, NULL, NULL, NULL,
                NULL,
                r.project_id
              FROM task_events te
              JOIN tasks t ON t.id = te.task_id
              LEFT JOIN repositories r ON r.id = t.repository_id

              UNION ALL

              SELECT
                'sync_event', se.sync_type, se.ts, se.repository_id,
                CASE WHEN r.github_name IS NOT NULL THEN
                  CASE WHEN instr(r.github_name, '/') > 0
                       THEN substr(r.github_name, instr(r.github_name, '/') + 1)
                       ELSE r.github_name END
                ELSE r.description END,
                NULL, NULL,
                NULL, NULL,
                se.sync_type, NULL, NULL, se.change_count,
                se.details,
                r.project_id
              FROM sync_events se
              LEFT JOIN repositories r ON r.id = se.repository_id

              UNION ALL

              SELECT
                'deploy_event', de.action, de.ts, de.repository_id,
                CASE WHEN r.github_name IS NOT NULL THEN
                  CASE WHEN instr(r.github_name, '/') > 0
                       THEN substr(r.github_name, instr(r.github_name, '/') + 1)
                       ELSE r.github_name END
                ELSE r.description END,
                NULL, NULL,
                NULL, NULL,
                NULL, de.action, e.name, NULL,
                NULL,
                r.project_id
              FROM deploy_events de
              LEFT JOIN repositories r ON r.id = de.repository_id
              LEFT JOIN deploy_environments e ON e.id = de.deploy_env_id
            )
            WHERE date(ts) BETWEEN ?1 AND ?2
        "#;
        let sql = format!(
            "{}{} ORDER BY ts DESC LIMIT ?3 OFFSET ?4",
            sql_body, where_extra
        );

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![&filter.start_date, &filter.end_date, limit, offset], |row| {
            Ok((
                row.get::<_, String>(0)?,            // kind
                row.get::<_, String>(1)?,            // event_type
                row.get::<_, String>(2)?,            // ts
                row.get::<_, Option<i64>>(3)?,       // repo_id
                row.get::<_, Option<String>>(4)?,    // repo_display_name
                row.get::<_, Option<String>>(5)?,    // bug_display_id
                row.get::<_, Option<String>>(6)?,    // task_display_id
                row.get::<_, Option<String>>(7)?,    // old_canonical
                row.get::<_, Option<String>>(8)?,    // new_canonical
                row.get::<_, Option<String>>(9)?,    // sync_type
                row.get::<_, Option<String>>(10)?,   // deploy_action
                row.get::<_, Option<String>>(11)?,   // deploy_env_name
                row.get::<_, Option<i64>>(12)?,      // change_count
                row.get::<_, Option<String>>(13)?,   // details (T-000103 Task 6)
                row.get::<_, Option<i64>>(14)?,      // project_id
            ))
        })?;

        let mut out: Vec<crate::models::ActivityEvent> = Vec::new();
        for row in rows {
            let (kind, event_type, ts, repo_id, repo_name, bug_id, task_id, old_c, new_c, sync_t, deploy_a, deploy_e, change_c, details_v, _project_id) = row?;

            // kind / repo_ids / project_ids are now filtered in SQL (above) —
            // only `search` (substring match) stays in Rust.
            if let Some(ref s) = filter.search {
                if !s.is_empty() {
                    let q = s.to_lowercase();
                    let haystack = format!("{} {} {}",
                        bug_id.as_deref().unwrap_or(""),
                        task_id.as_deref().unwrap_or(""),
                        repo_name.as_deref().unwrap_or("")).to_lowercase();
                    if !haystack.contains(&q) { continue; }
                }
            }

            out.push(crate::models::ActivityEvent {
                kind, event_type, ts,
                repo_id,
                repo_display_name: repo_name,
                bug_display_id: bug_id,
                task_display_id: task_id,
                old_canonical: old_c,
                new_canonical: new_c,
                sync_type: sync_t,
                deploy_action: deploy_a,
                deploy_env_name: deploy_e,
                change_count: change_c,
                details: details_v,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db() -> AppDb {
        AppDb::new(std::path::PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn test_recent_activity_orders_bug_events_and_renames_desc() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        let bug = db
            .insert_bug(
                repo.id, 1, "2026-04-20T00:00:00Z", "desc1", "minor", "other",
                "created", 0, None, None,
            )
            .unwrap();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                 VALUES (?1, 'created', '2026-04-21T10:00:00Z', NULL, 'created')",
                [bug.id],
            ).unwrap();
            conn.execute(
                "INSERT INTO repo_renames (repository_id, old_canonical, new_canonical, renamed_at)
                 VALUES (?1, 'old_name', 'new_name', '2026-04-22T15:00:00Z')",
                [repo.id],
            ).unwrap();
            conn.execute(
                "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                 VALUES (?1, 'confirmed', '2026-04-23T08:00:00Z', 'testing', 'confirmed')",
                [bug.id],
            ).unwrap();
        }

        let activity = db.recent_activity(10).unwrap();

        assert_eq!(activity.len(), 3, "3 events expected");
        assert_eq!(activity[0].event_type, "confirmed");
        assert_eq!(activity[0].kind, "bug_event");
        assert_eq!(activity[0].bug_display_id.as_deref(), Some("B-000001"));
        assert_eq!(activity[1].event_type, "renamed");
        assert_eq!(activity[1].kind, "repo_rename");
        assert_eq!(activity[1].old_canonical.as_deref(), Some("old_name"));
        assert_eq!(activity[2].event_type, "created");
        assert_eq!(activity[2].kind, "bug_event");
    }

    #[test]
    fn test_recent_activity_respects_limit() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        let bug = db
            .insert_bug(repo.id, 1, "2026-04-20T00:00:00Z", "d", "minor", "other", "created", 0, None, None)
            .unwrap();

        {
            let conn = db.conn.lock().unwrap();
            for i in 0..15 {
                conn.execute(
                    "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                     VALUES (?1, 'created', ?2, NULL, 'created')",
                    rusqlite::params![bug.id, format!("2026-04-{:02}T00:00:00Z", 10 + i)],
                ).unwrap();
            }
        }
        let activity = db.recent_activity(10).unwrap();
        assert_eq!(activity.len(), 10);
    }

    #[test]
    fn test_recent_activity_empty_db_returns_empty_vec() {
        let db = make_db();
        let activity = db.recent_activity(10).unwrap();
        assert!(activity.is_empty());
    }

    #[test]
    fn test_recent_activity_includes_repo_display_name_from_github_name() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "ignored_local_name", None, None).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE repositories SET github_name = 'owner/myrepo',
                                         github_url = 'https://github.com/owner/myrepo'
                 WHERE id = ?1",
                [repo.id],
            ).unwrap();
        }

        let bug = db.insert_bug(repo.id, 1, "2026-04-20T00:00:00Z", "d", "minor", "other", "created", 0, None, None).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                 VALUES (?1, 'created', '2026-04-21T00:00:00Z', NULL, 'created')",
                [bug.id],
            ).unwrap();
        }
        let activity = db.recent_activity(10).unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].repo_display_name.as_deref(), Some("myrepo"));
    }

    #[test]
    fn test_recent_activity_local_only_repo_uses_description() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "my_local_repo", None, None).unwrap();
        let bug = db.insert_bug(repo.id, 1, "2026-04-20T00:00:00Z", "d", "minor", "other", "created", 0, None, None).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                 VALUES (?1, 'created', '2026-04-21T00:00:00Z', NULL, 'created')",
                [bug.id],
            ).unwrap();
        }
        let activity = db.recent_activity(10).unwrap();
        assert_eq!(activity[0].repo_display_name.as_deref(), Some("my_local_repo"));
    }

    #[test]
    fn test_read_timeline_filters_by_date_range() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        let bug = db.insert_bug(repo.id, 1, "2026-04-20T00:00:00Z", "d", "minor", "other", "created", 0, None, None).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute("INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status) VALUES (?1, 'created', '2026-04-15T00:00:00Z', NULL, 'created')", [bug.id]).unwrap();
            conn.execute("INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status) VALUES (?1, 'confirmed', '2026-04-25T00:00:00Z', 'testing', 'confirmed')", [bug.id]).unwrap();
        }
        let filter = crate::models::TimelineFilter {
            start_date: "2026-04-20".into(),
            end_date: "2026-04-30".into(),
            event_kinds: None,
            project_ids: None,
            repo_ids: None,
            search: None,
        };
        let events = db.read_timeline_filtered(&filter, 0, 50).unwrap();
        assert_eq!(events.len(), 1, "only the 2026-04-25 confirmed event in range");
        assert_eq!(events[0].event_type, "confirmed");
    }

    #[test]
    fn test_read_timeline_pagination() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        let bug = db.insert_bug(repo.id, 1, "2026-04-20T00:00:00Z", "d", "minor", "other", "created", 0, None, None).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for i in 0..15 {
                conn.execute(
                    "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status) VALUES (?1, 'created', ?2, NULL, 'created')",
                    rusqlite::params![bug.id, format!("2026-04-{:02}T00:00:00Z", 10 + i)],
                ).unwrap();
            }
        }
        let filter = crate::models::TimelineFilter {
            start_date: "2026-04-01".into(),
            end_date: "2026-04-30".into(),
            event_kinds: None,
            project_ids: None,
            repo_ids: None,
            search: None,
        };
        let page1 = db.read_timeline_filtered(&filter, 0, 10).unwrap();
        let page2 = db.read_timeline_filtered(&filter, 10, 10).unwrap();
        assert_eq!(page1.len(), 10);
        assert_eq!(page2.len(), 5);
    }

    #[test]
    fn test_recent_activity_includes_new_event_sources() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        let bug = db.insert_bug(repo.id, 1, "2026-04-20T00:00:00Z", "d", "minor", "other", "created", 0, None, None).unwrap();

        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO bug_events (bug_id, event_type, ts, from_status, to_status)
                 VALUES (?1, 'created', '2026-04-21T00:00:00Z', NULL, 'created')",
                [bug.id],
            ).unwrap();
        }
        db.insert_sync_event(Some(repo.id), "tasks", "2026-04-22T00:00:00Z", 3, None).unwrap();
        db.insert_deploy_event(None, repo.id, "render", "2026-04-23T00:00:00Z", None).unwrap();

        let task = db.insert_task(repo.id, "T-001", "T", "Task A", Some(2.0), Some("high"), Some("open"), None, "todo", "2026-04-24").unwrap();
        db.insert_task_event(task.id, "taken", "2026-04-25T00:00:00Z", Some("open"), Some("in-progress")).unwrap();

        let activity = db.recent_activity(10).unwrap();
        let kinds: std::collections::HashSet<String> = activity.iter().map(|e| e.kind.clone()).collect();
        assert!(kinds.contains("bug_event"));
        assert!(kinds.contains("sync_event"));
        assert!(kinds.contains("deploy_event"));
        assert!(kinds.contains("task_event"));
        assert_eq!(activity.len(), 4);
        assert_eq!(activity[0].kind, "task_event");
    }
}
