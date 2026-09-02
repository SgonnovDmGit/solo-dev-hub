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
// A pair is therefore one `peer_links` row and nothing else, and a same-project
// pair is declared exactly the way a cross-product one is.
//
// T-000201 (v1.16.1) adds the one thing that is *not* the owner's choice: the
// two sides must actually be equals. Which pairs to open stays a decision; that
// a client and a server are not peers is a fact about the channel. Nothing is
// ever inferred or auto-linked — the check only refuses a pair that could not
// have meant anything.

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

/// The placeholder role. It says nothing about a repository's level, so it can
/// never establish equality with anything, including another `other`.
const UNPAIRABLE_ROLE: &str = "other";

impl AppDb {
    /// T-000201: are these two repositories actually equals?
    ///
    /// Equality is an exact role match plus a matching project type. Role alone
    /// is not enough: a server inside a microservice project answers to its
    /// parent servers, so it is not the equal of a server that heads an ordinary
    /// product, even though both are spelled `server`. Role families are not
    /// collapsed either — an admin client and an ordinary client solve different
    /// problems, and an exchange between them is vertical in nature.
    ///
    /// Returns the reason as an `Err` so both the guard and the dialog can show
    /// the same explanation instead of silently hiding a choice.
    pub fn check_peers_are_equal(&self, a_id: i64, b_id: i64) -> Result<(), String> {
        let a = self.get_repository(a_id).map_err(|e| e.to_string())?;
        let b = self.get_repository(b_id).map_err(|e| e.to_string())?;
        let ra = a.role.as_deref().unwrap_or("");
        let rb = b.role.as_deref().unwrap_or("");
        if ra.is_empty() || rb.is_empty() {
            return Err("Both peers need a role before they can be linked".to_string());
        }
        if ra == UNPAIRABLE_ROLE || rb == UNPAIRABLE_ROLE {
            return Err(
                "Role `other` cannot be paired: it says nothing about a repository's level"
                    .to_string(),
            );
        }
        if ra != rb {
            return Err(format!(
                "Peers must share a role, and these do not: `{}` and `{}`",
                ra, rb
            ));
        }
        let ta = self.project_type_of_repo(a_id)?;
        let tb = self.project_type_of_repo(b_id)?;
        if ta != tb {
            return Err(format!(
                "Peers must sit in the same kind of project, and these do not: `{}` and `{}`",
                ta, tb
            ));
        }
        Ok(())
    }

