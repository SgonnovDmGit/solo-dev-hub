// T-000094: deploy environments + deploy secrets + ensure_deploy_secrets_populated.
// Moved from db.rs.

use super::*;

impl AppDb {
    // ── set_deploy_target (repositories.deploy_target column) ─────────────────

    pub fn set_deploy_target(&self, repo_id: i64, target: Option<&str>) -> SqlResult<Repository> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE repositories SET deploy_target = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2",
            rusqlite::params![target, repo_id],
        )?;
        conn.query_row(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target FROM repositories WHERE id = ?1",
            rusqlite::params![repo_id],
            row_to_repo,
        )
    }

    // ── Deploy environments CRUD (v0.18.0) ────────────────────────────────────

    fn row_to_deploy_env(row: &rusqlite::Row) -> SqlResult<DeployEnvironment> {
        let extras_json: String = row.get::<_, String>(9).unwrap_or_else(|_| "{}".to_string());
        let extras = serde_json::from_str::<std::collections::HashMap<String, String>>(&extras_json)
            .unwrap_or_default();
        Ok(DeployEnvironment {
            id: row.get(0)?,
            repository_id: row.get(1)?,
            name: row.get(2)?,
            workflow_name: row.get(3)?,
            image_tag: row.get(4)?,
            compose_service: row.get(5)?,
            domain: row.get(6)?,
            deploy_branch: row.get(7)?,
            sort_order: row.get(8)?,
            extras,
            updated_at: row.get(10)?,
        })
    }

    const DEPLOY_ENV_COLS: &'static str =
        "id, repository_id, name, workflow_name, image_tag, compose_service, \
         domain, deploy_branch, sort_order, extras, updated_at";

    pub fn list_deploy_environments(&self, repo_id: i64) -> SqlResult<Vec<DeployEnvironment>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM deploy_environments WHERE repository_id = ?1 \
             ORDER BY sort_order ASC, name ASC",
            Self::DEPLOY_ENV_COLS,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows: Vec<DeployEnvironment> = stmt
            .query_map(rusqlite::params![repo_id], Self::row_to_deploy_env)?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn get_deploy_environment(&self, id: i64) -> SqlResult<Option<DeployEnvironment>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM deploy_environments WHERE id = ?1",
            Self::DEPLOY_ENV_COLS,
        );
        match conn.query_row(&sql, rusqlite::params![id], Self::row_to_deploy_env) {
            Ok(e) => Ok(Some(e)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn insert_deploy_environment(
        &self,
        args: &CreateDeployEnvironmentArgs,
    ) -> SqlResult<DeployEnvironment> {
        let conn = self.conn.lock().unwrap();
        let extras_json = serde_json::to_string(&args.extras)
            .unwrap_or_else(|_| "{}".to_string());
        // Compute next sort_order as max(existing) + 1
        let next_sort: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM deploy_environments WHERE repository_id = ?1",
            rusqlite::params![args.repository_id],
            |r| r.get(0),
        )?;
        conn.execute(
            "INSERT INTO deploy_environments
             (repository_id, name, workflow_name, image_tag, compose_service,
              domain, deploy_branch, sort_order, extras, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP)",
            rusqlite::params![
                args.repository_id, args.name, args.workflow_name, args.image_tag,
                args.compose_service, args.domain, args.deploy_branch,
                next_sort, extras_json,
            ],
        )?;
        let id = conn.last_insert_rowid();
        drop(conn);
        Ok(self.get_deploy_environment(id)?.expect("just inserted"))
    }

    pub fn update_deploy_environment(
        &self,
        args: &UpdateDeployEnvironmentArgs,
    ) -> SqlResult<DeployEnvironment> {
        let conn = self.conn.lock().unwrap();
        let extras_json = serde_json::to_string(&args.extras)
            .unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "UPDATE deploy_environments SET
                workflow_name = ?2,
                image_tag = ?3,
                compose_service = ?4,
                domain = ?5,
                deploy_branch = ?6,
                extras = ?7,
                updated_at = CURRENT_TIMESTAMP
             WHERE id = ?1",
            rusqlite::params![
                args.id, args.workflow_name, args.image_tag,
                args.compose_service, args.domain, args.deploy_branch, extras_json,
            ],
        )?;
        drop(conn);
        Ok(self.get_deploy_environment(args.id)?.expect("update target must exist"))
    }

    pub fn delete_deploy_environment(&self, id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM deploy_environments WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    pub fn reorder_deploy_environments(&self, repo_id: i64, ordered_ids: &[i64]) -> SqlResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        for (idx, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE deploy_environments SET sort_order = ?1 \
                 WHERE id = ?2 AND repository_id = ?3",
                rusqlite::params![idx as i64, id, repo_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ── Deploy secrets CRUD (v0.18.0) ─────────────────────────────────────────

    fn row_to_deploy_secret(row: &rusqlite::Row) -> SqlResult<DeploySecret> {
        Ok(DeploySecret {
            id: row.get(0)?,
            deploy_env_id: row.get(1)?,
            secret_name: row.get(2)?,
            role: row.get::<_, Option<String>>(3)?,
            included: row.get::<_, i64>(4)? != 0,
            override_enabled: row.get::<_, i64>(5)? != 0,
            sort_order: row.get(6)?,
        })
    }

    pub fn list_deploy_secrets(&self, deploy_env_id: i64) -> SqlResult<Vec<DeploySecret>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, deploy_env_id, secret_name, role, included, override_enabled, sort_order
             FROM deploy_secrets WHERE deploy_env_id = ?1
             ORDER BY secret_name ASC",
        )?;
        let rows: Vec<DeploySecret> = stmt
            .query_map(rusqlite::params![deploy_env_id], Self::row_to_deploy_secret)?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    pub fn upsert_deploy_secret(
        &self,
        deploy_env_id: i64,
        secret_name: &str,
        role: Option<&str>,
        included: bool,
        override_enabled: bool,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO deploy_secrets (deploy_env_id, secret_name, role, included, override_enabled)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(deploy_env_id, secret_name) DO UPDATE SET
                role = excluded.role,
                included = excluded.included,
                override_enabled = excluded.override_enabled",
            rusqlite::params![
                deploy_env_id, secret_name, role,
                if included { 1 } else { 0 },
                if override_enabled { 1 } else { 0 },
            ],
        )?;
        Ok(())
    }

    pub fn delete_deploy_secret(&self, deploy_env_id: i64, secret_name: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM deploy_secrets WHERE deploy_env_id = ?1 AND secret_name = ?2",
            rusqlite::params![deploy_env_id, secret_name],
        )?;
        Ok(())
    }

    pub fn clone_deploy_environment(
        &self,
        source_id: i64,
        new_name: &str,
    ) -> SqlResult<DeployEnvironment> {
        let src = self.get_deploy_environment(source_id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        let args = CreateDeployEnvironmentArgs {
            repository_id: src.repository_id,
            name: new_name.to_string(),
            workflow_name: src.workflow_name,
            image_tag: src.image_tag,
            compose_service: src.compose_service,
            domain: src.domain,
            deploy_branch: src.deploy_branch,
            extras: src.extras,
        };
        let cloned = self.insert_deploy_environment(&args)?;

        // Copy deploy_secrets flags (without values — values live in GitHub only).
        let src_secrets = self.list_deploy_secrets(source_id)?;
        for s in &src_secrets {
            self.upsert_deploy_secret(
                cloned.id,
                &s.secret_name,
                s.role.as_deref(),
                s.included,
                s.override_enabled,
            )?;
        }
        Ok(cloned)
    }

    /// v0.18.0: seed deploy_secrets rows for a newly-opened deploy env.
    /// Union of `repo_secret_names` (what user actually has in GitHub Secrets) and
    /// `meta_hints` (what template declares). For each name that has no DB row yet:
    ///   - role  = meta_hints.role if present, else "runtime"
    ///     (v0.29.2: changed from "deploy" → "runtime". Rationale: meta_hints
    ///     already covers deploy-infrastructure secrets (SSH_*, NPM_*) and any
    ///     build-time needs (Flutter API_BASE_URL etc) explicitly. Whatever the
    ///     user adds outside hints is almost always app config — DB creds, API
    ///     keys, etc — which belongs in runtime env-vars. Existing rows are
    ///     untouched; users can manually cycle via the role chip in DeployTable.)
    ///   - override_enabled = (meta_hints.scope == "environment")
    ///   - included = true
    /// Existing rows are untouched (idempotent).
    pub fn ensure_deploy_secrets_populated(
        &self,
        deploy_env_id: i64,
        repo_secret_names: &[String],
        meta_hints: &[MetaSecretHint],
    ) -> SqlResult<()> {
        use std::collections::{HashMap, HashSet};
        let hints_by_name: HashMap<&str, &MetaSecretHint> =
            meta_hints.iter().map(|h| (h.name.as_str(), h)).collect();
        let all_names: HashSet<&str> = repo_secret_names.iter().map(|s| s.as_str())
            .chain(meta_hints.iter().map(|h| h.name.as_str()))
            .collect();

        let conn = self.conn.lock().unwrap();
        let existing: HashSet<String> = {
            let mut stmt = conn.prepare(
                "SELECT secret_name FROM deploy_secrets WHERE deploy_env_id = ?1",
            )?;
            let x: HashSet<String> = stmt.query_map(rusqlite::params![deploy_env_id], |r| r.get::<_, String>(0))?
                .filter_map(Result::ok)
                .collect();
            x
        };

        for name in &all_names {
            if existing.contains(*name) {
                continue;
            }
            let (role, override_enabled) = match hints_by_name.get(name) {
                Some(h) => (h.role.as_str(), h.scope == "environment"),
                None => ("runtime", false),
            };
            conn.execute(
                "INSERT INTO deploy_secrets (deploy_env_id, secret_name, role, included, override_enabled)
                 VALUES (?1, ?2, ?3, 1, ?4)",
                rusqlite::params![
                    deploy_env_id, name, role,
                    if override_enabled { 1 } else { 0 },
                ],
            )?;
        }

        // Prune orphans: rows whose secret_name is in NEITHER current GitHub repo
        // secrets NOR meta.json required_secrets. Happens after a template
        // updates (e.g. CONTAINER_NAME removed in v0.25.0) or after the user
        // deletes a repo-level secret in GitHub. Caller must only invoke this
        // function with a fresh `repo_secret_names` from a successful list call
        // — empty-due-to-failure would falsely prune legitimate rows.
        for orphan in existing.iter().filter(|n| !all_names.contains(n.as_str())) {
            conn.execute(
                "DELETE FROM deploy_secrets WHERE deploy_env_id = ?1 AND secret_name = ?2",
                rusqlite::params![deploy_env_id, orphan],
            )?;
        }
        Ok(())
    }

    /// v0.18.0: sync-trigger called after a new repo-level GitHub secret is successfully
    /// PUT. Adds a deploy_secrets row (included=1, role='deploy', override_enabled=0)
    /// for every existing deploy_environments of this repo. Idempotent via INSERT OR IGNORE.
    pub fn register_repo_secret_in_deploys(&self, repo_id: i64, secret_name: &str) -> SqlResult<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let env_ids: Vec<i64> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM deploy_environments WHERE repository_id = ?1",
            )?;
            let x: Vec<i64> = stmt.query_map(rusqlite::params![repo_id], |r| r.get::<_, i64>(0))?
                .filter_map(Result::ok)
                .collect();
            x
        };
        for env_id in env_ids {
            tx.execute(
                "INSERT OR IGNORE INTO deploy_secrets
                 (deploy_env_id, secret_name, role, included, override_enabled)
                 VALUES (?1, ?2, 'deploy', 1, 0)",
                rusqlite::params![env_id, secret_name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db() -> AppDb {
        AppDb::new(std::path::PathBuf::from(":memory:")).unwrap()
    }

    fn seed_repo_for_deploy_tests(db: &AppDb) -> (i64, i64) {
        let p = db.create_project("p1", None, "tool").unwrap();
        let r = db.insert_local_repository("/tmp/r1", "r1", Some(p.id), None).unwrap();
        (p.id, r.id)
    }

    #[test]
    fn test_deploy_target_set_and_clear() {
        let db = make_db();
        let r = db
            .upsert_repository("owner/repo", None, None, None, None, None)
            .unwrap();
        assert!(r.deploy_target.is_none());

        let r2 = db.set_deploy_target(r.id, Some("flutter_web")).unwrap();
        assert_eq!(r2.deploy_target.as_deref(), Some("flutter_web"));

        let r3 = db.set_deploy_target(r.id, None).unwrap();
        assert!(r3.deploy_target.is_none());
    }

    #[test]
    fn test_insert_and_list_deploy_environments() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);

        let args = CreateDeployEnvironmentArgs {
            repository_id: r,
            name: "prod".to_string(),
            workflow_name: "Deploy Backend".to_string(),
            image_tag: "latest".to_string(),
            compose_service: "backend".to_string(),
            domain: "x.com".to_string(),
            deploy_branch: "master".to_string(),
            extras: Default::default(),
        };
        let env = db.insert_deploy_environment(&args).unwrap();
        assert_eq!(env.name, "prod");
        assert!(env.id > 0);

        let list = db.list_deploy_environments(r).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "prod");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_insert_deploy_environment_unique_name_per_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);

        let args = CreateDeployEnvironmentArgs {
            repository_id: r,
            name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        };
        db.insert_deploy_environment(&args).unwrap();
        let err = db.insert_deploy_environment(&args).unwrap_err();
        assert!(err.to_string().contains("UNIQUE"), "got: {}", err);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_update_deploy_environment_mutates_placeholders_not_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);

        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "old".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "old.com".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        let mut extras = std::collections::HashMap::new();
        extras.insert("APP_PORT".to_string(), "8080".to_string());
        db.update_deploy_environment(&UpdateDeployEnvironmentArgs {
            id: env.id,
            workflow_name: "new".to_string(),
            image_tag: "prod".to_string(),
            compose_service: "svc".to_string(),
            domain: "new.com".to_string(),
            deploy_branch: "main".to_string(),
            extras,
        }).unwrap();

        let updated = db.get_deploy_environment(env.id).unwrap().unwrap();
        assert_eq!(updated.name, "prod", "name MUST remain unchanged");
        assert_eq!(updated.workflow_name, "new");
        assert_eq!(updated.domain, "new.com");
        assert_eq!(updated.extras.get("APP_PORT"), Some(&"8080".to_string()));
        std::mem::forget(tmp);
    }

    #[test]
    fn test_delete_deploy_environment() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);

        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "test".to_string(),
            workflow_name: "W".to_string(), image_tag: "t".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        db.delete_deploy_environment(env.id).unwrap();
        assert!(db.get_deploy_environment(env.id).unwrap().is_none());
        std::mem::forget(tmp);
    }

    #[test]
    fn test_reorder_deploy_environments() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);

        let mk = |name: &str| CreateDeployEnvironmentArgs {
            repository_id: r, name: name.to_string(),
            workflow_name: "W".to_string(), image_tag: "t".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        };
        let a = db.insert_deploy_environment(&mk("prod")).unwrap();
        let b = db.insert_deploy_environment(&mk("test")).unwrap();
        let c = db.insert_deploy_environment(&mk("stg")).unwrap();

        db.reorder_deploy_environments(r, &[c.id, a.id, b.id]).unwrap();
        let list = db.list_deploy_environments(r).unwrap();
        let names: Vec<_> = list.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, vec!["stg", "prod", "test"]);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_upsert_and_list_deploy_secrets() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        db.upsert_deploy_secret(env.id, "SSH_HOST", Some("deploy"), true, true).unwrap();
        db.upsert_deploy_secret(env.id, "NPM_EMAIL", Some("deploy"), true, false).unwrap();
        db.upsert_deploy_secret(env.id, "UNUSED", None, false, false).unwrap();

        let secrets = db.list_deploy_secrets(env.id).unwrap();
        assert_eq!(secrets.len(), 3);
        let by_name: std::collections::HashMap<_, _> = secrets.iter()
            .map(|s| (s.secret_name.clone(), s.clone())).collect();
        assert_eq!(by_name["SSH_HOST"].role, Some("deploy".to_string()));
        assert!(by_name["SSH_HOST"].included);
        assert!(by_name["SSH_HOST"].override_enabled);
        assert!(by_name["UNUSED"].role.is_none());
        assert!(!by_name["UNUSED"].included);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_upsert_deploy_secret_is_update_on_conflict() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        db.upsert_deploy_secret(env.id, "X", Some("build"), true, false).unwrap();
        db.upsert_deploy_secret(env.id, "X", Some("runtime"), true, true).unwrap();

        let secrets = db.list_deploy_secrets(env.id).unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].role, Some("runtime".to_string()));
        assert!(secrets[0].override_enabled);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_delete_deploy_secret() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();
        db.upsert_deploy_secret(env.id, "A", Some("deploy"), true, false).unwrap();
        db.upsert_deploy_secret(env.id, "B", Some("deploy"), true, false).unwrap();

        db.delete_deploy_secret(env.id, "A").unwrap();
        let list = db.list_deploy_secrets(env.id).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].secret_name, "B");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_deploy_secrets_cascade_on_env_delete() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();
        db.upsert_deploy_secret(env.id, "X", Some("deploy"), true, false).unwrap();

        db.delete_deploy_environment(env.id).unwrap();
        let list = db.list_deploy_secrets(env.id).unwrap();
        assert!(list.is_empty());
        std::mem::forget(tmp);
    }

    #[test]
    fn test_clone_deploy_environment_copies_placeholders_and_secrets_flags() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);

        let mut extras = std::collections::HashMap::new();
        extras.insert("APP_PORT".to_string(), "8080".to_string());
        extras.insert("NETWORK_NAME".to_string(), "goapp_prod_net".to_string());
        let src = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "Prod Deploy".to_string(), image_tag: "prod".to_string(),
            compose_service: "backend".to_string(), domain: "x.com".to_string(),
            deploy_branch: "master".to_string(), extras,
        }).unwrap();
        db.upsert_deploy_secret(src.id, "SSH_HOST", Some("deploy"), true, true).unwrap();
        db.upsert_deploy_secret(src.id, "NPM_EMAIL", Some("deploy"), true, false).unwrap();
        db.upsert_deploy_secret(src.id, "EXCLUDED", None, false, false).unwrap();

        let cloned = db.clone_deploy_environment(src.id, "test").unwrap();
        assert_eq!(cloned.name, "test");
        assert_eq!(cloned.repository_id, r);
        assert_eq!(cloned.workflow_name, "Prod Deploy");
        assert_eq!(cloned.extras.get("APP_PORT"), Some(&"8080".to_string()));
        assert_eq!(cloned.extras.get("NETWORK_NAME"), Some(&"goapp_prod_net".to_string()));
        assert_ne!(cloned.id, src.id);

        let secrets = db.list_deploy_secrets(cloned.id).unwrap();
        assert_eq!(secrets.len(), 3);
        let by_name: std::collections::HashMap<_, _> = secrets.iter()
            .map(|s| (s.secret_name.clone(), s.clone())).collect();
        assert!(by_name["SSH_HOST"].included);
        assert!(by_name["SSH_HOST"].override_enabled, "override_enabled flag preserved");
        assert!(!by_name["EXCLUDED"].included);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_clone_deploy_environment_name_collision_fails() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let src = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        let err = db.clone_deploy_environment(src.id, "prod").unwrap_err();
        assert!(err.to_string().contains("UNIQUE"), "got: {}", err);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_ensure_deploy_secrets_populated_inserts_union_with_hints() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        let repo_secret_names = vec!["SSH_HOST".to_string(), "NPM_EMAIL".to_string()];
        let meta_hints = vec![
            MetaSecretHint { name: "SSH_HOST".to_string(), role: "deploy".to_string(), scope: "environment".to_string() },
            MetaSecretHint { name: "API_BASE_URL".to_string(), role: "build".to_string(), scope: "environment".to_string() },
            MetaSecretHint { name: "NPM_EMAIL".to_string(), role: "deploy".to_string(), scope: "repo".to_string() },
        ];

        db.ensure_deploy_secrets_populated(env.id, &repo_secret_names, &meta_hints).unwrap();

        let secrets = db.list_deploy_secrets(env.id).unwrap();
        let by_name: std::collections::HashMap<_, _> = secrets.iter()
            .map(|s| (s.secret_name.clone(), s.clone())).collect();
        assert_eq!(secrets.len(), 3);
        assert_eq!(by_name["SSH_HOST"].role, Some("deploy".to_string()));
        assert!(by_name["SSH_HOST"].override_enabled);
        assert_eq!(by_name["API_BASE_URL"].role, Some("build".to_string()));
        assert!(by_name["API_BASE_URL"].included);
        assert!(by_name["API_BASE_URL"].override_enabled);
        assert!(!by_name["NPM_EMAIL"].override_enabled);
        assert_eq!(by_name["NPM_EMAIL"].role, Some("deploy".to_string()));
        std::mem::forget(tmp);
    }

    #[test]
    fn test_ensure_deploy_secrets_populated_defaults_unknown_to_runtime() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        let repo_secret_names = vec!["DB_HOST".to_string(), "MASTER_KEY".to_string()];
        let meta_hints: Vec<MetaSecretHint> = vec![];

        db.ensure_deploy_secrets_populated(env.id, &repo_secret_names, &meta_hints).unwrap();

        let secrets = db.list_deploy_secrets(env.id).unwrap();
        let by_name: std::collections::HashMap<_, _> = secrets.iter()
            .map(|s| (s.secret_name.clone(), s.clone())).collect();
        assert_eq!(by_name["DB_HOST"].role, Some("runtime".to_string()));
        assert_eq!(by_name["MASTER_KEY"].role, Some("runtime".to_string()));
        assert!(by_name["DB_HOST"].included);
        assert!(!by_name["DB_HOST"].override_enabled, "scope defaults to repo (override off)");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_ensure_deploy_secrets_populated_prunes_orphans() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();

        let old_hints = vec![
            MetaSecretHint { name: "SSH_HOST".to_string(), role: "deploy".to_string(), scope: "environment".to_string() },
            MetaSecretHint { name: "CONTAINER_NAME".to_string(), role: "deploy".to_string(), scope: "environment".to_string() },
        ];
        db.ensure_deploy_secrets_populated(env.id, &["SSH_HOST".to_string()], &old_hints).unwrap();
        assert_eq!(db.list_deploy_secrets(env.id).unwrap().len(), 2);

        let new_hints = vec![
            MetaSecretHint { name: "SSH_HOST".to_string(), role: "deploy".to_string(), scope: "environment".to_string() },
        ];
        db.ensure_deploy_secrets_populated(env.id, &["SSH_HOST".to_string()], &new_hints).unwrap();

        let secrets = db.list_deploy_secrets(env.id).unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].secret_name, "SSH_HOST");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_ensure_deploy_secrets_populated_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();
        let repo_secrets = vec!["X".to_string()];
        let hints = vec![];

        db.ensure_deploy_secrets_populated(env.id, &repo_secrets, &hints).unwrap();
        db.upsert_deploy_secret(env.id, "X", Some("runtime"), true, true).unwrap();
        db.ensure_deploy_secrets_populated(env.id, &repo_secrets, &hints).unwrap();

        let secrets = db.list_deploy_secrets(env.id).unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].role, Some("runtime".to_string()), "user edit preserved");
        assert!(secrets[0].override_enabled, "user edit preserved");
    }

    #[test]
    fn test_register_repo_secret_in_all_deploys() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let e1 = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();
        let e2 = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "test".to_string(),
            workflow_name: "W2".to_string(), image_tag: "t".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "dev".to_string(), extras: Default::default(),
        }).unwrap();

        db.register_repo_secret_in_deploys(r, "NEW_SECRET").unwrap();

        let s1 = db.list_deploy_secrets(e1.id).unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].secret_name, "NEW_SECRET");
        assert!(s1[0].included);
        assert_eq!(s1[0].role, Some("deploy".to_string()), "default role is 'deploy'");
        assert!(!s1[0].override_enabled);

        let s2 = db.list_deploy_secrets(e2.id).unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].secret_name, "NEW_SECRET");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_register_repo_secret_in_deploys_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        let (_p, r) = seed_repo_for_deploy_tests(&db);
        let env = db.insert_deploy_environment(&CreateDeployEnvironmentArgs {
            repository_id: r, name: "prod".to_string(),
            workflow_name: "W".to_string(), image_tag: "l".to_string(),
            compose_service: "s".to_string(), domain: "d".to_string(),
            deploy_branch: "m".to_string(), extras: Default::default(),
        }).unwrap();
        db.upsert_deploy_secret(env.id, "EXISTING", Some("runtime"), true, true).unwrap();

        db.register_repo_secret_in_deploys(r, "EXISTING").unwrap();

        let s = db.list_deploy_secrets(env.id).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].role, Some("runtime".to_string()), "existing role preserved");
        assert!(s[0].override_enabled, "existing override preserved");
        std::mem::forget(tmp);
    }
}
