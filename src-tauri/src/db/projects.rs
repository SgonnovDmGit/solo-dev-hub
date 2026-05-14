// T-000094: project-related queries (CRUD, ordering, microservice connections,
// settings, templates, project_renames). Moved from db.rs.

use super::*;
use rusqlite::OptionalExtension;

impl AppDb {
    // ── Projects ──────────────────────────────────────────────────────────────

    pub fn create_project(
        &self,
        name: &str,
        description: Option<&str>,
        project_type: &str,
    ) -> SqlResult<Project> {
        let conn = self.conn.lock().unwrap();
        // F-025: new project goes to the top of the list — sort_order = MIN - 10.
        // Replaces the session-only freshProjectIds logic (persisted in DB now).
        let min_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MIN(sort_order), 0) FROM projects",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let new_order = min_order - 10;
        conn.execute(
            "INSERT INTO projects (name, description, project_type, sort_order) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![name, description, project_type, new_order],
        )?;
        let id = conn.last_insert_rowid();
        conn.query_row(
            "SELECT id, name, description, created_at, project_type FROM projects WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    created_at: row.get(3)?,
                    project_type: row.get(4)?,
                })
            },
        )
    }

    pub fn list_projects(&self) -> SqlResult<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        // F-025: ORDER BY sort_order first (manual user order), name as tie-breaker.
        let mut stmt = conn.prepare(
            "SELECT id, name, description, created_at, project_type FROM projects ORDER BY sort_order ASC, name ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                project_type: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn update_project(
        &self,
        id: i64,
        name: &str,
        description: Option<&str>,
    ) -> SqlResult<Project> {
        let conn = self.conn.lock().unwrap();
        // T-000092: detect name change → log to project_renames so sync-preamble
        // can rename `microservice-api/<old>/` to `<new>/` on parent server side.
        // Only logs when name actually differs (no-op for description-only edits).
        let old_name: Option<String> = conn
            .query_row(
                "SELECT name FROM projects WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .optional()?;
        conn.execute(
            "UPDATE projects SET name = ?1, description = ?2 WHERE id = ?3",
            rusqlite::params![name, description, id],
        )?;
        if let Some(prev) = old_name {
            if prev != name && !prev.is_empty() && !name.is_empty() {
                conn.execute(
                    "INSERT INTO project_renames (project_id, old_name, new_name) VALUES (?1, ?2, ?3)",
                    rusqlite::params![id, prev, name],
                )?;
            }
        }
        conn.query_row(
            "SELECT id, name, description, created_at, project_type FROM projects WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    created_at: row.get(3)?,
                    project_type: row.get(4)?,
                })
            },
        )
    }

    #[allow(dead_code)]
    pub fn get_project(&self, id: i64) -> SqlResult<Project> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, description, created_at, project_type FROM projects WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    created_at: row.get(3)?,
                    project_type: row.get(4)?,
                })
            },
        )
    }

    /// Delete project. If project is microservice type AND has parents, returns Err.
    pub fn delete_project(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        // Guard: microservice with parents must be disconnected first
        let ptype: Option<String> = conn
            .query_row(
                "SELECT project_type FROM projects WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .ok();
        if ptype.as_deref() == Some("microservice") {
            let parents: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM project_microservices WHERE microservice_project_id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if parents > 0 {
                return Err(format!(
                    "Microservice project has {} parent(s) — disconnect them first",
                    parents
                ));
            }
        }
        conn.execute("DELETE FROM projects WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ── F-025 Manual ordering (projects) ──────────────────────────────────────

    /// Move a project one slot up or down, with wrap-around at list boundaries.
    /// ▲ on first → moves to end; ▼ on last → moves to start.
    pub fn reorder_project(&self, id: i64, direction: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let current: i64 = conn.query_row(
            "SELECT sort_order FROM projects WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )?;
        match direction {
            "up" => {
                // find neighbor with the largest sort_order < current
                let neighbor: SqlResult<(i64, i64)> = conn.query_row(
                    "SELECT id, sort_order FROM projects WHERE sort_order < ?1
                     ORDER BY sort_order DESC LIMIT 1",
                    rusqlite::params![current],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                );
                if let Ok((nid, nord)) = neighbor {
                    conn.execute(
                        "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                        rusqlite::params![nord, id],
                    )?;
                    conn.execute(
                        "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                        rusqlite::params![current, nid],
                    )?;
                } else {
                    // already first → wrap to end (MAX + 10)
                    let max_order: i64 = conn.query_row(
                        "SELECT COALESCE(MAX(sort_order), 0) FROM projects",
                        [],
                        |row| row.get(0),
                    )?;
                    conn.execute(
                        "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                        rusqlite::params![max_order + 10, id],
                    )?;
                }
            }
            "down" => {
                let neighbor: SqlResult<(i64, i64)> = conn.query_row(
                    "SELECT id, sort_order FROM projects WHERE sort_order > ?1
                     ORDER BY sort_order ASC LIMIT 1",
                    rusqlite::params![current],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                );
                if let Ok((nid, nord)) = neighbor {
                    conn.execute(
                        "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                        rusqlite::params![nord, id],
                    )?;
                    conn.execute(
                        "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                        rusqlite::params![current, nid],
                    )?;
                } else {
                    // already last → wrap to start (MIN - 10)
                    let min_order: i64 = conn.query_row(
                        "SELECT COALESCE(MIN(sort_order), 0) FROM projects",
                        [],
                        |row| row.get(0),
                    )?;
                    conn.execute(
                        "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                        rusqlite::params![min_order - 10, id],
                    )?;
                }
            }
            _ => {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        Ok(())
    }

    /// Re-number a list of project ids to 10, 20, 30, ... (always-rebalance for projects).
    pub fn rebalance_projects(&self, ordered_ids: &[i64]) -> SqlResult<()> {
        if ordered_ids.is_empty() {
            return Ok(());
        }
        let mut sql = String::from("UPDATE projects SET sort_order = CASE id");
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        for (i, id) in ordered_ids.iter().enumerate() {
            sql.push_str(&format!(" WHEN ?{} THEN ?{}", i * 2 + 1, i * 2 + 2));
            params.push(Box::new(*id));
            params.push(Box::new(((i as i64) + 1) * 10));
        }
        sql.push_str(" END WHERE id IN (");
        for (i, _) in ordered_ids.iter().enumerate() {
            if i > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&format!("?{}", ordered_ids.len() * 2 + i + 1));
        }
        sql.push(')');
        for id in ordered_ids {
            params.push(Box::new(*id));
        }
        let conn = self.conn.lock().unwrap();
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        conn.execute(&sql, refs.as_slice())?;
        Ok(())
    }

    /// Reset all sort_order values to alphabetical ordering.
    /// - Projects: by `name COLLATE NOCASE ASC`, spaced 10 apart.
    /// - Repositories: grouped by role-priority (server → admin_client → client → …),
    ///   alphabetical `github_name` within each group, spaced 10 apart.
    /// Destructive: overwrites any manual user ordering. UI must confirm before calling.
    pub fn auto_sort_all(&self) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Projects — alphabetical
        let proj_ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM projects ORDER BY LOWER(name) ASC, id ASC",
            )?;
            let ids = stmt
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<SqlResult<Vec<i64>>>()?;
            ids
        };
        for (idx, pid) in proj_ids.iter().enumerate() {
            let order = (idx as i64 + 1) * 10;
            tx.execute(
                "UPDATE projects SET sort_order = ?1 WHERE id = ?2",
                rusqlite::params![order, pid],
            )?;
        }

        // Repositories — role-priority group → alphabetical within group
        let role_groups: [(Option<&str>, i64); 8] = [
            (Some("server"), 0),
            (Some("admin_client"), 1),
            (Some("client"), 2),
            (Some("test_client"), 3),
            (Some("microservice"), 4),
            (Some("landing"), 5),
            (Some("tool"), 6),
            (None, 99), // catch-all: role is NULL or unknown
        ];
        for (role, priority) in role_groups.iter() {
            let ids: Vec<i64> = if let Some(r) = role {
                let mut stmt = tx.prepare(
                    "SELECT id FROM repositories WHERE role = ?1 ORDER BY LOWER(COALESCE(github_name, description, '')) ASC, id ASC",
                )?;
                let v = stmt
                    .query_map(rusqlite::params![r], |row| row.get::<_, i64>(0))?
                    .collect::<SqlResult<Vec<i64>>>()?;
                v
            } else {
                let mut stmt = tx.prepare(
                    "SELECT id FROM repositories \
                     WHERE role IS NULL OR role NOT IN \
                       ('server','admin_client','client','test_client','microservice','landing','tool') \
                     ORDER BY LOWER(COALESCE(github_name, description, '')) ASC, id ASC",
                )?;
                let v = stmt
                    .query_map([], |row| row.get::<_, i64>(0))?
                    .collect::<SqlResult<Vec<i64>>>()?;
                v
            };
            for (idx, rid) in ids.iter().enumerate() {
                let order = priority * 1000 + (idx as i64 + 1) * 10;
                tx.execute(
                    "UPDATE repositories SET sort_order = ?1 WHERE id = ?2",
                    rusqlite::params![order, rid],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    // ── Microservice connections ──────────────────────────────────────────────

    /// Connect parent project to a microservice project. Validates:
    /// 1. target exists and project_type='microservice'
    /// 2. no cycle would be formed (DFS from target — if parent is reachable → cycle)
    /// 3. self-loop guarded by CHECK constraint at DB layer
    pub fn connect_microservice(
        &self,
        project_id: i64,
        microservice_project_id: i64,
    ) -> Result<(), String> {
        // Validate target type
        let ms_type: String = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT project_type FROM projects WHERE id = ?1",
                rusqlite::params![microservice_project_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?
        };
        if ms_type != "microservice" {
            return Err("Target project is not of type 'microservice'".to_string());
        }

        // Cycle check: inserting (parent → ms) creates a cycle if `parent` is reachable from `ms`.
        if self
            .is_reachable(microservice_project_id, project_id)
            .map_err(|e| e.to_string())?
        {
            return Err(
                "Cycle detected: target already references this project transitively".to_string(),
            );
        }

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO project_microservices (project_id, microservice_project_id) VALUES (?1, ?2)",
            rusqlite::params![project_id, microservice_project_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn disconnect_microservice(
        &self,
        project_id: i64,
        microservice_project_id: i64,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM project_microservices WHERE project_id = ?1 AND microservice_project_id = ?2",
            rusqlite::params![project_id, microservice_project_id],
        )?;
        Ok(())
    }

    /// DFS: is `target` reachable from `start` by following microservice_project_id edges?
    fn is_reachable(&self, start: i64, target: i64) -> SqlResult<bool> {
        if start == target {
            return Ok(true);
        }
        let conn = self.conn.lock().unwrap();
        let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut stack: Vec<i64> = vec![start];
        while let Some(node) = stack.pop() {
            if !visited.insert(node) {
                continue;
            }
            let mut stmt = conn.prepare(
                "SELECT microservice_project_id FROM project_microservices WHERE project_id = ?1",
            )?;
            let children: Vec<i64> = stmt
                .query_map(rusqlite::params![node], |row| row.get::<_, i64>(0))?
                .collect::<SqlResult<Vec<_>>>()?;
            for child in children {
                if child == target {
                    return Ok(true);
                }
                if !visited.contains(&child) {
                    stack.push(child);
                }
            }
        }
        Ok(false)
    }

    pub fn list_project_microservices(&self, project_id: i64) -> SqlResult<Vec<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT microservice_project_id FROM project_microservices WHERE project_id = ?1 ORDER BY microservice_project_id",
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id], |row| row.get(0))?;
        rows.collect()
    }

    /// List all projects of type 'microservice'.
    pub fn list_microservice_projects(&self) -> SqlResult<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, created_at, project_type FROM projects WHERE project_type = 'microservice' ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                project_type: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// List projects that have this microservice-project connected.
    pub fn list_parents_of_microservice(&self, ms_project_id: i64) -> SqlResult<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.name, p.description, p.created_at, p.project_type
             FROM projects p
             INNER JOIN project_microservices pm ON pm.project_id = p.id
             WHERE pm.microservice_project_id = ?1
             ORDER BY p.name",
        )?;
        let rows = stmt.query_map(rusqlite::params![ms_project_id], |row| {
            Ok(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                project_type: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Find exactly one 'server' role repository inside a microservice project.
    /// Err if 0 or >1 matches — sync direction needs a single clear target.
    pub fn server_repo_of_microservice(&self, ms_project_id: i64) -> Result<Repository, String> {
        let servers = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
                     FROM repositories WHERE project_id = ?1 AND role = 'server'",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![ms_project_id], row_to_repo)
                .map_err(|e| e.to_string())?;
            rows.collect::<SqlResult<Vec<Repository>>>()
                .map_err(|e| e.to_string())?
        };
        match servers.len() {
            0 => Err(format!(
                "Microservice project {} has no server-repo",
                ms_project_id
            )),
            1 => Ok(servers.into_iter().next().unwrap()),
            n => Err(format!(
                "Microservice project {} has {} server-repos (expected exactly 1)",
                ms_project_id, n
            )),
        }
    }

    /// Change project_type. Only blocked when the project is currently a **microservice**
    /// that is connected to parents — changing its type would leave parents with a dangling
    /// "microservice" pointer into a standard project. Repos and own-connected microservices
    /// are NOT a blocker.
    pub fn update_project_type(&self, id: i64, new_type: &str) -> Result<Project, String> {
        if new_type != "standard" && new_type != "microservice" {
            return Err(format!("Invalid project_type: {}", new_type));
        }
        {
            let conn = self.conn.lock().unwrap();
            let parent_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM project_microservices WHERE microservice_project_id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .map_err(|e| e.to_string())?;
            if parent_count > 0 {
                return Err(
                    "Project is connected to parents as a microservice — disconnect first".to_string(),
                );
            }
            conn.execute(
                "UPDATE projects SET project_type = ?1 WHERE id = ?2",
                rusqlite::params![new_type, id],
            )
            .map_err(|e| e.to_string())?;
        }
        // Return the updated project
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, description, created_at, project_type FROM projects WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    created_at: row.get(3)?,
                    project_type: row.get(4)?,
                })
            },
        )
        .map_err(|e| e.to_string())
    }

    // ── Project-rename log (T-000092) ──────────────────────────────────────────

    pub fn list_renames_for_project(
        &self,
        project_id: i64,
    ) -> SqlResult<Vec<crate::models::ProjectRename>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, old_name, new_name, renamed_at
             FROM project_renames WHERE project_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id], |row| {
            Ok(crate::models::ProjectRename {
                id: row.get(0)?,
                project_id: row.get(1)?,
                old_name: row.get(2)?,
                new_name: row.get(3)?,
                renamed_at: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ── Settings ──────────────────────────────────────────────────────────────

    pub fn get_setting(&self, key: &str) -> SqlResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query(rusqlite::params![key])?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(None)
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM settings WHERE key = ?1", rusqlite::params![key])?;
        Ok(())
    }

    // ── Templates (0.6.0) ─────────────────────────────────────────────────────

    pub fn list_template_languages(&self) -> SqlResult<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT DISTINCT language_key FROM templates ORDER BY language_key")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn list_template_files(&self, language_key: &str) -> SqlResult<Vec<TemplateFile>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT language_key, file_name, content, is_custom, updated_at
             FROM templates WHERE language_key = ?1 ORDER BY file_name",
        )?;
        let rows = stmt.query_map(rusqlite::params![language_key], |row| {
            Ok(TemplateFile {
                language_key: row.get(0)?,
                file_name: row.get(1)?,
                content: row.get(2)?,
                is_custom: row.get::<_, i64>(3)? != 0,
                updated_at: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn get_template_file(
        &self,
        language_key: &str,
        file_name: &str,
    ) -> SqlResult<Option<TemplateFile>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT language_key, file_name, content, is_custom, updated_at
             FROM templates WHERE language_key = ?1 AND file_name = ?2",
            rusqlite::params![language_key, file_name],
            |row| {
                Ok(TemplateFile {
                    language_key: row.get(0)?,
                    file_name: row.get(1)?,
                    content: row.get(2)?,
                    is_custom: row.get::<_, i64>(3)? != 0,
                    updated_at: row.get(4)?,
                })
            },
        );
        match result {
            Ok(f) => Ok(Some(f)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn upsert_template_file(
        &self,
        language_key: &str,
        file_name: &str,
        content: &str,
        is_custom: bool,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO templates (language_key, file_name, content, is_custom, updated_at)
             VALUES (?1, ?2, ?3, ?4, CURRENT_TIMESTAMP)
             ON CONFLICT(language_key, file_name) DO UPDATE SET
                content = excluded.content,
                is_custom = excluded.is_custom,
                updated_at = CURRENT_TIMESTAMP",
            rusqlite::params![
                language_key,
                file_name,
                content,
                if is_custom { 1 } else { 0 }
            ],
        )?;
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

    // ── Project tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_create_project() {
        let db = make_db();
        let p = db
            .create_project("My App", Some("A great app"), "standard")
            .unwrap();
        assert_eq!(p.name, "My App");
        assert_eq!(p.description.as_deref(), Some("A great app"));
        assert!(p.id > 0);
    }

    #[test]
    fn test_list_projects() {
        let db = make_db();
        db.create_project("Alpha", None, "standard").unwrap();
        db.create_project("Beta", Some("desc"), "standard").unwrap();
        let projects = db.list_projects().unwrap();
        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn test_update_project() {
        let db = make_db();
        let p = db.create_project("Old Name", None, "standard").unwrap();
        let updated = db
            .update_project(p.id, "New Name", Some("new desc"))
            .unwrap();
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.description.as_deref(), Some("new desc"));
    }

    #[test]
    fn test_update_project_logs_rename() {
        // T-000092: name change → entry in project_renames.
        let db = make_db();
        let p = db.create_project("Old MS", None, "microservice").unwrap();
        db.update_project(p.id, "New MS", None).unwrap();
        let renames = db.list_renames_for_project(p.id).unwrap();
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0].old_name, "Old MS");
        assert_eq!(renames[0].new_name, "New MS");
    }

    #[test]
    fn test_update_project_description_only_no_rename_log() {
        // T-000092: description-only edit must NOT create a project_renames row.
        let db = make_db();
        let p = db.create_project("Stable MS", None, "microservice").unwrap();
        db.update_project(p.id, "Stable MS", Some("new desc")).unwrap();
        let renames = db.list_renames_for_project(p.id).unwrap();
        assert!(renames.is_empty());
    }

    #[test]
    fn test_update_project_multi_rename_chain() {
        // T-000092: two consecutive renames → two entries in chain order.
        let db = make_db();
        let p = db.create_project("V1", None, "microservice").unwrap();
        db.update_project(p.id, "V2", None).unwrap();
        db.update_project(p.id, "V3", None).unwrap();
        let renames = db.list_renames_for_project(p.id).unwrap();
        assert_eq!(renames.len(), 2);
        assert_eq!(renames[0].old_name, "V1");
        assert_eq!(renames[0].new_name, "V2");
        assert_eq!(renames[1].old_name, "V2");
        assert_eq!(renames[1].new_name, "V3");
    }

    #[test]
    fn test_delete_project() {
        let db = make_db();
        let p = db.create_project("ToDelete", None, "standard").unwrap();
        db.delete_project(p.id).unwrap();
        let projects = db.list_projects().unwrap();
        assert!(projects.is_empty());
    }

    // ── Settings tests ────────────────────────────────────────────────────────

    #[test]
    fn test_settings() {
        let db = make_db();
        assert!(db.get_setting("foo").unwrap().is_none());
        db.set_setting("foo", "bar").unwrap();
        assert_eq!(db.get_setting("foo").unwrap().as_deref(), Some("bar"));
        // Upsert
        db.set_setting("foo", "baz").unwrap();
        assert_eq!(db.get_setting("foo").unwrap().as_deref(), Some("baz"));
        // Delete
        db.delete_setting("foo").unwrap();
        assert!(db.get_setting("foo").unwrap().is_none());
    }

    // ── F-012: Microservice as project type — tests ──────────────────────────

    fn make_ms_project(db: &AppDb, name: &str, with_server_repo: bool) -> i64 {
        let p = db.create_project(name, None, "microservice").unwrap();
        if with_server_repo {
            db.insert_local_repository("/tmp/srv", "srv", Some(p.id), Some("server"))
                .unwrap();
        }
        p.id
    }

    #[test]
    fn test_create_project_with_type() {
        let db = make_db();
        let p = db.create_project("ms", None, "microservice").unwrap();
        assert_eq!(p.project_type, "microservice");
        let p2 = db.create_project("std", None, "standard").unwrap();
        assert_eq!(p2.project_type, "standard");
    }

    #[test]
    fn test_connect_microservice_rejects_standard_target() {
        let db = make_db();
        let parent = db.create_project("parent", None, "standard").unwrap();
        let target = db.create_project("not-ms", None, "standard").unwrap();
        let err = db
            .connect_microservice(parent.id, target.id)
            .unwrap_err();
        assert!(err.contains("not of type"));
    }

    #[test]
    fn test_connect_microservice_detects_direct_cycle() {
        let db = make_db();
        let a = make_ms_project(&db, "A", true);
        let b = make_ms_project(&db, "B", true);
        // A → B
        db.connect_microservice(a, b).unwrap();
        // B → A would form a 2-cycle
        let err = db.connect_microservice(b, a).unwrap_err();
        assert!(err.contains("Cycle"));
    }

    #[test]
    fn test_connect_microservice_detects_transitive_cycle() {
        let db = make_db();
        let a = make_ms_project(&db, "A", true);
        let b = make_ms_project(&db, "B", true);
        let c = make_ms_project(&db, "C", true);
        db.connect_microservice(a, b).unwrap(); // A → B
        db.connect_microservice(b, c).unwrap(); // B → C
        // C → A would form A → B → C → A
        let err = db.connect_microservice(c, a).unwrap_err();
        assert!(err.contains("Cycle"));
    }

    #[test]
    fn test_disconnect_microservice_works() {
        let db = make_db();
        let a = make_ms_project(&db, "A", true);
        let b = make_ms_project(&db, "B", true);
        db.connect_microservice(a, b).unwrap();
        assert_eq!(db.list_project_microservices(a).unwrap(), vec![b]);
        db.disconnect_microservice(a, b).unwrap();
        assert!(db.list_project_microservices(a).unwrap().is_empty());
    }

    #[test]
    fn test_list_microservice_projects() {
        let db = make_db();
        let _std = db.create_project("std", None, "standard").unwrap();
        make_ms_project(&db, "ms1", true);
        make_ms_project(&db, "ms2", true);
        let ms = db.list_microservice_projects().unwrap();
        assert_eq!(ms.len(), 2);
        let names: Vec<&str> = ms.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"ms1"));
        assert!(names.contains(&"ms2"));
    }

    #[test]
    fn test_list_parents_of_microservice() {
        let db = make_db();
        let p1 = db.create_project("parent1", None, "standard").unwrap();
        let p2 = db.create_project("parent2", None, "standard").unwrap();
        let ms = make_ms_project(&db, "ms", true);
        db.connect_microservice(p1.id, ms).unwrap();
        db.connect_microservice(p2.id, ms).unwrap();
        let parents = db.list_parents_of_microservice(ms).unwrap();
        assert_eq!(parents.len(), 2);
        let names: Vec<&str> = parents.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"parent1"));
        assert!(names.contains(&"parent2"));
    }

    #[test]
    fn test_delete_microservice_blocked_with_parents() {
        let db = make_db();
        let parent = db.create_project("parent", None, "standard").unwrap();
        let ms = make_ms_project(&db, "ms", true);
        db.connect_microservice(parent.id, ms).unwrap();
        let err = db.delete_project(ms).unwrap_err();
        assert!(err.contains("parent"));
        // Disconnect parent → delete works
        db.disconnect_microservice(parent.id, ms).unwrap();
        db.delete_project(ms).unwrap();
    }

    #[test]
    fn test_server_repo_of_microservice_exact_one() {
        let db = make_db();
        let ms = make_ms_project(&db, "ms-no-server", false);
        let err = db.server_repo_of_microservice(ms).unwrap_err();
        assert!(err.contains("no server-repo"));

        let ms2 = make_ms_project(&db, "ms-one", true);
        let r = db.server_repo_of_microservice(ms2).unwrap();
        assert_eq!(r.role.as_deref(), Some("server"));

        let ms3 = make_ms_project(&db, "ms-two", true);
        db.insert_local_repository("/tmp/srv2", "srv2", Some(ms3), Some("server"))
            .unwrap();
        let err = db.server_repo_of_microservice(ms3).unwrap_err();
        assert!(err.contains("2"));
    }

    #[test]
    fn test_update_project_type_blocked_only_when_connected_as_microservice() {
        let db = make_db();
        // Setup: project A is microservice with own repo + own connected ms B.
        // Parent C connects to A.
        let a = make_ms_project(&db, "A", true);
        let b = make_ms_project(&db, "B", true);
        let c = db.create_project("C", None, "standard").unwrap();
        db.connect_microservice(a, b).unwrap();
        db.connect_microservice(c.id, a).unwrap();

        // A has parents — block.
        let err = db.update_project_type(a, "standard").unwrap_err();
        assert!(err.contains("parents"));

        // Disconnect parent C → A no longer has parents. Update should succeed
        // even though A has its own repo + own microservice (B).
        db.disconnect_microservice(c.id, a).unwrap();
        let updated = db.update_project_type(a, "standard").unwrap();
        assert_eq!(updated.project_type, "standard");

        // Invalid type rejected
        let err = db.update_project_type(a, "garbage").unwrap_err();
        assert!(err.contains("Invalid"));
    }

    // ── Templates tests ───────────────────────────────────────────────────────

    #[test]
    fn test_template_upsert_and_get() {
        let db = make_db();
        db.upsert_template_file("go", "Dockerfile", "FROM scratch", false).unwrap();
        let f = db.get_template_file("go", "Dockerfile").unwrap().unwrap();
        assert_eq!(f.language_key, "go");
        assert_eq!(f.file_name, "Dockerfile");
        assert_eq!(f.content, "FROM scratch");
        assert!(!f.is_custom);

        // Upsert overwrite
        db.upsert_template_file("go", "Dockerfile", "FROM alpine", true).unwrap();
        let f = db.get_template_file("go", "Dockerfile").unwrap().unwrap();
        assert_eq!(f.content, "FROM alpine");
        assert!(f.is_custom);
    }

    #[test]
    fn test_template_get_missing_returns_none() {
        let db = make_db();
        assert!(db.get_template_file("go", "missing").unwrap().is_none());
    }

    #[test]
    fn test_template_list_languages_and_files() {
        let db = make_db();
        db.upsert_template_file("go", "Dockerfile", "x", false).unwrap();
        db.upsert_template_file("go", "compose.yml", "x", false).unwrap();
        db.upsert_template_file("rust", "Dockerfile", "x", false).unwrap();
        let langs = db.list_template_languages().unwrap();
        assert_eq!(langs, vec!["go".to_string(), "rust".to_string()]);
        let go_files = db.list_template_files("go").unwrap();
        assert_eq!(go_files.len(), 2);
    }
}
