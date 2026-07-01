// v1.6.0 (F-000043): encrypted-at-rest persisted deploy secret values.
// Mirrors `db/bundle.rs` — same keyring data key + AES-256-GCM cipher.
// Values are NEVER stored in plaintext, logged, or printed.
use super::*;
use crate::crypto::bundle_cipher;
use crate::keyring_store;

impl AppDb {
    /// Encrypt `value` and upsert it for `(deploy_env_id, secret_name)`.
    pub fn set_deploy_secret_value(
        &self,
        deploy_env_id: i64,
        secret_name: &str,
        value: &str,
    ) -> SqlResult<()> {
        let key = keyring_store::get_or_create_bundle_key()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
        let (ciphertext, nonce) = bundle_cipher::encrypt(&key, value.as_bytes())
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO deploy_secret_values (deploy_env_id, secret_name, ciphertext, nonce, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(deploy_env_id, secret_name)
             DO UPDATE SET ciphertext = excluded.ciphertext,
                           nonce = excluded.nonce,
                           updated_at = excluded.updated_at",
            rusqlite::params![deploy_env_id, secret_name, ciphertext, nonce, utc_now_rfc3339()],
        )?;
        Ok(())
    }

    /// Delete a persisted value for `(deploy_env_id, secret_name)`. No-op if absent.
    pub fn delete_deploy_secret_value(
        &self,
        deploy_env_id: i64,
        secret_name: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM deploy_secret_values WHERE deploy_env_id = ?1 AND secret_name = ?2",
            rusqlite::params![deploy_env_id, secret_name],
        )?;
        Ok(())
    }

    /// Decrypt all persisted values for a deploy env. Returns `[]` when none.
    pub fn get_deploy_secret_values(
        &self,
        deploy_env_id: i64,
    ) -> SqlResult<Vec<DeploySecretValue>> {
        let key = keyring_store::get_or_create_bundle_key()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT secret_name, ciphertext, nonce FROM deploy_secret_values
             WHERE deploy_env_id = ?1 ORDER BY secret_name COLLATE NOCASE",
        )?;
        let rows: Vec<(String, Vec<u8>, Vec<u8>)> = stmt
            .query_map(rusqlite::params![deploy_env_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);

        let mut out = Vec::with_capacity(rows.len());
        for (secret_name, ct, nonce) in rows {
            let plain = bundle_cipher::decrypt(&key, &ct, &nonce)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
            out.push(DeploySecretValue {
                secret_name,
                value: String::from_utf8(plain)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
            });
        }
        Ok(out)
    }
}

/// Minimal error wrapper so crypto/keyring `String` errors can ride inside
/// `rusqlite::Error` without a new error enum. (Local copy — the identical
/// helper in `db/bundle.rs` is private to that module.)
#[derive(Debug)]
struct StringError(String);
impl std::fmt::Display for StringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for StringError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    /// Create a repo + a single deploy environment, returning the env id.
    fn seed_env(db: &AppDb) -> i64 {
        let repo = db
            .insert_local_repository("/tmp/r1", "r1", None, None)
            .unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO deploy_environments (repository_id, name, workflow_name, image_tag,
             compose_service, domain, deploy_branch, extras)
             VALUES (?1, 'prod', 'Deploy', 'latest', 'backend', 'x.com', 'master', '{}')",
            rusqlite::params![repo.id],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn set_get_roundtrip() {
        let db = db();
        let env = seed_env(&db);
        db.set_deploy_secret_value(env, "SSH_HOST", "1.2.3.4")
            .unwrap();
        let vals = db.get_deploy_secret_values(env).unwrap();
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0].secret_name, "SSH_HOST");
        assert_eq!(vals[0].value, "1.2.3.4");
    }

    #[test]
    fn upsert_replaces_value_single_row() {
        let db = db();
        let env = seed_env(&db);
        db.set_deploy_secret_value(env, "K", "old").unwrap();
        db.set_deploy_secret_value(env, "K", "new").unwrap();
        let vals = db.get_deploy_secret_values(env).unwrap();
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0].value, "new");
        let conn = db.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM deploy_secret_values", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(n, 1, "upsert must not create a second row");
    }

    #[test]
    fn delete_removes_value() {
        let db = db();
        let env = seed_env(&db);
        db.set_deploy_secret_value(env, "K", "v").unwrap();
        db.delete_deploy_secret_value(env, "K").unwrap();
        assert!(db.get_deploy_secret_values(env).unwrap().is_empty());
    }

    #[test]
    fn deleting_parent_env_cascades_values() {
        let db = db();
        let env = seed_env(&db);
        db.set_deploy_secret_value(env, "K", "v").unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM deploy_environments WHERE id = ?1",
            rusqlite::params![env],
        )
        .unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM deploy_secret_values WHERE deploy_env_id = ?1",
                rusqlite::params![env],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 0,
            "deploy_secret_values must cascade-delete with the env"
        );
    }

    #[test]
    fn empty_env_returns_empty() {
        let db = db();
        let env = seed_env(&db);
        assert!(db.get_deploy_secret_values(env).unwrap().is_empty());
    }
}
