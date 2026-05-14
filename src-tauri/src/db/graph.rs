// T-000094: F-013 1-hop project graph. Returns center repo + ring of in-project
// repos and connected microservice projects (or parent server projects, for
// microservice-type projects).

use super::*;

impl AppDb {
    /// Build a 1-hop graph view for a project. Center node depends on project type:
    /// - 'standard' → server-role repo (or first by sort_order if none)
    /// - 'microservice' → the project's main repo
    ///
    /// Ring: for standard projects = other repos in project + connected microservice
    /// projects (1-hop, via project_microservices). For microservice projects =
    /// parent server projects (reverse lookup via project_microservices).
    pub fn get_project_graph(&self, project_id: i64) -> SqlResult<ProjectGraph> {
        let conn = self.conn.lock().unwrap();

        let project_type: String = conn.query_row(
            "SELECT project_type FROM projects WHERE id = ?1",
            rusqlite::params![project_id],
            |row| row.get(0),
        )?;

        // Load all repos in this project
        let mut stmt = conn.prepare(
            "SELECT id, project_id, github_name, github_url, role, description, language, last_pushed_at, added_at, updated_at, local_path, github_id, deploy_target
             FROM repositories WHERE project_id = ?1 ORDER BY sort_order, id"
        )?;
        let repos: Vec<Repository> = stmt
            .query_map(rusqlite::params![project_id], row_to_repo)?
            .collect::<SqlResult<Vec<_>>>()?;
        drop(stmt);

        if repos.is_empty() {
            return Ok(ProjectGraph { center: None, ring: vec![], edges: vec![] });
        }

        // Use canonical_folder_name() (last segment of github_name, or description)
        // — mirrors frontend getDisplayName, avoids 'owner/repo' prefix in graph labels.
        let repo_to_node = |r: &Repository| GraphNode {
            id: format!("repo:{}", r.id),
            label: r.canonical_folder_name(),
            kind: GraphNodeKind::Repo,
            role: r.role.clone(),
            repo_id: Some(r.id),
            project_id: None,
        };

        let project_to_node = |id: i64, name: String, role: &str| GraphNode {
            id: format!("project:{}", id),
            label: name,
            kind: GraphNodeKind::Project,
            role: Some(role.to_string()),
            repo_id: None,
            project_id: Some(id),
        };

        let (center, ring_repos): (Repository, Vec<Repository>) = if project_type == "standard" {
            // Center = server-role repo, or first repo if no server
            let center_idx = repos.iter().position(|r| r.role.as_deref() == Some("server")).unwrap_or(0);
            let mut rest = repos.clone();
            let center = rest.remove(center_idx);
            (center, rest)
        } else {
            // microservice project — center = first repo
            let mut rest = repos.clone();
            let center = rest.remove(0);
            (center, rest)
        };

        let center_node = repo_to_node(&center);
        let center_id = center_node.id.clone();
        let mut ring: Vec<GraphNode> = ring_repos.iter().map(repo_to_node).collect();
        let mut edges: Vec<GraphEdge> = ring.iter().map(|n| GraphEdge {
            source: center_id.clone(),
            target: n.id.clone(),
            kind: GraphEdgeKind::InProject,
        }).collect();

        // Cross-project edges (microservices)
        if project_type == "standard" {
            // project_microservices: project_id = parent, microservice_project_id = ms
            let mut ms_stmt = conn.prepare(
                "SELECT p.id, p.name FROM projects p
                 JOIN project_microservices pm ON p.id = pm.microservice_project_id
                 WHERE pm.project_id = ?1 ORDER BY p.sort_order, p.id"
            )?;
            let ms_rows: Vec<(i64, String)> = ms_stmt
                .query_map(rusqlite::params![project_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            for (ms_id, ms_name) in ms_rows {
                let node = project_to_node(ms_id, ms_name, "microservice");
                edges.push(GraphEdge {
                    source: center_id.clone(),
                    target: node.id.clone(),
                    kind: GraphEdgeKind::CrossProjectMs,
                });
                ring.push(node);
            }
        } else {
            // microservice project — find parent server projects (reverse lookup)
            // project_microservices: project_id = parent, microservice_project_id = ms
            let mut p_stmt = conn.prepare(
                "SELECT p.id, p.name FROM projects p
                 JOIN project_microservices pm ON p.id = pm.project_id
                 WHERE pm.microservice_project_id = ?1 ORDER BY p.sort_order, p.id"
            )?;
            let parents: Vec<(i64, String)> = p_stmt
                .query_map(rusqlite::params![project_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<SqlResult<Vec<_>>>()?;
            for (pid, pname) in parents {
                let node = project_to_node(pid, pname, "server");
                edges.push(GraphEdge {
                    source: center_id.clone(),
                    target: node.id.clone(),
                    kind: GraphEdgeKind::CrossProjectMs,
                });
                ring.push(node);
            }
        }

        Ok(ProjectGraph { center: Some(center_node), ring, edges })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn test_get_project_graph_server_project() {
        let db = make_db();
        let p = db.create_project("P", None, "standard").unwrap();
        let server = db.insert_local_repository("/s", "server-repo", Some(p.id), Some("server")).unwrap();
        let _client = db.insert_local_repository("/c", "client-repo", Some(p.id), Some("client")).unwrap();
        // Connect a microservice project
        let ms = db.create_project("MS", None, "microservice").unwrap();
        db.connect_microservice(p.id, ms.id).unwrap();

        let g = db.get_project_graph(p.id).unwrap();
        let center = g.center.expect("center exists");
        assert!(matches!(center.kind, GraphNodeKind::Repo));
        assert_eq!(center.repo_id, Some(server.id));
        // Ring should have: client repo + microservice project
        assert_eq!(g.ring.len(), 2);
        // Edges should have 2 entries (one InProject, one CrossProjectMs)
        assert_eq!(g.edges.len(), 2);
        let has_inproj = g.edges.iter().any(|e| matches!(e.kind, GraphEdgeKind::InProject));
        let has_cross = g.edges.iter().any(|e| matches!(e.kind, GraphEdgeKind::CrossProjectMs));
        assert!(has_inproj);
        assert!(has_cross);
    }

    #[test]
    fn test_get_project_graph_microservice_project_returns_parent_servers() {
        let db = make_db();
        let ms = db.create_project("MS", None, "microservice").unwrap();
        db.insert_local_repository("/ms", "ms-repo", Some(ms.id), Some("server")).unwrap();
        let parent = db.create_project("Parent", None, "standard").unwrap();
        db.connect_microservice(parent.id, ms.id).unwrap();

        let g = db.get_project_graph(ms.id).unwrap();
        assert!(g.center.is_some());
        // Ring should include the parent project as a node
        let has_parent = g.ring.iter().any(|n| n.project_id == Some(parent.id));
        assert!(has_parent, "microservice graph should list parent server projects");
    }

    #[test]
    fn test_get_project_graph_empty_project_returns_no_center() {
        let db = make_db();
        let p = db.create_project("Empty", None, "standard").unwrap();
        let g = db.get_project_graph(p.id).unwrap();
        assert!(g.center.is_none(), "empty project: no center");
        assert!(g.ring.is_empty());
        assert!(g.edges.is_empty());
    }

    #[test]
    fn test_get_project_graph_no_server_role_uses_first_repo() {
        let db = make_db();
        let p = db.create_project("NoServer", None, "standard").unwrap();
        let r1 = db.insert_local_repository("/a", "a-repo", Some(p.id), Some("client")).unwrap();
        db.insert_local_repository("/b", "b-repo", Some(p.id), Some("tool")).unwrap();
        let g = db.get_project_graph(p.id).unwrap();
        let center = g.center.expect("center exists");
        assert_eq!(center.repo_id, Some(r1.id), "fallback to first repo when no server");
    }
}
