// T-000094: tasks CRUD + task_events + sync_events + deploy_events.
// Plus the small `recent_activity` wrapper that delegates to timeline.
// Moved from db.rs.

use super::*;

impl AppDb {
    /// v0.20.0: Recent activity feed for Dashboard.
    /// Delegates to `read_timeline_filtered` with a wide date window so that
    /// all 5 event sources (bug_events, repo_renames, task_events, sync_events,
    /// deploy_events) are included. Eliminates SQL duplication (D-12 spec).
    pub fn recent_activity(&self, limit: u32) -> SqlResult<Vec<crate::models::ActivityEvent>> {
        let filter = crate::models::TimelineFilter {
            start_date: "1970-01-01".into(),
            end_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            event_kinds: None,
            project_ids: None,
            repo_ids: None,
            search: None,
        };
        self.read_timeline_filtered(&filter, 0, limit)
    }

    // ── Tasks ─────────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub fn insert_task(
        &self,
        repository_id: i64,
        task_id: &str,
        prefix: &str,
        description: &str,
        effort: Option<f64>,
        priority: Option<&str>,
        status: Option<&str>,
        version: Option<&str>,
        source: &str,
        created_at: &str,
    ) -> SqlResult<crate::models::Task> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tasks (repository_id, task_id, prefix, description, effort, priority, status, version, source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                repository_id, task_id, prefix, description, effort, priority, status, version, source, created_at, now
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(crate::models::Task {
            id,
            repository_id,
            task_id: task_id.to_string(),
            prefix: prefix.to_string(),
            description: description.to_string(),
            effort,
            priority: priority.map(String::from),
            status: status.map(String::from),
            version: version.map(String::from),
            source: source.to_string(),
            created_at: created_at.to_string(),
            updated_at: now,
        })
    }

    pub fn update_task_status(&self, task_id: i64, new_status: Option<&str>) -> SqlResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![new_status, now, task_id],
        )?;
        Ok(())
    }

    pub fn update_task_source(&self, task_id: i64, new_source: &str) -> SqlResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET source = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![new_source, now, task_id],
        )?;
        Ok(())
    }

    /// T-000109: write a new value into `tasks.version`. Called by sync when a
    /// todo.md row's inherited section-header version changes (e.g. the user
    /// moves the task between `## v0.32.0` and `## v1.0.0` sections), and
    /// on the todo→done flip to capture the release the task shipped in.
    pub fn update_task_version(&self, task_id: i64, new_version: Option<&str>) -> SqlResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tasks SET version = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![new_version, now, task_id],
        )?;
        Ok(())
    }

    pub fn delete_task(&self, id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tasks WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    pub fn list_tasks_by_repo(
        &self,
        repository_id: i64,
        source: &str,
    ) -> SqlResult<Vec<crate::models::Task>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, task_id, prefix, description, effort, priority, status, version, source, created_at, updated_at
             FROM tasks WHERE repository_id = ?1 AND source = ?2 ORDER BY task_id",
        )?;
        let rows = stmt.query_map(rusqlite::params![repository_id, source], |r| {
            Ok(crate::models::Task {
                id: r.get(0)?,
                repository_id: r.get(1)?,
                task_id: r.get(2)?,
                prefix: r.get(3)?,
                description: r.get(4)?,
                effort: r.get(5)?,
                priority: r.get(6)?,
                status: r.get(7)?,
                version: r.get(8)?,
                source: r.get(9)?,
                created_at: r.get(10)?,
                updated_at: r.get(11)?,
            })
        })?;
        rows.collect()
    }

    pub fn insert_task_event(
        &self,
        task_id: i64,
        event_type: &str,
        ts: &str,
        from_status: Option<&str>,
        to_status: Option<&str>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO task_events (task_id, event_type, ts, from_status, to_status)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![task_id, event_type, ts, from_status, to_status],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn list_task_events_by_task(
        &self,
        task_id: i64,
    ) -> SqlResult<Vec<crate::models::TaskEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, event_type, ts, from_status, to_status
             FROM task_events WHERE task_id = ?1 ORDER BY ts ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![task_id], |r| {
            Ok(crate::models::TaskEvent {
                id: r.get(0)?,
                task_id: r.get(1)?,
                event_type: r.get(2)?,
                ts: r.get(3)?,
                from_status: r.get(4)?,
                to_status: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn mark_tasks_migrated(&self, repository_id: i64, ts: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET tasks_migrated_at = ?1 WHERE id = ?2",
            rusqlite::params![ts, repository_id],
        )?;
        Ok(())
    }

    pub fn get_tasks_migrated_at(&self, repository_id: i64) -> SqlResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT tasks_migrated_at FROM repositories WHERE id = ?1",
            rusqlite::params![repository_id],
            |r| r.get::<_, Option<String>>(0),
        )
    }

    // ── Sync events ───────────────────────────────────────────────────────────

    pub fn insert_sync_event(
        &self,
        repository_id: Option<i64>,
        sync_type: &str,
        ts: &str,
        change_count: i64,
        details: Option<&str>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sync_events (repository_id, sync_type, ts, change_count, details)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![repository_id, sync_type, ts, change_count, details],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn list_sync_events(
        &self,
        limit: u32,
        offset: u32,
    ) -> SqlResult<Vec<crate::models::SyncEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, sync_type, ts, change_count, details
             FROM sync_events ORDER BY ts DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit, offset], |r| {
            Ok(crate::models::SyncEvent {
                id: r.get(0)?,
                repository_id: r.get(1)?,
                sync_type: r.get(2)?,
                ts: r.get(3)?,
                change_count: r.get(4)?,
                details: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    // ── Deploy events ─────────────────────────────────────────────────────────

    pub fn insert_deploy_event(
        &self,
        deploy_env_id: Option<i64>,
        repository_id: i64,
        action: &str,
        ts: &str,
        details: Option<&str>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO deploy_events (deploy_env_id, repository_id, action, ts, details)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![deploy_env_id, repository_id, action, ts, details],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub fn list_deploy_events(
        &self,
        limit: u32,
        offset: u32,
    ) -> SqlResult<Vec<crate::models::DeployEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, deploy_env_id, repository_id, action, ts, details
             FROM deploy_events ORDER BY ts DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit, offset], |r| {
            Ok(crate::models::DeployEvent {
                id: r.get(0)?,
                deploy_env_id: r.get(1)?,
                repository_id: r.get(2)?,
                action: r.get(3)?,
                ts: r.get(4)?,
                details: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    // ── Secret-push audit (v1.8.0, T-000135) ───────────────────────────────────

    /// Unified read over already-logged GitHub Actions secret pushes.
    /// Repo-level events live in `sync_events` (sync_type='secret', verb inside
    /// `details.action`); env-level events live in `deploy_events`
    /// (`action IN ('env_secret_set','env_secret_delete')`, name inside
    /// `details.name`). Both `details` blobs are `serde_json::json!`-produced, so
    /// they're parsed with serde_json (portable + unit-testable), not SQL
    /// json_extract. Rows with malformed/absent `details` are skipped. Combined,
    /// sorted by `ts` DESC, then paginated in Rust (secret events are low-volume).
    pub fn list_secret_push_events(
        &self,
        limit: u32,
        offset: u32,
    ) -> SqlResult<Vec<crate::models::SecretPushEvent>> {
        // Small local shape for parsing the JSON `details` blobs.
        #[derive(serde::Deserialize)]
        struct Details {
            action: Option<String>,
            name: Option<String>,
        }

        // Mirror Repository::display_name() exactly (same as list_deploy_report).
        fn display_name(github_name: Option<String>, description: Option<String>) -> String {
            match github_name {
                Some(gh) => gh.rsplit('/').next().unwrap_or("").to_string(),
                None => description.unwrap_or_else(|| "<local>".to_string()),
            }
        }

        let conn = self.conn.lock().unwrap();
        let mut out: Vec<crate::models::SecretPushEvent> = Vec::new();

        // Query A — repo-level secret events (sync_events).
        {
            let mut stmt = conn.prepare(
                "SELECT se.repository_id, r.github_name, r.description, se.details, se.ts \
                 FROM sync_events se \
                 JOIN repositories r ON r.id = se.repository_id \
                 WHERE se.sync_type = 'secret'",
            )?;
            let rows = stmt.query_map([], |row| {
                let repository_id: i64 = row.get(0)?;
                let github_name: Option<String> = row.get(1)?;
                let description: Option<String> = row.get(2)?;
                let details: Option<String> = row.get(3)?;
                let ts: String = row.get(4)?;
                Ok((repository_id, github_name, description, details, ts))
            })?;
            for row in rows {
                let (repository_id, github_name, description, details, ts) = row?;
                let parsed: Option<Details> = details
                    .as_deref()
                    .and_then(|d| serde_json::from_str(d).ok());
                let Some(Details {
                    action: Some(action),
                    name: Some(secret_name),
                }) = parsed
                else {
                    continue; // malformed / missing action|name → skip
                };
                out.push(crate::models::SecretPushEvent {
                    source: "repo".to_string(),
                    repository_id,
                    repo_name: display_name(github_name, description),
                    deploy_env_id: None,
                    env_name: None,
                    secret_name,
                    action,
                    ts,
                });
            }
        }

        // Query B — env-level secret events (deploy_events).
        {
            let mut stmt = conn.prepare(
                "SELECT de.repository_id, r.github_name, r.description, de.deploy_env_id, \
                        env.name, de.action, de.details, de.ts \
                 FROM deploy_events de \
                 JOIN repositories r ON r.id = de.repository_id \
                 LEFT JOIN deploy_environments env ON env.id = de.deploy_env_id \
                 WHERE de.action IN ('env_secret_set', 'env_secret_delete')",
            )?;
            let rows = stmt.query_map([], |row| {
                let repository_id: i64 = row.get(0)?;
                let github_name: Option<String> = row.get(1)?;
                let description: Option<String> = row.get(2)?;
                let deploy_env_id: Option<i64> = row.get(3)?;
                let env_name: Option<String> = row.get(4)?;
                let action: String = row.get(5)?;
                let details: Option<String> = row.get(6)?;
                let ts: String = row.get(7)?;
                Ok((
                    repository_id,
                    github_name,
                    description,
                    deploy_env_id,
                    env_name,
                    action,
                    details,
                    ts,
                ))
            })?;
            for row in rows {
                let (
                    repository_id,
                    github_name,
                    description,
                    deploy_env_id,
                    env_name,
                    action_col,
                    details,
                    ts,
                ) = row?;
                let parsed: Option<Details> = details
                    .as_deref()
                    .and_then(|d| serde_json::from_str(d).ok());
                let Some(Details {
                    name: Some(secret_name),
                    ..
                }) = parsed
                else {
                    continue; // malformed / missing name → skip
                };
                let action = match action_col.as_str() {
                    "env_secret_set" => "set",
                    "env_secret_delete" => "delete",
                    _ => continue, // unreachable given WHERE, but stay defensive
                }
                .to_string();
                out.push(crate::models::SecretPushEvent {
                    source: "env".to_string(),
                    repository_id,
                    repo_name: display_name(github_name, description),
                    deploy_env_id,
                    env_name,
                    secret_name,
                    action,
                    ts,
                });
            }
        }

        // Sort by ts DESC, then paginate in Rust.
        out.sort_by(|a, b| b.ts.cmp(&a.ts));
        let paged = out
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        Ok(paged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db() -> AppDb {
        AppDb::new(std::path::PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn test_insert_task_returns_row_with_id() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        let task = db
            .insert_task(
                repo.id,
                "T-042",
                "T",
                "Some task",
                Some(4.0),
                Some("high"),
                Some("open"),
                None,
                "todo",
                "2026-04-26",
            )
            .unwrap();
        assert_eq!(task.task_id, "T-042");
        assert_eq!(task.prefix, "T");
        assert_eq!(task.priority.as_deref(), Some("high"));
    }

    #[test]
    fn test_list_tasks_by_repo_filters_source() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        db.insert_task(
            repo.id,
            "T-001",
            "T",
            "Open task",
            Some(2.0),
            Some("medium"),
            Some("open"),
            None,
            "todo",
            "2026-04-20",
        )
        .unwrap();
        db.insert_task(
            repo.id,
            "T-002",
            "T",
            "Done task",
            None,
            None,
            None,
            Some("v0.20.0"),
            "done",
            "2026-04-19",
        )
        .unwrap();
        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        let dones = db.list_tasks_by_repo(repo.id, "done").unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(dones.len(), 1);
        assert_eq!(todos[0].task_id, "T-001");
        assert_eq!(dones[0].task_id, "T-002");
    }

    #[test]
    fn test_insert_task_event_links_to_task() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        let task = db
            .insert_task(
                repo.id,
                "T-001",
                "T",
                "Test",
                Some(1.0),
                Some("low"),
                Some("open"),
                None,
                "todo",
                "2026-04-26",
            )
            .unwrap();
        db.insert_task_event(
            task.id,
            "created",
            "2026-04-26T00:00:00Z",
            None,
            Some("open"),
        )
        .unwrap();
        let events = db.list_task_events_by_task(task.id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "created");
    }

    #[test]
    fn test_mark_tasks_migrated_sets_timestamp() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        db.mark_tasks_migrated(repo.id, "2026-04-26T12:00:00Z")
            .unwrap();
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());
    }

    #[test]
    fn test_get_tasks_migrated_at_null_when_unset() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_none());
    }

    #[test]
    fn test_insert_sync_event_with_repo_id() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        db.insert_sync_event(
            Some(repo.id),
            "project_sync",
            "2026-04-26T10:00:00Z",
            3,
            None,
        )
        .unwrap();
        let events = db.list_sync_events(10, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sync_type, "project_sync");
        assert_eq!(events[0].change_count, 3);
    }

    #[test]
    fn test_insert_sync_event_portfolio_wide_null_repo() {
        let db = make_db();
        db.insert_sync_event(None, "tasks", "2026-04-26T10:00:00Z", 0, None)
            .unwrap();
        let events = db.list_sync_events(10, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].repository_id.is_none());
    }

    #[test]
    fn test_insert_deploy_event_with_details_json() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        db.insert_deploy_event(
            None,
            repo.id,
            "render",
            "2026-04-26T10:00:00Z",
            Some(r#"{"env":"prod"}"#),
        )
        .unwrap();
        let events = db.list_deploy_events(10, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "render");
        assert_eq!(events[0].details.as_deref(), Some(r#"{"env":"prod"}"#));
    }

    // ── Secret-push audit (v1.8.0, T-000135) ───────────────────────────────────

    /// Seed a repo with a real `github_name` (so repo_name derives from its last
    /// segment) and one deploy environment. Returns (repository_id, deploy_env_id).
    fn seed_repo_and_env(db: &AppDb) -> (i64, i64) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO repositories (github_name, github_url, project_id, role, description, local_path, sort_order)
             VALUES ('owner/audit-repo', NULL, NULL, 'server', 'Audit repo', '/tmp/audit', 10)",
            [],
        )
        .unwrap();
        let repo_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO deploy_environments
                (repository_id, name, workflow_name, image_tag, compose_service, domain, deploy_branch)
             VALUES (?1, 'prod', 'Deploy', 'latest', 'backend', 'x.com', 'master')",
            rusqlite::params![repo_id],
        )
        .unwrap();
        let env_id = conn.last_insert_rowid();
        (repo_id, env_id)
    }

    #[test]
    fn test_list_secret_push_events_normalizes_and_sorts() {
        let db = make_db();
        let (repo_id, env_id) = seed_repo_and_env(&db);

        // Repo secret 'set' (verb inside details.action).
        db.insert_sync_event(
            Some(repo_id),
            "secret",
            "2026-07-01T10:00:00Z",
            1,
            Some(&serde_json::json!({ "action": "set", "name": "API_KEY" }).to_string()),
        )
        .unwrap();
        // Repo secret 'delete'.
        db.insert_sync_event(
            Some(repo_id),
            "secret",
            "2026-07-01T11:00:00Z",
            1,
            Some(&serde_json::json!({ "action": "delete", "name": "OLD_KEY" }).to_string()),
        )
        .unwrap();
        // Env secret set (verb inside the action column; name-only details).
        db.insert_deploy_event(
            Some(env_id),
            repo_id,
            "env_secret_set",
            "2026-07-01T12:00:00Z",
            Some(&serde_json::json!({ "name": "DB_PASSWORD" }).to_string()),
        )
        .unwrap();

        let events = db.list_secret_push_events(100, 0).unwrap();
        assert_eq!(events.len(), 3);

        // Sorted by ts DESC → env event first, then delete, then set.
        assert_eq!(events[0].source, "env");
        assert_eq!(events[0].action, "set");
        assert_eq!(events[0].secret_name, "DB_PASSWORD");
        assert_eq!(events[0].repo_name, "audit-repo");
        assert_eq!(events[0].deploy_env_id, Some(env_id));
        assert_eq!(events[0].env_name.as_deref(), Some("prod"));

        assert_eq!(events[1].source, "repo");
        assert_eq!(events[1].action, "delete");
        assert_eq!(events[1].secret_name, "OLD_KEY");
        assert_eq!(events[1].repo_name, "audit-repo");
        assert!(events[1].deploy_env_id.is_none());
        assert!(events[1].env_name.is_none());

        assert_eq!(events[2].source, "repo");
        assert_eq!(events[2].action, "set");
        assert_eq!(events[2].secret_name, "API_KEY");
    }

    #[test]
    fn test_list_secret_push_events_excludes_non_secret_rows() {
        let db = make_db();
        let (repo_id, env_id) = seed_repo_and_env(&db);

        // Non-secret sync event must be excluded.
        db.insert_sync_event(
            Some(repo_id),
            "project_sync",
            "2026-07-01T09:00:00Z",
            2,
            None,
        )
        .unwrap();
        // Non-secret deploy event (render) must be excluded.
        db.insert_deploy_event(
            Some(env_id),
            repo_id,
            "render",
            "2026-07-01T09:30:00Z",
            Some(r#"{"env":"prod"}"#),
        )
        .unwrap();
        // One real secret event so we know the query itself works.
        db.insert_sync_event(
            Some(repo_id),
            "secret",
            "2026-07-01T10:00:00Z",
            1,
            Some(&serde_json::json!({ "action": "set", "name": "TOKEN" }).to_string()),
        )
        .unwrap();

        let events = db.list_secret_push_events(100, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].secret_name, "TOKEN");
        assert_eq!(events[0].action, "set");
    }

    #[test]
    fn test_list_secret_push_events_pagination() {
        let db = make_db();
        let (repo_id, _env_id) = seed_repo_and_env(&db);

        for (i, hour) in ["10", "11", "12"].iter().enumerate() {
            db.insert_sync_event(
                Some(repo_id),
                "secret",
                &format!("2026-07-01T{hour}:00:00Z"),
                1,
                Some(
                    &serde_json::json!({ "action": "set", "name": format!("KEY_{i}") }).to_string(),
                ),
            )
            .unwrap();
        }

        // ts DESC → KEY_2 (12h), KEY_1 (11h), KEY_0 (10h).
        let first = db.list_secret_push_events(2, 0).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].secret_name, "KEY_2");
        assert_eq!(first[1].secret_name, "KEY_1");

        let second = db.list_secret_push_events(2, 2).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].secret_name, "KEY_0");
    }
}
