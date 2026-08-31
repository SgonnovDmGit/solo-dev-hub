// T-000191 (v1.15.0): peer links for the InOutBox channel between equals
// (client<->client, server<->server, microservice<->microservice).
//
// **Every pair is explicit.** A first cut derived pairs automatically from a
// matching role inside one project; the owner rejected that on sight
// (2026-08-31), and the arithmetic says why — six test clients in one project
// derive twenty channels, and everyone ends up able to write to everyone.
// Connecting a peer is a decision, exactly as connecting a microservice is:
// you open the channels you actually need and no others.
//
// A pair is therefore one `peer_links` row and nothing else. Roles are not
// checked here: the owner decides who is a peer of whom, and a same-project
// pair is declared exactly the way a cross-product one is.

use super::*;

/// Normalise a symmetric pair so one logical link is exactly one row.
/// Mirrors the `CHECK (repo_a_id < repo_b_id)` constraint from migration v31.
fn normalize_pair(a: i64, b: i64) -> (i64, i64) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

impl AppDb {
    /// Declare two repositories to be peers. Symmetric — argument order is
    /// irrelevant, the pair is normalised before insert. Idempotent: linking an
    /// already-linked pair is a no-op, not an error.
    pub fn add_peer_link(&self, repo_a_id: i64, repo_b_id: i64) -> Result<(), String> {
        if repo_a_id == repo_b_id {
            return Err("A repository cannot be its own peer".to_string());
        }
        let (a, b) = normalize_pair(repo_a_id, repo_b_id);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO peer_links (repo_a_id, repo_b_id, created_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![a, b, utc_now_rfc3339()],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Drop an explicit peer link. Symmetric, and a no-op when the pair was
    /// never linked. Does not touch files: unlinking stops future syncing of
    /// the channel, it never deletes what has already been exchanged.
    pub fn remove_peer_link(&self, repo_a_id: i64, repo_b_id: i64) -> SqlResult<()> {
        let (a, b) = normalize_pair(repo_a_id, repo_b_id);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM peer_links WHERE repo_a_id = ?1 AND repo_b_id = ?2",
            rusqlite::params![a, b],
        )?;
        Ok(())
    }

    /// All explicit links, newest first.
    pub fn list_peer_links(&self) -> SqlResult<Vec<PeerLink>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, repo_a_id, repo_b_id, created_at FROM peer_links
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PeerLink {
                id: row.get(0)?,
                repo_a_id: row.get(1)?,
                repo_b_id: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Ids of repositories explicitly linked to `repo_id`. Reads both columns
    /// because the pair is stored normalised, not per-direction.
    pub fn explicit_peers_of_repo(&self, repo_id: i64) -> SqlResult<Vec<i64>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT CASE WHEN repo_a_id = ?1 THEN repo_b_id ELSE repo_a_id END
             FROM peer_links WHERE repo_a_id = ?1 OR repo_b_id = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![repo_id], |row| row.get(0))?;
        rows.collect()
    }

    /// Every peer of `repo_id`, ordered by repo id so a sync pass is
    /// deterministic. Explicit links are the only source — nothing is inferred.
    pub fn peers_of_repo(&self, repo_id: i64) -> SqlResult<Vec<Repository>> {
        let mut out: Vec<Repository> = Vec::new();
        for id in self.explicit_peers_of_repo(repo_id)? {
            // A link whose target row is gone is skipped rather than failing the
            // whole resolve — FK CASCADE normally prevents this.
            if let Ok(repo) = self.get_repository(id) {
                out.push(repo);
            }
        }
        out.sort_by_key(|r| r.id);
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

    fn repo(db: &AppDb, path: &str, name: &str, project: Option<i64>, role: Option<&str>) -> i64 {
        db.insert_local_repository(path, name, project, role)
            .unwrap()
            .id
    }

    #[test]
    fn peer_link_is_symmetric_and_normalised() {
        let db = make_db();
        let a = repo(&db, "/tmp/a", "a", None, None);
        let b = repo(&db, "/tmp/b", "b", None, None);

        // Linked in the "wrong" order — must still normalise to one row.
        db.add_peer_link(b, a).unwrap();
        db.add_peer_link(a, b).unwrap(); // idempotent
        let links = db.list_peer_links().unwrap();
        assert_eq!(links.len(), 1);
        assert!(links[0].repo_a_id < links[0].repo_b_id);

        assert_eq!(db.explicit_peers_of_repo(a).unwrap(), vec![b]);
        assert_eq!(db.explicit_peers_of_repo(b).unwrap(), vec![a]);

        db.remove_peer_link(b, a).unwrap();
        assert!(db.list_peer_links().unwrap().is_empty());
    }

    #[test]
    fn self_link_is_rejected() {
        let db = make_db();
        let a = repo(&db, "/tmp/a", "a", None, None);
        assert!(db.add_peer_link(a, a).is_err());
    }

    #[test]
    fn nothing_is_inferred_from_a_matching_role() {
        // Two clients in one project are NOT peers until the owner says so.
        // Without this, six test clients in one project would open twenty
        // channels nobody asked for.
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let a = repo(&db, "/tmp/c1", "c1", Some(p.id), Some("client"));
        let b = repo(&db, "/tmp/c2", "c2", Some(p.id), Some("client"));
        assert!(db.peers_of_repo(a).unwrap().is_empty());

        db.add_peer_link(a, b).unwrap();
        let peers = db.peers_of_repo(a).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, b);
    }

    #[test]
    fn explicit_link_pairs_across_projects() {
        let db = make_db();
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let p2 = db.create_project("P2", None, "standard").unwrap();
        let s1 = repo(&db, "/tmp/s1", "s1", Some(p1.id), Some("server"));
        let s2 = repo(&db, "/tmp/s2", "s2", Some(p2.id), Some("server"));
        assert!(db.peers_of_repo(s1).unwrap().is_empty());

        db.add_peer_link(s1, s2).unwrap();
        let peers = db.peers_of_repo(s1).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, s2);
    }

    #[test]
    fn a_link_is_listed_from_both_ends_exactly_once() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let a = repo(&db, "/tmp/s1", "s1", Some(p.id), Some("server"));
        let b = repo(&db, "/tmp/s2", "s2", Some(p.id), Some("server"));
        db.add_peer_link(a, b).unwrap();
        assert_eq!(db.peers_of_repo(a).unwrap().len(), 1);
        assert_eq!(db.peers_of_repo(b).unwrap().len(), 1);
    }

    #[test]
    fn deleting_a_repo_cascades_its_links() {
        let db = make_db();
        let a = repo(&db, "/tmp/a", "a", None, None);
        let b = repo(&db, "/tmp/b", "b", None, None);
        db.add_peer_link(a, b).unwrap();
        db.delete_repository(a).unwrap();
        assert!(db.list_peer_links().unwrap().is_empty());
    }
}