    /// The `project_type` of the project this repository belongs to. An
    /// unassigned repository has no level to compare, which is itself the
    /// reason it cannot be paired.
    fn project_type_of_repo(&self, repo_id: i64) -> Result<String, String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT p.project_type FROM repositories r
               JOIN projects p ON p.id = r.project_id
              WHERE r.id = ?1",
            rusqlite::params![repo_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| {
            "Both peers must belong to a project — an unassigned repository has no level"
                .to_string()
        })
    }

    /// Declare two repositories to be peers. Symmetric — argument order is
    /// irrelevant, the pair is normalised before insert. Idempotent: linking an
    /// already-linked pair is a no-op, not an error.
    pub fn add_peer_link(&self, repo_a_id: i64, repo_b_id: i64) -> Result<(), String> {
        if repo_a_id == repo_b_id {
            return Err("A repository cannot be its own peer".to_string());
        }
        // T-000201: enforced in the store, not only in the dialog. A UI filter
        // is a convenience that can be bypassed; the contract has to hold here.
        self.check_peers_are_equal(repo_a_id, repo_b_id)?;
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

    /// A pair that satisfies the T-000201 equality rule: same role, same kind
    /// of project. Most of these tests are about linking mechanics, not about
    /// equality, so they start from a valid pair.
    fn equal_pair(db: &AppDb) -> (i64, i64) {
        let p = db.create_project("P", None, "standard").unwrap();
        let a = repo(db, "/tmp/a", "a", Some(p.id), Some("server"));
        let b = repo(db, "/tmp/b", "b", Some(p.id), Some("server"));
        (a, b)
    }

    #[test]
    fn peer_link_is_symmetric_and_normalised() {
        let db = make_db();
        let (a, b) = equal_pair(&db);

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
        let (a, b) = equal_pair(&db);
        db.add_peer_link(a, b).unwrap();
        db.delete_repository(a).unwrap();
        assert!(db.list_peer_links().unwrap().is_empty());
    }

    // ── T-000201: only equals can be linked ──────────────────────────────────

    #[test]
    fn a_client_and_a_server_are_not_peers() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let c = repo(&db, "/tmp/c", "c", Some(p.id), Some("client"));
        let s = repo(&db, "/tmp/s", "s", Some(p.id), Some("server"));
        let err = db.add_peer_link(c, s).unwrap_err();
        assert!(err.contains("share a role"), "unhelpful message: {}", err);
        assert!(db.list_peer_links().unwrap().is_empty());
    }

    #[test]
    fn an_admin_client_is_not_the_equal_of_a_plain_client() {
        // Owner's call, 2026-09-02: role families are not collapsed. An admin
        // client and an ordinary client solve different problems, so an
        // exchange between them is vertical in nature, not peer-to-peer.
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let a = repo(&db, "/tmp/ac", "ac", Some(p.id), Some("admin_client"));
        let c = repo(&db, "/tmp/c", "c", Some(p.id), Some("client"));
        assert!(db.add_peer_link(a, c).is_err());
        // …but two admin clients are equals.
        let a2 = repo(&db, "/tmp/ac2", "ac2", Some(p.id), Some("admin_client"));
        db.add_peer_link(a, a2).unwrap();
    }

    #[test]
    fn the_placeholder_role_cannot_be_paired() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let a = repo(&db, "/tmp/o1", "o1", Some(p.id), Some("other"));
        let b = repo(&db, "/tmp/o2", "o2", Some(p.id), Some("other"));
        // Matching literally, yet `other` states no level — so not even two of
        // them are equals.
        assert!(db.add_peer_link(a, b).is_err());
    }

    #[test]
    fn a_microservice_server_is_not_the_equal_of_an_ordinary_server() {
        // Same role spelling, different standing: a microservice's server
        // answers to its parent servers, so the two are not on one level.
        let db = make_db();
        let std_p = db.create_project("Product", None, "standard").unwrap();
        let ms_p = db.create_project("MS", None, "microservice").unwrap();
        let s1 = repo(&db, "/tmp/s1", "s1", Some(std_p.id), Some("server"));
        let s2 = repo(&db, "/tmp/s2", "s2", Some(ms_p.id), Some("server"));
        let err = db.add_peer_link(s1, s2).unwrap_err();
        assert!(
            err.contains("same kind of project"),
            "unhelpful message: {}",
            err
        );
    }

    #[test]
    fn two_microservice_servers_are_peers() {
        let db = make_db();
        let m1 = db.create_project("MS1", None, "microservice").unwrap();
        let m2 = db.create_project("MS2", None, "microservice").unwrap();
        let s1 = repo(&db, "/tmp/m1", "m1", Some(m1.id), Some("server"));
        let s2 = repo(&db, "/tmp/m2", "m2", Some(m2.id), Some("server"));
        db.add_peer_link(s1, s2).unwrap();
        assert_eq!(db.peers_of_repo(s1).unwrap().len(), 1);
    }

    #[test]
    fn a_repo_without_a_role_or_project_cannot_be_paired() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let assigned = repo(&db, "/tmp/s", "s", Some(p.id), Some("server"));
        let roleless = repo(&db, "/tmp/n", "n", Some(p.id), None);
        assert!(db.add_peer_link(assigned, roleless).is_err());

        let orphan = repo(&db, "/tmp/orphan", "orphan", None, Some("server"));
        let err = db.add_peer_link(assigned, orphan).unwrap_err();
        assert!(err.contains("belong to a project"), "message: {}", err);
    }

    #[test]
    fn an_existing_link_survives_a_later_role_change() {
        // The guard runs on linking, never retroactively: revalidating stored
        // pairs would one day sever a live exchange without being asked to.
        let db = make_db();
        let (a, b) = equal_pair(&db);
        db.add_peer_link(a, b).unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE repositories SET role = 'client' WHERE id = ?1",
                rusqlite::params![b],
            )
            .unwrap();
        }
        assert_eq!(db.peers_of_repo(a).unwrap().len(), 1);
        assert_eq!(db.list_peer_links().unwrap().len(), 1);
    }
}
