// T-000191 (v1.15.0): peer links and peer resolution for the InOutBox channel
// between equals (client<->client, server<->server, microservice<->microservice).
//
// A repo's peers come from two sources, unioned:
//
//  - *derived* — repositories of the same project that carry the same role.
//    The role match is strict: `client` and `admin_client` are NOT peers
//    automatically (precedent: codex lanes, which pair `server<->server` and
//    `admin_client<->admin_client` and never cross the two). A NULL role and
//    the role `other` are excluded — they mean "role not set", not a level.
//
//  - *explicit* — rows in `peer_links`. This is the only way to pair repos in
//    different projects (server<->server between two products), which the
//    derivation cannot see.
//
// The two sources overlap freely: a pair may be both derived and explicit, and
// both flags are reported so the UI can offer "unlink" only where a row exists.

use super::*;

/// Roles that never form a *derived* peer pair. `other` is "role not set",
/// which is not a level and must not silently pair two unrelated repos.
const NON_PEER_ROLES: [&str; 1] = ["other"];

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

    /// Every peer of `repo_id` — derived ∪ explicit, deduplicated by repo id.
    /// Ordered by repo id so a sync pass is deterministic.
    pub fn peers_of_repo(&self, repo_id: i64) -> SqlResult<Vec<PeerOf>> {
        let me = self.get_repository(repo_id)?;

        // Derived: same project, same role, role is set and pairable.
        let mut derived: Vec<Repository> = Vec::new();
        if let (Some(pid), Some(role)) = (me.project_id, me.role.as_deref()) {
            if !NON_PEER_ROLES.contains(&role) {
                derived = self
                    .list_repos_by_project(Some(pid))?
                    .into_iter()
                    .filter(|r| r.id != me.id && r.role.as_deref() == Some(role))
                    .collect();
            }
        }

        let explicit_ids = self.explicit_peers_of_repo(repo_id)?;

        let mut out: Vec<PeerOf> = Vec::new();
        for repo in derived {
            let explicit = explicit_ids.contains(&repo.id);
            out.push(PeerOf {
                repo,
                derived: true,
                explicit,
            });
        }
        for id in explicit_ids {
            if out.iter().any(|p| p.repo.id == id) {
                continue; // already counted as derived
            }
            // A link whose target row is gone is skipped rather than failing the
            // whole resolve — FK CASCADE normally prevents this.
            if let Ok(repo) = self.get_repository(id) {
                out.push(PeerOf {
                    repo,
                    derived: false,
                    explicit: true,
                });
            }
        }
        out.sort_by_key(|p| p.repo.id);
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
    fn same_project_same_role_pairs_automatically() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let a = repo(&db, "/tmp/c1", "c1", Some(p.id), Some("client"));
        let b = repo(&db, "/tmp/c2", "c2", Some(p.id), Some("client"));

        let peers = db.peers_of_repo(a).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].repo.id, b);
        assert!(peers[0].derived);
        assert!(!peers[0].explicit);
    }

    #[test]
    fn different_role_and_other_role_do_not_pair() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        // Strict role match: client vs admin_client are not peers.
        let c = repo(&db, "/tmp/c", "c", Some(p.id), Some("client"));
        repo(&db, "/tmp/ac", "ac", Some(p.id), Some("admin_client"));
        assert!(db.peers_of_repo(c).unwrap().is_empty());

        // `other` is "role not set" — two such repos must not pair.
        let o1 = repo(&db, "/tmp/o1", "o1", Some(p.id), Some("other"));
        repo(&db, "/tmp/o2", "o2", Some(p.id), Some("other"));
        assert!(db.peers_of_repo(o1).unwrap().is_empty());

        // A NULL role likewise pairs with nothing.
        let n1 = repo(&db, "/tmp/n1", "n1", Some(p.id), None);
        repo(&db, "/tmp/n2", "n2", Some(p.id), None);
        assert!(db.peers_of_repo(n1).unwrap().is_empty());
    }

    #[test]
    fn explicit_link_pairs_across_projects() {
        let db = make_db();
        let p1 = db.create_project("P1", None, "standard").unwrap();
        let p2 = db.create_project("P2", None, "standard").unwrap();
        let s1 = repo(&db, "/tmp/s1", "s1", Some(p1.id), Some("server"));
        let s2 = repo(&db, "/tmp/s2", "s2", Some(p2.id), Some("server"));

        // Same role but different projects — derivation must not pair them.
        assert!(db.peers_of_repo(s1).unwrap().is_empty());

        db.add_peer_link(s1, s2).unwrap();
        let peers = db.peers_of_repo(s1).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].repo.id, s2);
        assert!(!peers[0].derived);
        assert!(peers[0].explicit);
    }

    #[test]
    fn derived_and_explicit_overlap_is_one_peer_with_both_flags() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let a = repo(&db, "/tmp/s1", "s1", Some(p.id), Some("server"));
        let b = repo(&db, "/tmp/s2", "s2", Some(p.id), Some("server"));
        db.add_peer_link(a, b).unwrap();

        let peers = db.peers_of_repo(a).unwrap();
        assert_eq!(peers.len(), 1, "one peer, not two");
        assert!(peers[0].derived && peers[0].explicit);
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
