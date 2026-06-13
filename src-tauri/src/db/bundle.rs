// v1.3.0: secret bundles — encrypted-at-rest reusable secret value sets.
use super::*;
use crate::crypto::bundle_cipher;
use crate::keyring_store;

impl AppDb {
    /// List all bundles with their item names (no values). Ordered by name.
    pub fn list_secret_bundles(&self) -> SqlResult<Vec<SecretBundle>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, created_at, updated_at
             FROM secret_bundles ORDER BY name COLLATE NOCASE",
        )?;
        let metas: Vec<(i64, String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?
            .filter_map(Result::ok)
            .collect();

        let mut out = Vec::with_capacity(metas.len());
        for (id, name, description, created_at, updated_at) in metas {
            let mut nstmt = conn.prepare(
                "SELECT secret_name FROM secret_bundle_items
                 WHERE bundle_id = ?1 ORDER BY secret_name COLLATE NOCASE",
            )?;
            let names: Vec<String> = nstmt
                .query_map(rusqlite::params![id], |r| r.get(0))?
                .filter_map(Result::ok)
                .collect();
            out.push(SecretBundle {
                id,
                name,
                description,
                created_at,
                updated_at,
                secret_names: names,
            });
        }
        Ok(out)
    }

    pub fn create_secret_bundle(&self, name: &str, description: &str) -> SqlResult<i64> {
        let now = utc_now_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO secret_bundles (name, description, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
            rusqlite::params![name, description, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn rename_secret_bundle(&self, id: i64, name: &str, description: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE secret_bundles SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![name, description, utc_now_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn delete_secret_bundle(&self, id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM secret_bundles WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    /// Encrypt `value` and upsert the item (insert or replace by UNIQUE name).
    pub fn upsert_bundle_item(&self, bundle_id: i64, secret_name: &str, value: &str) -> SqlResult<()> {
        let key = keyring_store::get_or_create_bundle_key()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
        let (ciphertext, nonce) = bundle_cipher::encrypt(&key, value.as_bytes())
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO secret_bundle_items (bundle_id, secret_name, ciphertext, nonce)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(bundle_id, secret_name)
             DO UPDATE SET ciphertext = excluded.ciphertext, nonce = excluded.nonce",
            rusqlite::params![bundle_id, secret_name, ciphertext, nonce],
        )?;
        conn.execute(
            "UPDATE secret_bundles SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![utc_now_rfc3339(), bundle_id],
        )?;
        Ok(())
    }

    pub fn delete_bundle_item(&self, item_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM secret_bundle_items WHERE id = ?1",
            rusqlite::params![item_id],
        )?;
        Ok(())
    }

    /// Decrypt all items of a bundle. Returns `[]` for an empty/missing bundle.
    pub fn get_bundle_decrypted(&self, bundle_id: i64) -> SqlResult<Vec<SecretBundleItemValue>> {
        let key = keyring_store::get_or_create_bundle_key()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, secret_name, ciphertext, nonce FROM secret_bundle_items
             WHERE bundle_id = ?1 ORDER BY secret_name COLLATE NOCASE",
        )?;
        let rows: Vec<(i64, String, Vec<u8>, Vec<u8>)> = stmt
            .query_map(rusqlite::params![bundle_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);

        let mut out = Vec::with_capacity(rows.len());
        for (id, secret_name, ct, nonce) in rows {
            let plain = bundle_cipher::decrypt(&key, &ct, &nonce)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(StringError(e))))?;
            out.push(SecretBundleItemValue {
                id,
                secret_name,
                value: String::from_utf8(plain)
                    .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
            });
        }
        Ok(out)
    }
}

/// Minimal error wrapper so crypto/keyring `String` errors can ride inside
/// `rusqlite::Error` without a new error enum.
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

    #[test]
    fn create_list_roundtrip_no_values() {
        let db = db();
        let id = db.create_secret_bundle("prod-server-1", "main app server").unwrap();
        db.upsert_bundle_item(id, "SSH_HOST", "1.2.3.4").unwrap();
        db.upsert_bundle_item(id, "DB_PASSWORD", "p@ss").unwrap();

        let bundles = db.list_secret_bundles().unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].name, "prod-server-1");
        assert_eq!(bundles[0].secret_names, vec!["DB_PASSWORD", "SSH_HOST"]);
    }

    #[test]
    fn decrypt_returns_values() {
        let db = db();
        let id = db.create_secret_bundle("b", "").unwrap();
        db.upsert_bundle_item(id, "SSH_HOST", "1.2.3.4").unwrap();
        let items = db.get_bundle_decrypted(id).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].secret_name, "SSH_HOST");
        assert_eq!(items[0].value, "1.2.3.4");
    }

    #[test]
    fn upsert_overwrites_value() {
        let db = db();
        let id = db.create_secret_bundle("b", "").unwrap();
        db.upsert_bundle_item(id, "K", "old").unwrap();
        db.upsert_bundle_item(id, "K", "new").unwrap();
        let items = db.get_bundle_decrypted(id).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "new");
    }

    #[test]
    fn delete_bundle_cascades_items() {
        let db = db();
        let id = db.create_secret_bundle("b", "").unwrap();
        db.upsert_bundle_item(id, "K", "v").unwrap();
        db.delete_secret_bundle(id).unwrap();
        assert!(db.list_secret_bundles().unwrap().is_empty());
        let conn = db.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM secret_bundle_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn duplicate_bundle_name_fails() {
        let db = db();
        db.create_secret_bundle("dup", "").unwrap();
        assert!(db.create_secret_bundle("dup", "").is_err());
    }

    #[test]
    fn empty_bundle_decrypts_to_empty() {
        let db = db();
        let id = db.create_secret_bundle("b", "").unwrap();
        assert!(db.get_bundle_decrypted(id).unwrap().is_empty());
    }
}
