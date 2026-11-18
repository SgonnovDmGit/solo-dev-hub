// T-000257 (v1.24.5): the delivery ledger of the outbox→inbox mirror
// (migration v37). A prune may only remove a file THIS install copied —
// "absent from my local outbox" is not proof the author retracted it when
// the inbox copy can equally be another machine's delivery that arrived
// through git. The key is (src repo, dst repo, lane-relative path): repo ids
// survive a peer-folder rename and the path keeps its nested components, so
// `sub/REQ-001.md` and `REQ-001.md` never collide.
//
// The table is installation-local state and is never transported — each
// machine prunes only what its own ledger proves it delivered.

use super::*;
use rusqlite::OptionalExtension;

impl AppDb {
    /// Remember that this install delivered `rel_path` into `dst`'s inbox
    /// lane with content hash `sha` (sha256 of the delivered bytes).
    pub fn record_peer_delivery(
        &self,
        src_repo_id: i64,
        dst_repo_id: i64,
        rel_path: &str,
        blob_sha: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO peer_channel_deliveries \
                 (src_repo_id, dst_repo_id, rel_path, blob_sha, delivered_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (src_repo_id, dst_repo_id, rel_path) \
             DO UPDATE SET blob_sha = excluded.blob_sha, \
                           delivered_at = excluded.delivered_at",
            rusqlite::params![
                src_repo_id,
                dst_repo_id,
                rel_path,
                blob_sha,
                utc_now_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// The content hash this install last delivered at `rel_path` — `None`
    /// means this file is NOT ours to prune.
    pub fn peer_delivery_sha(
        &self,
        src_repo_id: i64,
        dst_repo_id: i64,
        rel_path: &str,
    ) -> SqlResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT blob_sha FROM peer_channel_deliveries \
             WHERE src_repo_id = ?1 AND dst_repo_id = ?2 AND rel_path = ?3",
            rusqlite::params![src_repo_id, dst_repo_id, rel_path],
            |row| row.get(0),
        )
        .optional()
    }

    /// Forget one delivery: the file it tracked was pruned, or its bytes no
    /// longer match the row (a stale claim is dropped, not trusted).
    pub fn drop_peer_delivery(
        &self,
        src_repo_id: i64,
        dst_repo_id: i64,
        rel_path: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM peer_channel_deliveries \
             WHERE src_repo_id = ?1 AND dst_repo_id = ?2 AND rel_path = ?3",
            rusqlite::params![src_repo_id, dst_repo_id, rel_path],
        )?;
        Ok(())
    }

    /// Forget every delivery of this lane whose file is gone from `dst`:
    /// teardowns and marker-driven closes remove files outside the prune
    /// pass, and their rows would otherwise linger forever.
    pub fn sweep_peer_deliveries(
        &self,
        src_repo_id: i64,
        dst_repo_id: i64,
        present: &[String],
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        if present.is_empty() {
            conn.execute(
                "DELETE FROM peer_channel_deliveries \
                 WHERE src_repo_id = ?1 AND dst_repo_id = ?2",
                rusqlite::params![src_repo_id, dst_repo_id],
            )?;
            return Ok(());
        }
        // Dynamic IN-list — rel_path is a forward-slash lane-relative path.
        let marks = present.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "DELETE FROM peer_channel_deliveries \
             WHERE src_repo_id = ?1 AND dst_repo_id = ?2 AND rel_path NOT IN ({})",
            marks
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(src_repo_id), Box::new(dst_repo_id)];
        for p in present {
            params.push(Box::new(p.clone()));
        }
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, refs.as_slice())?;
        Ok(())
    }
}
