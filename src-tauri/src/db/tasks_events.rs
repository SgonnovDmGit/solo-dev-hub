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

    pub fn delete_task(&self, id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tasks WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    pub fn list_tasks_by_repo(&self, repository_id: i64, source: &str) -> SqlResult<Vec<crate::models::Task>> {
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

    pub fn list_task_events_by_task(&self, task_id: i64) -> SqlResult<Vec<crate::models::TaskEvent>> {
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

    pub fn list_sync_events(&self, limit: u32, offset: u32) -> SqlResult<Vec<crate::models::SyncEvent>> {
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

    pub fn list_deploy_events(&self, limit: u32, offset: u32) -> SqlResult<Vec<crate::models::DeployEvent>> {
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
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        let task = db.insert_task(
            repo.id, "T-042", "T", "Some task",
            Some(4.0), Some("high"), Some("open"), None, "todo", "2026-04-26",
        ).unwrap();
        assert_eq!(task.task_id, "T-042");
        assert_eq!(task.prefix, "T");
        assert_eq!(task.priority.as_deref(), Some("high"));
    }

    #[test]
    fn test_list_tasks_by_repo_filters_source() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        db.insert_task(repo.id, "T-001", "T", "Open task", Some(2.0), Some("medium"), Some("open"), None, "todo", "2026-04-20").unwrap();
        db.insert_task(repo.id, "T-002", "T", "Done task", None, None, None, Some("v0.20.0"), "done", "2026-04-19").unwrap();
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
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        let task = db.insert_task(repo.id, "T-001", "T", "Test", Some(1.0), Some("low"), Some("open"), None, "todo", "2026-04-26").unwrap();
        db.insert_task_event(task.id, "created", "2026-04-26T00:00:00Z", None, Some("open")).unwrap();
        let events = db.list_task_events_by_task(task.id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "created");
    }

    #[test]
    fn test_mark_tasks_migrated_sets_timestamp() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        db.mark_tasks_migrated(repo.id, "2026-04-26T12:00:00Z").unwrap();
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());
    }

    #[test]
    fn test_get_tasks_migrated_at_null_when_unset() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_none());
    }

    #[test]
    fn test_insert_sync_event_with_repo_id() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        db.insert_sync_event(Some(repo.id), "project_sync", "2026-04-26T10:00:00Z", 3, None).unwrap();
        let events = db.list_sync_events(10, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sync_type, "project_sync");
        assert_eq!(events[0].change_count, 3);
    }

    #[test]
    fn test_insert_sync_event_portfolio_wide_null_repo() {
        let db = make_db();
        db.insert_sync_event(None, "tasks", "2026-04-26T10:00:00Z", 0, None).unwrap();
        let events = db.list_sync_events(10, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].repository_id.is_none());
    }

    #[test]
    fn test_insert_deploy_event_with_details_json() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r1", "r1", None, None).unwrap();
        db.insert_deploy_event(None, repo.id, "render", "2026-04-26T10:00:00Z", Some(r#"{"env":"prod"}"#)).unwrap();
        let events = db.list_deploy_events(10, 0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "render");
        assert_eq!(events[0].details.as_deref(), Some(r#"{"env":"prod"}"#));
    }
}
