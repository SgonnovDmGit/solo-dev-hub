// T-000094: repository CRUD + upsert with rename detection + repo_renames.
// Moved from db.rs.

use super::*;
use std::path::Path;

impl AppDb {
    // ── Repositories ──────────────────────────────────────────────────────────

    /// Test-only wrapper returning `Repository` directly. Converts Ambiguous into
    /// an error — production code must use `upsert_repository_with_outcome` and
    /// handle Ambiguous via the merge dialog flow.
    #[cfg(test)]
    pub fn upsert_repository(
        &self,
        github_name: &str,
        github_url: Option<&str>,
        description: Option<&str>,
        language: Option<&str>,
        last_pushed_at: Option<&str>,
        github_id: Option<i64>,
    ) -> SqlResult<Repository> {
        match self.upsert_repository_with_outcome(
            github_name,
            github_url,
            description,
            language,
            last_pushed_at,
            github_id,
        )? {
            UpsertRepoOutcome::Inserted { repo } => Ok(repo),
            UpsertRepoOutcome::Merged { repo, .. } => Ok(repo),
            UpsertRepoOutcome::Ambiguous { .. } => Err(rusqlite::Error::InvalidQuery),
        }
    }

    pub fn upsert_repository_with_outcome(
        &self,
        github_name: &str,
        github_url: Option<&str>,
        description: Option<&str>,
        language: Option<&str>,
        last_pushed_at: Option<&str>,
        github_id: Option<i64>,
    ) -> SqlResult<UpsertRepoOutcome> {
        let conn = self.conn.lock().unwrap();

        // If github_id provided, try to find existing repo by github_id first
        if let Some(gid) = github_id {
            let existing: SqlResult<(i64, String)> = conn.query_row(
                "SELECT id, github_name FROM repositories WHERE github_id = ?1",
                rusqlite::params![gid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            );
            if let Ok((existing_id, existing_name)) = existing {
                // Found by github_id — update it (handles renames)
                if existing_name != github_name {
                    // F-033: log rename to repo_renames so sync-preamble can rename
                    // counterparty-side folders (client-requirements/<X>, etc.) on fs.
                    // Canonical = last segment after '/' for GitHub names.
                    let old_canonical =
                        existing_name.rsplit('/').next().unwrap_or("").to_string();
                    let new_canonical = github_name.rsplit('/').next().unwrap_or("").to_string();
                    if !old_canonical.is_empty()
                        && !new_canonical.is_empty()
                        && old_canonical != new_canonical
                    {
                        conn.execute(
                            "INSERT INTO repo_renames (repository_id, old_canonical, new_canonical)
                             VALUES (?1, ?2, ?3)",
                            rusqlite::params![existing_id, old_canonical, new_canonical],
                        )?;
                    }
                    conn.execute(
                        "UPDATE repositories SET github_name = ?1, github_url = ?2, description = ?3,
                            language = ?4, last_pushed_at = ?5, github_id = ?6, updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?7",
                        rusqlite::params![github_name, github_url, description, language, last_pushed_at, gid, existing_id],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE repositories SET github_url = ?1, description = ?2, language = ?3,
                            last_pushed_at = ?4, github_id = ?5, updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?6",
                        rusqlite::params![
                            github_url,
                            description,
                            language,
                            last_pushed_at,
                            gid,
                            existing_id
                        ],
                    )?;
                }
                let repo = conn.query_row(
                    "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
                     FROM repositories WHERE id = ?1",
                    rusqlite::params![existing_id],
                    row_to_repo,
                )?;
                return Ok(UpsertRepoOutcome::Inserted { repo });
            }
        }

        // B-007: try to merge with local-only record(s) whose local_path basename
        // matches the github repo name (case-insensitive).
        let repo_basename = github_name.rsplit('/').next().unwrap_or(github_name);
        let mut stmt = conn.prepare(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories
             WHERE github_name IS NULL AND github_id IS NULL AND local_path IS NOT NULL",
        )?;
        let rows = stmt.query_map([], row_to_repo)?;
        let mut matches: Vec<Repository> = Vec::new();
        for row in rows {
            let repo = row?;
            if let Some(ref path) = repo.local_path {
                if let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) {
                    if name.eq_ignore_ascii_case(repo_basename) {
                        matches.push(repo);
                    }
                }
            }
        }
        drop(stmt);

        match matches.len() {
            0 => {
                // Normal upsert by github_name. F-025: new repos go to end of unassigned group.
                let max_order: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(sort_order), 0) FROM repositories WHERE project_id IS NULL",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                let new_order = max_order + 10;
                conn.execute(
                    "INSERT INTO repositories (github_name, github_url, description, language, last_pushed_at, github_id, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(github_name) DO UPDATE SET
                        github_url = excluded.github_url,
                        description = excluded.description,
                        language = excluded.language,
                        last_pushed_at = excluded.last_pushed_at,
                        github_id = COALESCE(excluded.github_id, repositories.github_id),
                        updated_at = CURRENT_TIMESTAMP",
                    rusqlite::params![github_name, github_url, description, language, last_pushed_at, github_id, new_order],
                )?;
                let repo = conn.query_row(
                    "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
                     FROM repositories WHERE github_name = ?1",
                    rusqlite::params![github_name],
                    row_to_repo,
                )?;
                Ok(UpsertRepoOutcome::Inserted { repo })
            }
            1 => {
                let local = &matches[0];
                let local_id = local.id;
                let local_path = local.local_path.clone().unwrap_or_default();
                conn.execute(
                    "UPDATE repositories SET
                        github_name = ?1, github_url = ?2, description = ?3,
                        language = ?4, last_pushed_at = ?5, github_id = ?6,
                        updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?7",
                    rusqlite::params![github_name, github_url, description, language, last_pushed_at, github_id, local_id],
                )?;
                let repo = conn.query_row(
                    "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
                     FROM repositories WHERE id = ?1",
                    rusqlite::params![local_id],
                    row_to_repo,
                )?;
                Ok(UpsertRepoOutcome::Merged {
                    repo,
                    merged_with_local_id: local_id,
                    local_path,
                })
            }
            _ => Ok(UpsertRepoOutcome::Ambiguous {
                github_name: github_name.to_string(),
                github_url: github_url.map(String::from),
                description: description.map(String::from),
                language: language.map(String::from),
                last_pushed_at: last_pushed_at.map(String::from),
                github_id,
                candidates: matches,
            }),
        }
    }

    /// Merge a GitHub repo into a specific local-only record (user picked from ambiguous dialog).
    pub fn resolve_merge_with_local(
        &self,
        local_id: i64,
        github_name: &str,
        github_url: Option<&str>,
        description: Option<&str>,
        language: Option<&str>,
        last_pushed_at: Option<&str>,
        github_id: Option<i64>,
    ) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET
                github_name = ?1, github_url = ?2, description = ?3,
                language = ?4, last_pushed_at = ?5, github_id = ?6,
                updated_at = CURRENT_TIMESTAMP
             WHERE id = ?7",
            rusqlite::params![github_name, github_url, description, language, last_pushed_at, github_id, local_id],
        )?;
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE id = ?1",
            rusqlite::params![local_id],
            row_to_repo,
        )
    }

    /// Force-insert a GitHub repo, bypassing local-only basename dedup
    /// (user chose "create new entry" in the ambiguous dialog).
    pub fn force_insert_github_repo(
        &self,
        github_name: &str,
        github_url: Option<&str>,
        description: Option<&str>,
        language: Option<&str>,
        last_pushed_at: Option<&str>,
        github_id: Option<i64>,
    ) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        // F-025: same placement rule as upsert_repository_with_outcome.
        let max_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM repositories WHERE project_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let new_order = max_order + 10;
        conn.execute(
            "INSERT INTO repositories (github_name, github_url, description, language, last_pushed_at, github_id, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(github_name) DO UPDATE SET
                github_url = excluded.github_url,
                description = excluded.description,
                language = excluded.language,
                last_pushed_at = excluded.last_pushed_at,
                github_id = COALESCE(excluded.github_id, repositories.github_id),
                updated_at = CURRENT_TIMESTAMP",
            rusqlite::params![github_name, github_url, description, language, last_pushed_at, github_id, new_order],
        )?;
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE github_name = ?1",
            rusqlite::params![github_name],
            row_to_repo,
        )
    }

    /// Insert a local-only repository (no GitHub association).
    /// `display_name` is stored in `description` column and serves as the UI title
    /// (since `github_name` is NULL). This is a semantic overload of `description`:
    /// for GitHub-imported repos it holds the GitHub description; for local repos
    /// it holds the human-readable folder name the user typed.
    pub fn insert_local_repository(
        &self,
        local_path: &str,
        display_name: &str,
        project_id: Option<i64>,
        role: Option<&str>,
    ) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        // F-025: new repo goes to the end of its project group — sort_order = MAX + 10.
        let max_order: i64 = if let Some(pid) = project_id {
            conn.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM repositories WHERE project_id = ?1",
                rusqlite::params![pid],
                |row| row.get(0),
            )
            .unwrap_or(0)
        } else {
            conn.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM repositories WHERE project_id IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
        };
        let new_order = max_order + 10;
        conn.execute(
            "INSERT INTO repositories
                (github_name, github_url, project_id, role, description, local_path, sort_order)
             VALUES (NULL, NULL, ?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![project_id, role, display_name, local_path, new_order],
        )?;
        let id = conn.last_insert_rowid();
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE id = ?1",
            rusqlite::params![id],
            row_to_repo,
        )
    }

    // ── F-025 Manual ordering (repos) ─────────────────────────────────────────

    /// Move a repo one slot up or down within its project group, with wrap-around.
    pub fn reorder_repo(&self, repo_id: i64, direction: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let (project_id, current): (Option<i64>, i64) = conn.query_row(
            "SELECT project_id, sort_order FROM repositories WHERE id = ?1",
            rusqlite::params![repo_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (neighbor_sql, wrap_agg_sql) = match direction {
            "up" => (
                "SELECT id, sort_order FROM repositories
                 WHERE sort_order < ?1 AND (project_id IS ?2 OR (project_id IS NULL AND ?2 IS NULL))
                 ORDER BY sort_order DESC LIMIT 1",
                "SELECT COALESCE(MAX(sort_order), 0) FROM repositories
                 WHERE (project_id IS ?1 OR (project_id IS NULL AND ?1 IS NULL))",
            ),
            "down" => (
                "SELECT id, sort_order FROM repositories
                 WHERE sort_order > ?1 AND (project_id IS ?2 OR (project_id IS NULL AND ?2 IS NULL))
                 ORDER BY sort_order ASC LIMIT 1",
                "SELECT COALESCE(MIN(sort_order), 0) FROM repositories
                 WHERE (project_id IS ?1 OR (project_id IS NULL AND ?1 IS NULL))",
            ),
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        let neighbor: SqlResult<(i64, i64)> = conn.query_row(
            neighbor_sql,
            rusqlite::params![current, project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );
        if let Ok((nid, nord)) = neighbor {
            conn.execute(
                "UPDATE repositories SET sort_order = ?1 WHERE id = ?2",
                rusqlite::params![nord, repo_id],
            )?;
            conn.execute(
                "UPDATE repositories SET sort_order = ?1 WHERE id = ?2",
                rusqlite::params![current, nid],
            )?;
        } else {
            // wrap-around: move to opposite end of group
            let edge: i64 = conn.query_row(
                wrap_agg_sql,
                rusqlite::params![project_id],
                |row| row.get(0),
            )?;
            let new_order = if direction == "up" { edge + 10 } else { edge - 10 };
            conn.execute(
                "UPDATE repositories SET sort_order = ?1 WHERE id = ?2",
                rusqlite::params![new_order, repo_id],
            )?;
        }
        Ok(())
    }

    /// Re-number a list of repo ids within a project to 10, 20, 30, ... (always-rebalance strategy).
    /// Used on D&D drop within a group. Single query via CASE expression = atomic.
    pub fn rebalance_repo_group(&self, ordered_ids: &[i64]) -> SqlResult<()> {
        if ordered_ids.is_empty() {
            return Ok(());
        }
        let mut sql = String::from("UPDATE repositories SET sort_order = CASE id");
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

    pub fn assign_repository(
        &self,
        id: i64,
        project_id: Option<i64>,
        role: Option<&str>,
    ) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        // F-025: cross-project move places repo at the end of the new group (MAX + 10).
        // Preserves intra-group order for stayers; moved repo lands at bottom of target.
        let current_pid: Option<i64> = conn
            .query_row(
                "SELECT project_id FROM repositories WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap_or(None);
        let group_changed = current_pid != project_id;
        if group_changed {
            let max_order: i64 = if let Some(pid) = project_id {
                conn.query_row(
                    "SELECT COALESCE(MAX(sort_order), 0) FROM repositories WHERE project_id = ?1",
                    rusqlite::params![pid],
                    |row| row.get(0),
                )
                .unwrap_or(0)
            } else {
                conn.query_row(
                    "SELECT COALESCE(MAX(sort_order), 0) FROM repositories WHERE project_id IS NULL",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0)
            };
            let new_order = max_order + 10;
            conn.execute(
                "UPDATE repositories SET project_id = ?1, role = ?2, sort_order = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?4",
                rusqlite::params![project_id, role, new_order, id],
            )?;
        } else {
            conn.execute(
                "UPDATE repositories SET project_id = ?1, role = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?3",
                rusqlite::params![project_id, role, id],
            )?;
        }
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE id = ?1",
            rusqlite::params![id],
            row_to_repo,
        )
    }

    pub fn list_repos_by_project(&self, project_id: Option<i64>) -> SqlResult<Vec<Repository>> {
        let conn = self.conn.lock().unwrap();
        // F-025: ORDER BY sort_order (user manual), github_name as tie-breaker.
        let sql = if project_id.is_some() {
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE project_id = ?1 ORDER BY sort_order ASC, github_name ASC"
        } else {
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE project_id IS NULL ORDER BY sort_order ASC, github_name ASC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = if let Some(pid) = project_id {
            stmt.query_map(rusqlite::params![pid], row_to_repo)?
                .collect::<SqlResult<Vec<Repository>>>()?
        } else {
            stmt.query_map([], row_to_repo)?
                .collect::<SqlResult<Vec<Repository>>>()?
        };
        Ok(rows)
    }

    pub fn list_all_repos(&self) -> SqlResult<Vec<Repository>> {
        let conn = self.conn.lock().unwrap();
        // F-025: ORDER BY sort_order per project_id grouping, then github_name.
        let mut stmt = conn.prepare(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories ORDER BY sort_order ASC, github_name ASC",
        )?;
        let rows = stmt.query_map([], row_to_repo)?;
        rows.collect()
    }

    pub fn get_repository(&self, id: i64) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE id = ?1",
            rusqlite::params![id],
            row_to_repo,
        )
    }

    pub fn get_repository_by_name(&self, github_name: &str) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE github_name = ?1",
            rusqlite::params![github_name],
            row_to_repo,
        )
    }

    pub fn delete_repository(&self, id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM repositories WHERE id = ?1",
            rusqlite::params![id],
        )?;
        Ok(())
    }

    pub fn set_repo_local_path(&self, id: i64, local_path: Option<&str>) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET local_path = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![local_path, id],
        )?;
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target FROM repositories WHERE id = ?1",
            rusqlite::params![id],
            row_to_repo,
        )
    }

    /// Update a repo's `description`. For local-only repos this also drives
    /// `canonical_folder_name()` (used as cross-repo sync subfolder name), so
    /// changes get logged to `repo_renames` for downstream sync-preamble replay.
    /// For GitHub-tracked repos the canonical comes from `github_name` and is
    /// unaffected by description, so no rename event is written even if description changes.
    pub fn update_repo_description(&self, id: i64, new_description: &str) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Read current repo state (including github_id to know if local-only)
        let old_repo: Repository = tx.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target FROM repositories WHERE id = ?1",
            rusqlite::params![id],
            row_to_repo,
        )?;
        let old_canonical = old_repo.canonical_folder_name();

        tx.execute(
            "UPDATE repositories SET description = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![new_description, id],
        )?;

        // Build a hypothetical new state to compute new canonical without re-querying
        let new_repo = Repository {
            description: Some(new_description.to_string()),
            ..old_repo.clone()
        };
        let new_canonical = new_repo.canonical_folder_name();

        if old_canonical != new_canonical {
            tx.execute(
                "INSERT INTO repo_renames (repository_id, old_canonical, new_canonical, renamed_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, old_canonical, new_canonical, chrono::Utc::now().to_rfc3339()],
            )?;
        }

        tx.commit()?;

        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target FROM repositories WHERE id = ?1",
            rusqlite::params![id],
            row_to_repo,
        )
    }

    // ── Rename log (F-033) ────────────────────────────────────────────────────

    pub fn list_renames_for_repo(&self, repo_id: i64) -> SqlResult<Vec<RepoRename>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, old_canonical, new_canonical, renamed_at
             FROM repo_renames WHERE repository_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![repo_id], |row| {
            Ok(RepoRename {
                id: row.get(0)?,
                repository_id: row.get(1)?,
                old_canonical: row.get(2)?,
                new_canonical: row.get(3)?,
                renamed_at: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn list_all_renames(&self) -> SqlResult<Vec<RepoRename>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repository_id, old_canonical, new_canonical, renamed_at
             FROM repo_renames ORDER BY renamed_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RepoRename {
                id: row.get(0)?,
                repository_id: row.get(1)?,
                old_canonical: row.get(2)?,
                new_canonical: row.get(3)?,
                renamed_at: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    // ── Repository tests ──────────────────────────────────────────────────────

    #[test]
    fn test_upsert_repository_insert() {
        let db = make_db();
        let r = db
            .upsert_repository(
                "owner/test-repo",
                Some("https://github.com/owner/test-repo"),
                Some("A test repo"),
                Some("Rust"),
                None,
                None,
            )
            .unwrap();
        assert_eq!(r.github_name.as_deref(), Some("owner/test-repo"));
        assert_eq!(r.description.as_deref(), Some("A test repo"));
        assert_eq!(r.language.as_deref(), Some("Rust"));
    }

    #[test]
    fn test_delete_repository_cascades() {
        // Asserts FK ON DELETE CASCADE behavior at repository level.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        let db = AppDb::new(path).unwrap();
        let project = db.create_project("p1", None, "tool").unwrap();
        let repo = db.insert_local_repository("/tmp/r1", "r1", Some(project.id), None).unwrap();

        let conn = db.conn.lock().unwrap();
        // Insert a child row in deploy_environments
        conn.execute(
            "INSERT INTO deploy_environments (repository_id, name, workflow_name, image_tag,
             compose_service, domain, deploy_branch, extras)
             VALUES (?1, 'prod', 'Deploy', 'latest', 'svc', 'x.com', 'master', '{}')",
            rusqlite::params![repo.id],
        ).unwrap();
        drop(conn);

        db.delete_repository(repo.id).unwrap();

        let conn = db.conn.lock().unwrap();
        let remaining: i64 = conn.query_row(
            "SELECT COUNT(*) FROM deploy_environments WHERE repository_id = ?1",
            rusqlite::params![repo.id],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(remaining, 0);
        drop(conn);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_upsert_repository_update() {
        let db = make_db();
        let _ = db
            .upsert_repository("owner/repo", None, Some("Old"), None, None, None)
            .unwrap();
        // Second upsert with same name updates.
        let r2 = db
            .upsert_repository("owner/repo", None, Some("New"), Some("Go"), None, None)
            .unwrap();
        assert_eq!(r2.description.as_deref(), Some("New"));
        assert_eq!(r2.language.as_deref(), Some("Go"));
        let repos = db.list_all_repos().unwrap();
        assert_eq!(repos.len(), 1);
    }

    #[test]
    fn test_upsert_repository_rename_logs_repo_rename() {
        // F-033: when github_id matches but github_name changes, write a repo_renames row.
        let db = make_db();
        let r = db
            .upsert_repository("owner/old-name", None, None, None, None, Some(42))
            .unwrap();
        let _r2 = db
            .upsert_repository("owner/new-name", None, None, None, None, Some(42))
            .unwrap();
        let renames = db.list_renames_for_repo(r.id).unwrap();
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0].old_canonical, "old-name");
        assert_eq!(renames[0].new_canonical, "new-name");
    }

    #[test]
    fn test_upsert_repository_no_rename_when_canonical_same() {
        // F-033: changing the owner portion only must NOT log a rename
        // (canonical = last segment).
        let db = make_db();
        let _ = db
            .upsert_repository("alice/repo", None, None, None, None, Some(7))
            .unwrap();
        let r2 = db
            .upsert_repository("bob/repo", None, None, None, None, Some(7))
            .unwrap();
        let renames = db.list_renames_for_repo(r2.id).unwrap();
        assert!(renames.is_empty());
    }

    #[test]
    fn test_assign_repo_to_project() {
        let db = make_db();
        let proj = db.create_project("My App", None, "standard").unwrap();
        let repo = db
            .upsert_repository("owner/api", None, None, None, None, None)
            .unwrap();
        let updated = db
            .assign_repository(repo.id, Some(proj.id), Some("server"))
            .unwrap();
        assert_eq!(updated.project_id, Some(proj.id));
        assert_eq!(updated.role.as_deref(), Some("server"));
    }

    #[test]
    fn test_list_repos_by_project() {
        let db = make_db();
        let proj = db.create_project("App", None, "standard").unwrap();
        let r1 = db
            .upsert_repository("owner/a", None, None, None, None, None)
            .unwrap();
        let r2 = db
            .upsert_repository("owner/b", None, None, None, None, None)
            .unwrap();
        let _r3 = db
            .upsert_repository("owner/c", None, None, None, None, None)
            .unwrap();
        db.assign_repository(r1.id, Some(proj.id), Some("server"))
            .unwrap();
        db.assign_repository(r2.id, Some(proj.id), Some("client"))
            .unwrap();
        // r3 stays unassigned

        let in_proj = db.list_repos_by_project(Some(proj.id)).unwrap();
        assert_eq!(in_proj.len(), 2);
        let unassigned = db.list_repos_by_project(None).unwrap();
        assert_eq!(unassigned.len(), 1);
        assert_eq!(unassigned[0].github_name.as_deref(), Some("owner/c"));
    }

    #[test]
    fn test_list_all_repos() {
        let db = make_db();
        db.upsert_repository("owner/a", None, None, None, None, None)
            .unwrap();
        db.upsert_repository("owner/b", None, None, None, None, None)
            .unwrap();
        let all = db.list_all_repos().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_delete_project_sets_repos_to_null() {
        let db = make_db();
        let proj = db.create_project("App", None, "standard").unwrap();
        let r = db
            .upsert_repository("owner/api", None, None, None, None, None)
            .unwrap();
        db.assign_repository(r.id, Some(proj.id), Some("server"))
            .unwrap();
        db.delete_project(proj.id).unwrap();
        let r2 = db
            .upsert_repository("owner/api", None, None, None, None, None)
            .unwrap();
        // After project deletion, repo's project_id should be NULL
        assert_eq!(r2.project_id, None);
    }

    // ── B-007 merge local-only ↔ GitHub sync ──────────────────────────────────

    fn make_repo(db: &AppDb) -> i64 {
        let r = db
            .upsert_repository("owner/x", None, None, None, None, None)
            .unwrap();
        r.id
    }

    #[test]
    fn test_b007_merges_single_local_only_by_basename() {
        let db = make_db();
        // Seed a local-only row whose path basename matches the incoming repo name.
        let local = db
            .insert_local_repository("/tmp/my-app", "my-app local", None, None)
            .unwrap();
        assert_eq!(local.github_name, None);

        let outcome = db
            .upsert_repository_with_outcome(
                "owner/my-app",
                Some("https://github.com/owner/my-app"),
                Some("From GitHub"),
                Some("Rust"),
                None,
                Some(99),
            )
            .unwrap();
        // Expect Merged outcome with the local id
        match outcome {
            UpsertRepoOutcome::Merged {
                repo,
                merged_with_local_id,
                local_path,
            } => {
                assert_eq!(repo.id, local.id);
                assert_eq!(merged_with_local_id, local.id);
                assert_eq!(local_path, "/tmp/my-app");
                assert_eq!(repo.github_name.as_deref(), Some("owner/my-app"));
                assert_eq!(repo.local_path.as_deref(), Some("/tmp/my-app"));
                assert_eq!(repo.description.as_deref(), Some("From GitHub"));
            }
            other => panic!("Expected Merged, got: {:?}", other),
        }

        // No duplicate created
        let total = db.list_all_repos().unwrap().len();
        assert_eq!(total, 1);
    }

    #[test]
    fn test_b007_ambiguous_when_multiple_local_match() {
        let db = make_db();
        // Two local rows with same basename
        let a = db.insert_local_repository("/a/my-app", "First", None, None).unwrap();
        let b = db.insert_local_repository("/b/my-app", "Second", None, None).unwrap();

        let outcome = db
            .upsert_repository_with_outcome(
                "owner/my-app",
                None,
                Some("From GH"),
                None,
                None,
                None,
            )
            .unwrap();

        match outcome {
            UpsertRepoOutcome::Ambiguous {
                github_name,
                candidates,
                ..
            } => {
                assert_eq!(github_name, "owner/my-app");
                let ids: Vec<i64> = candidates.iter().map(|r| r.id).collect();
                assert!(ids.contains(&a.id));
                assert!(ids.contains(&b.id));
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("Expected Ambiguous, got: {:?}", other),
        }
        // Nothing inserted yet
        let total = db.list_all_repos().unwrap().len();
        assert_eq!(total, 2);
    }

    #[test]
    fn test_b007_inserts_when_no_local_match() {
        let db = make_db();
        // Local-only with different basename
        let local = db.insert_local_repository("/tmp/something-else", "X", None, None).unwrap();

        let outcome = db
            .upsert_repository_with_outcome(
                "owner/my-app",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        match outcome {
            UpsertRepoOutcome::Inserted { repo } => {
                assert_eq!(repo.github_name.as_deref(), Some("owner/my-app"));
                assert_ne!(repo.id, local.id);
            }
            other => panic!("Expected Inserted, got: {:?}", other),
        }
        // Two rows now
        let total = db.list_all_repos().unwrap().len();
        assert_eq!(total, 2);
    }

    #[test]
    fn test_b007_basename_match_is_case_insensitive() {
        let db = make_db();
        let local = db.insert_local_repository("/tmp/My-App", "Local", None, None).unwrap();
        let outcome = db
            .upsert_repository_with_outcome(
                "owner/my-app",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        match outcome {
            UpsertRepoOutcome::Merged { repo, .. } => {
                assert_eq!(repo.id, local.id);
            }
            _ => panic!("Expected Merged (case-insensitive)"),
        }
    }

    #[test]
    fn test_b007_resolve_merge_with_local_updates_chosen() {
        let db = make_db();
        let a = db.insert_local_repository("/a/my-app", "First", None, None).unwrap();
        let _b = db.insert_local_repository("/b/my-app", "Second", None, None).unwrap();
        // User picked first candidate
        let resolved = db
            .resolve_merge_with_local(
                a.id,
                "owner/my-app",
                Some("https://x"),
                Some("desc"),
                Some("Rust"),
                None,
                Some(99),
            )
            .unwrap();
        assert_eq!(resolved.id, a.id);
        assert_eq!(resolved.github_name.as_deref(), Some("owner/my-app"));
        assert_eq!(resolved.local_path.as_deref(), Some("/a/my-app"));
        assert_eq!(resolved.github_id, Some(99));
        // Second local stays untouched
        let total = db.list_all_repos().unwrap().len();
        assert_eq!(total, 2);
    }

    #[test]
    fn test_b007_force_insert_creates_new_entry() {
        let db = make_db();
        let _a = db.insert_local_repository("/a/my-app", "First", None, None).unwrap();
        let _b = db.insert_local_repository("/b/my-app", "Second", None, None).unwrap();
        let new_repo = db
            .force_insert_github_repo(
                "owner/my-app",
                None,
                Some("From GH"),
                None,
                None,
                Some(77),
            )
            .unwrap();
        assert_eq!(new_repo.github_name.as_deref(), Some("owner/my-app"));
        let total = db.list_all_repos().unwrap().len();
        assert_eq!(total, 3);
    }

    // ── F-025 Manual ordering tests ───────────────────────────────────────────

    fn conn_sort_order_project(db: &AppDb, id: i64) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT sort_order FROM projects WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn conn_sort_order_repo(db: &AppDb, id: i64) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            "SELECT sort_order FROM repositories WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn test_f025_new_project_gets_min_minus_10() {
        let db = make_db();
        let p1 = db.create_project("A", None, "standard").unwrap();
        let p2 = db.create_project("B", None, "standard").unwrap();
        let p3 = db.create_project("C", None, "standard").unwrap();
        // Each new project must sort strictly above the previous ones.
        let o1 = conn_sort_order_project(&db, p1.id);
        let o2 = conn_sort_order_project(&db, p2.id);
        let o3 = conn_sort_order_project(&db, p3.id);
        assert!(o3 < o2 && o2 < o1, "newer project must have smaller sort_order; got {} < {} < {}", o3, o2, o1);
        // And list_projects returns them in order newest → oldest.
        let list = db.list_projects().unwrap();
        assert_eq!(list[0].id, p3.id);
        assert_eq!(list[1].id, p2.id);
        assert_eq!(list[2].id, p1.id);
    }

    #[test]
    fn test_f025_new_repo_gets_group_max_plus_10() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/a", "a", Some(p.id), None).unwrap();
        let r2 = db.insert_local_repository("/b", "b", Some(p.id), None).unwrap();
        let o1 = conn_sort_order_repo(&db, r1.id);
        let o2 = conn_sort_order_repo(&db, r2.id);
        assert!(o2 > o1, "second repo in group must sort after first");
    }

    #[test]
    fn test_f025_reorder_project_swaps_neighbors() {
        let db = make_db();
        let a = db.create_project("A", None, "standard").unwrap();
        let b = db.create_project("B", None, "standard").unwrap();
        let _c = db.create_project("C", None, "standard").unwrap();
        // Order is [C, B, A] (newest → oldest); move B down (to A position).
        db.reorder_project(b.id, "down").unwrap();
        let list = db.list_projects().unwrap();
        assert_eq!(list[1].id, a.id);
        assert_eq!(list[2].id, b.id);
    }

    #[test]
    fn test_f025_reorder_project_wrap_first_to_end() {
        let db = make_db();
        let a = db.create_project("A", None, "standard").unwrap();
        let b = db.create_project("B", None, "standard").unwrap();
        let c = db.create_project("C", None, "standard").unwrap();
        // Order [C, B, A] — C is first. ▲ on C should wrap to end.
        db.reorder_project(c.id, "up").unwrap();
        let list = db.list_projects().unwrap();
        assert_eq!(list[list.len() - 1].id, c.id, "C wraps to end");
        // And B/A keep relative order.
        assert_eq!(list[0].id, b.id);
        assert_eq!(list[1].id, a.id);
    }

    #[test]
    fn test_f025_reorder_project_wrap_last_to_start() {
        let db = make_db();
        let a = db.create_project("A", None, "standard").unwrap();
        db.create_project("B", None, "standard").unwrap();
        db.create_project("C", None, "standard").unwrap();
        // Order [C, B, A] — A is last. ▼ on A should wrap to start.
        db.reorder_project(a.id, "down").unwrap();
        let list = db.list_projects().unwrap();
        assert_eq!(list[0].id, a.id, "A wraps to start");
    }

    #[test]
    fn test_f025_reorder_repo_within_project() {
        let db = make_db();
        let p = db.create_project("p", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/a", "a", Some(p.id), Some("server")).unwrap();
        let r2 = db.insert_local_repository("/b", "b", Some(p.id), Some("client")).unwrap();
        let o1 = conn_sort_order_repo(&db, r1.id);
        let o2 = conn_sort_order_repo(&db, r2.id);
        db.reorder_repo(r2.id, "up").unwrap();
        // r1 and r2 swap orders
        assert_eq!(conn_sort_order_repo(&db, r2.id), o1);
        assert_eq!(conn_sort_order_repo(&db, r1.id), o2);
    }

    #[test]
    fn test_f025_rebalance_repo_group_sets_10_20_30() {
        let db = make_db();
        let p = db.create_project("p", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/a", "a", Some(p.id), None).unwrap();
        let r2 = db.insert_local_repository("/b", "b", Some(p.id), None).unwrap();
        let r3 = db.insert_local_repository("/c", "c", Some(p.id), None).unwrap();
        // Manual reorder: r3, r1, r2
        db.rebalance_repo_group(&[r3.id, r1.id, r2.id]).unwrap();
        assert_eq!(conn_sort_order_repo(&db, r3.id), 10);
        assert_eq!(conn_sort_order_repo(&db, r1.id), 20);
        assert_eq!(conn_sort_order_repo(&db, r2.id), 30);
    }

    #[test]
    fn test_f025_auto_sort_all_restores_role_formula() {
        let db = make_db();
        let p = db.create_project("p", None, "standard").unwrap();
        let server = db.insert_local_repository("/a", "z-server", Some(p.id), Some("server")).unwrap();
        let client = db.insert_local_repository("/b", "a-client", Some(p.id), Some("client")).unwrap();
        let tool = db.insert_local_repository("/c", "m-tool", Some(p.id), Some("tool")).unwrap();
        // Mess up orders
        db.rebalance_repo_group(&[client.id, server.id, tool.id]).unwrap();
        db.auto_sort_all().unwrap();
        // server (role 0) < client (role 2) < tool (role 6)
        let server_o = conn_sort_order_repo(&db, server.id);
        let client_o = conn_sort_order_repo(&db, client.id);
        let tool_o = conn_sort_order_repo(&db, tool.id);
        assert!(server_o < client_o);
        assert!(client_o < tool_o);
        // Server's group_priority * 1000 = 0, so server gets ≈ 10-20
        assert!(server_o >= 10 && server_o < 1000);
        // Client gets ≈ 2010-3000 (role 2)
        assert!(client_o >= 2000 && client_o < 3000);
    }

    #[test]
    fn test_f025_auto_sort_all_alphabetical_within_same_role() {
        let db = make_db();
        let p = db.create_project("p", None, "standard").unwrap();
        let z = db.insert_local_repository("/a", "zebra", Some(p.id), Some("client")).unwrap();
        let a = db.insert_local_repository("/b", "alpha", Some(p.id), Some("client")).unwrap();
        let m = db.insert_local_repository("/c", "milk", Some(p.id), Some("client")).unwrap();
        db.auto_sort_all().unwrap();
        // Same role → alphabetical by description (which doubles as github_name fallback)
        let a_o = conn_sort_order_repo(&db, a.id);
        let m_o = conn_sort_order_repo(&db, m.id);
        let z_o = conn_sort_order_repo(&db, z.id);
        assert!(a_o < m_o);
        assert!(m_o < z_o);
    }

    #[test]
    fn test_f025_auto_sort_all_alphabetical_projects() {
        let db = make_db();
        let z = db.create_project("zebra", None, "standard").unwrap();
        let a = db.create_project("alpha", None, "standard").unwrap();
        let m = db.create_project("milk", None, "standard").unwrap();
        db.auto_sort_all().unwrap();
        assert_eq!(conn_sort_order_project(&db, a.id), 10);
        assert_eq!(conn_sort_order_project(&db, m.id), 20);
        assert_eq!(conn_sort_order_project(&db, z.id), 30);
    }

    #[test]
    fn test_f025_cross_project_move_lands_at_group_end() {
        let db = make_db();
        let p1 = db.create_project("p1", None, "standard").unwrap();
        let p2 = db.create_project("p2", None, "standard").unwrap();
        let _r1 = db.insert_local_repository("/a", "a", Some(p2.id), Some("server")).unwrap();
        let _r2 = db.insert_local_repository("/b", "b", Some(p2.id), Some("client")).unwrap();
        // r3 starts in p1, then moves to p2
        let r3 = db.insert_local_repository("/c", "c", Some(p1.id), Some("tool")).unwrap();
        let r3_before = conn_sort_order_repo(&db, r3.id);
        db.assign_repository(r3.id, Some(p2.id), Some("tool")).unwrap();
        // r3 should now have sort_order > all p2 originals
        let r3_after = conn_sort_order_repo(&db, r3.id);
        assert!(r3_after > r3_before);
        // r3 should be the last in p2 group
        let in_p2 = db.list_repos_by_project(Some(p2.id)).unwrap();
        assert_eq!(in_p2.last().unwrap().id, r3.id);
    }

    // ── set_repo_local_path test ──────────────────────────────────────────────

    #[test]
    fn test_set_repo_local_path() {
        let db = make_db();
        let r = db
            .upsert_repository("owner/repo", None, None, None, None, None)
            .unwrap();
        assert_eq!(r.local_path, None);

        // Set a path
        let updated = db
            .set_repo_local_path(r.id, Some("/home/user/projects/repo"))
            .unwrap();
        assert_eq!(
            updated.local_path.as_deref(),
            Some("/home/user/projects/repo")
        );

        // Clear the path
        let cleared = db.set_repo_local_path(r.id, None).unwrap();
        assert_eq!(cleared.local_path, None);
    }

    // ── update_repo_description tests (rename log for local-only) ─────────────

    #[test]
    fn test_update_repo_description_logs_rename_for_local_only() {
        let db = make_db();
        let r = db.insert_local_repository("/tmp/x", "Old Name", None, None).unwrap();
        let _u = db.update_repo_description(r.id, "New Name").unwrap();
        let renames = db.list_renames_for_repo(r.id).unwrap();
        assert_eq!(renames.len(), 1);
        assert_eq!(renames[0].old_canonical, "Old Name");
        assert_eq!(renames[0].new_canonical, "New Name");
    }

    #[test]
    fn test_update_repo_description_no_rename_on_same_description() {
        let db = make_db();
        let r = db.insert_local_repository("/tmp/x", "Same", None, None).unwrap();
        db.update_repo_description(r.id, "Same").unwrap();
        let renames = db.list_renames_for_repo(r.id).unwrap();
        assert!(renames.is_empty());
    }

    #[test]
    fn test_update_repo_description_no_rename_for_github_repo() {
        // For GitHub repo, canonical comes from github_name — description change must NOT log rename.
        let db = make_db();
        let r = db
            .upsert_repository("owner/repo", None, Some("Old desc"), None, None, None)
            .unwrap();
        db.update_repo_description(r.id, "New desc").unwrap();
        let renames = db.list_renames_for_repo(r.id).unwrap();
        assert!(renames.is_empty());
    }

    // ── Cascade tests for delete_repo (used by bugs.rs migration tests too) ───

    #[test]
    fn test_delete_repo_cascades_bugs() {
        let db = make_db();
        let repo = db.insert_local_repository("/tmp/r", "r", None, None).unwrap();
        let bug = db.insert_bug(repo.id, 1, "2026-01-01T00:00:00Z", "x", "minor", "other", "created", 0, None, None).unwrap();
        let conn = db.conn.lock().unwrap();
        let before: i64 = conn.query_row("SELECT COUNT(*) FROM bugs", [], |r| r.get(0)).unwrap();
        assert_eq!(before, 1);
        drop(conn);
        db.delete_repository(repo.id).unwrap();
        let conn = db.conn.lock().unwrap();
        let after: i64 = conn.query_row("SELECT COUNT(*) FROM bugs", [], |r| r.get(0)).unwrap();
        assert_eq!(after, 0);
        let _ = bug;
    }

    // ── B-007 negative: github_name still UNIQUE for non-NULL ─────────────────

    #[test]
    fn test_multiple_local_repos_coexist() {
        let db = make_db();
        let _a = db.insert_local_repository("/tmp/a", "A", None, None).unwrap();
        let _b = db.insert_local_repository("/tmp/b", "B", None, None).unwrap();
        let count: i64 = {
            let conn = db.conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM repositories WHERE github_name IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 2);
    }

    #[test]
    fn test_github_name_unique_still_enforced_for_non_null() {
        let db = make_db();
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO repositories (github_name) VALUES ('owner/repo')", []).unwrap();
        let result = conn.execute(
            "INSERT INTO repositories (github_name) VALUES ('owner/repo')",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_insert_local_repository_null_github_name() {
        let db = make_db();
        let repo = db
            .insert_local_repository("/tmp/my-local", "My Local Folder", None, None)
            .unwrap();
        assert_eq!(repo.github_name, None);
        assert_eq!(repo.description, Some("My Local Folder".to_string()));
        assert_eq!(repo.local_path, Some("/tmp/my-local".to_string()));
        assert_eq!(repo.project_id, None);
    }

    #[test]
    fn test_insert_local_repository_with_project() {
        let db = make_db();
        let proj = db.create_project("Test", None, "standard").unwrap();
        let repo = db
            .insert_local_repository("/tmp/assigned", "Assigned Folder", Some(proj.id), Some("server"))
            .unwrap();
        assert_eq!(repo.project_id, Some(proj.id));
        assert_eq!(repo.role, Some("server".to_string()));
    }
}
