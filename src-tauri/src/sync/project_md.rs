// docs/project.md generation. Renders project structure (repos, microservices,
// parents) into a Markdown file consumed by both humans and the cross-repo
// announcement workflow (parent server-repo path lookup).

use crate::db::AppDb;
use std::fs;
use std::path::Path;

/// Generate docs/project.md describing the project's structure.
/// Overwrites existing file on each call (app-owned).
pub fn generate_project_md(db: &AppDb, project_id: i64, repo_root: &Path) -> Result<(), String> {
    let project = db.get_project(project_id).map_err(|e| e.to_string())?;
    let repos = db
        .list_repos_by_project(Some(project_id))
        .map_err(|e| e.to_string())?;
    let ms_ids = db
        .list_project_microservices(project_id)
        .map_err(|e| e.to_string())?;
    let parents = db
        .list_parents_of_microservice(project_id)
        .map_err(|e| e.to_string())?;

    let type_display = match project.project_type.as_str() {
        "microservice" => "⚙ Microservice",
        "codex" => "📚 Codex",
        _ => "📁 Standard",
    };

    let mut md = format!(
        "# {}\n\n{}\n\n**Project type:** {}\n\n",
        project.name,
        project.description.as_deref().unwrap_or(""),
        type_display,
    );

    md.push_str("## Repositories\n\n");
    if repos.is_empty() {
        md.push_str("_No repositories._\n\n");
    } else {
        md.push_str("| Repository | Role | Path | GitHub |\n");
        md.push_str("|------------|------|------|--------|\n");
        for r in &repos {
            let name = r.display_name();
            let role = r.role.as_deref().unwrap_or("—");
            let path = r.local_path.as_deref().unwrap_or("—");
            let gh = if r.github_name.is_some() {
                "✓"
            } else {
                "📁 local"
            };
            md.push_str(&format!("| {} | {} | {} | {} |\n", name, role, path, gh));
        }
        md.push('\n');
    }

    // B-000013: unified table layout across all three sections — humans see
    // the same shape everywhere, LLMs still grep the same marker strings
    // (`_no local path configured_`, `⚠ server repo not resolvable`).
    md.push_str("## Connected microservices\n\n");
    if ms_ids.is_empty() {
        md.push_str("_No connected microservices._\n\n");
    } else {
        md.push_str("| Microservice | Server repo | Path | GitHub |\n");
        md.push_str("|--------------|-------------|------|--------|\n");
        for ms_id in &ms_ids {
            let ms_proj = db.get_project(*ms_id).map_err(|e| e.to_string())?;
            let (srv_label, path_label, gh_label) = match db.server_repo_of_microservice(*ms_id) {
                Ok(srv) => {
                    let path = srv
                        .local_path
                        .clone()
                        .unwrap_or_else(|| "_no local path configured_".to_string());
                    let gh = if srv.github_name.is_some() {
                        "✓"
                    } else {
                        "📁 local"
                    };
                    (srv.display_name(), path, gh.to_string())
                }
                Err(_) => (
                    "⚠ server repo not resolvable".to_string(),
                    "—".to_string(),
                    "—".to_string(),
                ),
            };
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                ms_proj.name, srv_label, path_label, gh_label
            ));
        }
        md.push('\n');
    }

    // T-000166: a codex publishes on up to two audience lanes (server,
    // admin_client), each backed by one repo in the codex project. Its svod
    // lands in the consumer's same-role repo under docs/codex/<codex-name>/.
    // Render one row per lane the codex actually has.
    let codex_ids = db
        .list_connected_codices(project_id)
        .map_err(|e| e.to_string())?;
    md.push_str("## Connected Codices\n\n");
    if codex_ids.is_empty() {
        md.push_str("_No connected codices._\n\n");
    } else {
        md.push_str("| Codex | Lane | Source repo | Path | Inbox |\n");
        md.push_str("|-------|------|-------------|------|-------|\n");
        for codex_id in &codex_ids {
            let codex_proj = db.get_project(*codex_id).map_err(|e| e.to_string())?;
            let inbox = format!("docs/codex/{}/", codex_proj.name);
            let mut any_lane = false;
            for lane in ["server", "admin_client"] {
                if let Ok(repo) = db.repo_of_project_with_role(*codex_id, lane) {
                    any_lane = true;
                    let path = repo
                        .local_path
                        .clone()
                        .unwrap_or_else(|| "_no local path configured_".to_string());
                    md.push_str(&format!(
                        "| {} | {} | {} | {} | {} |\n",
                        codex_proj.name,
                        lane,
                        repo.display_name(),
                        path,
                        inbox
                    ));
                }
            }
            // A codex with no resolvable lane repos still gets a visible marker row.
            if !any_lane {
                md.push_str(&format!(
                    "| {} | ⚠ no lane repos | — | — | {} |\n",
                    codex_proj.name, inbox
                ));
            }
        }
        md.push('\n');
    }

    // T-000193: peer channels. This section is what tells the agent WHO it may
    // open an InOutBox exchange with — every pair is an explicit link, so an
    // absent row means the channel does not exist and must not be invented.
    md.push_str(
        "## Peer channels

",
    );
    let mut peer_rows: Vec<String> = Vec::new();
    for repo in &repos {
        for peer in db.peers_of_repo(repo.id).unwrap_or_default() {
            let peer_project = peer
                .project_id
                .and_then(|pid| db.get_project(pid).ok())
                .map(|p| p.name)
                .unwrap_or_else(|| "—".to_string());
            peer_rows.push(format!(
                "| {} | {} | {} | docs/InOutBox/…/{}/ |
",
                repo.display_name(),
                peer.display_name(),
                peer_project,
                peer.canonical_folder_name()
            ));
        }
    }
    if peer_rows.is_empty() {
        md.push_str("_No peer channels. Sharing a role opens nothing — a peer is connected deliberately, in Solo Dev Hub._

");
    } else {
        md.push_str(
            "| This repo | Peer repo | Peer project | Channel |
",
        );
        md.push_str(
            "|-----------|-----------|--------------|---------|
",
        );
        for row in peer_rows {
            md.push_str(&row);
        }
        md.push_str("
Only the pairs listed here may exchange PACT / HANDOFF / NOTE (skills `sdh-peer-channel-send`, `sdh-peer-channel-answer`). A repo that is absent from this table is not a peer, however similar its role looks.

");
    }

    // Keep the "## Parent projects" heading verbatim — the announce workflow
    // greps that exact string. Only the rows differ by project type.
    md.push_str("## Parent projects\n\n");
    if parents.is_empty() {
        md.push_str("_No parent projects._\n\n");
    } else if project.project_type == "codex" {
        // T-000175: for a codex's own project.md, `parents` are its CONSUMERS.
        // Resolve each consumer by the codex's audience lane (server /
        // admin_client) instead of always the server repo — a codex-for-admins
        // must point at consumers' admin repos, not their server repos. Only
        // lanes the codex itself publishes on are probed. Inbox is this codex's
        // own folder in the consumer's tree.
        let inbox = format!("docs/codex/{}/", project.name);
        let codex_lanes: Vec<&str> = ["server", "admin_client"]
            .into_iter()
            .filter(|lane| db.repo_of_project_with_role(project_id, lane).is_ok())
            .collect();
        md.push_str("| Consumer | Lane | Contact repo | Path | Inbox |\n");
        md.push_str("|----------|------|--------------|------|-------|\n");
        for p in &parents {
            let mut any_lane = false;
            for lane in &codex_lanes {
                if let Ok(repo) = db.repo_of_project_with_role(p.id, lane) {
                    any_lane = true;
                    let path = repo
                        .local_path
                        .clone()
                        .unwrap_or_else(|| "_no local path configured_".to_string());
                    md.push_str(&format!(
                        "| {} | {} | {} | {} | {} |\n",
                        p.name,
                        lane,
                        repo.display_name(),
                        path,
                        inbox
                    ));
                }
            }
            // A consumer with no matching-lane repo still gets a marker row.
            if !any_lane {
                md.push_str(&format!(
                    "| {} | ⚠ no matching-lane repo | — | — | {} |\n",
                    p.name, inbox
                ));
            }
        }
        md.push('\n');
    } else {
        // F-000040: include each parent server-repo's local path so MS-LLM
        // can write proactive announcements directly into that filesystem.
        // server_repo_of_microservice() is generic over project_id and
        // returns the server-role repo of any project (despite its name).
        md.push_str("| Parent project | Server repo | Path | GitHub |\n");
        md.push_str("|----------------|-------------|------|--------|\n");
        for p in &parents {
            let (srv_label, path_label, gh_label) = match db.server_repo_of_microservice(p.id) {
                Ok(srv) => {
                    let path = srv
                        .local_path
                        .clone()
                        .unwrap_or_else(|| "_no local path configured_".to_string());
                    let gh = if srv.github_name.is_some() {
                        "✓"
                    } else {
                        "📁 local"
                    };
                    (srv.display_name(), path, gh.to_string())
                }
                Err(_) => (
                    "⚠ server repo not resolvable".to_string(),
                    "—".to_string(),
                    "—".to_string(),
                ),
            };
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                p.name, srv_label, path_label, gh_label
            ));
        }
        md.push('\n');
    }

    md.push_str("---\n_Auto-generated by Solo Dev Hub. Do not edit manually — changes will be overwritten on next sync._\n");

    let docs_dir = repo_root.join("docs");
    fs::create_dir_all(&docs_dir).map_err(|e| e.to_string())?;
    fs::write(docs_dir.join("project.md"), md).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template_seeder::seed_bundled_templates;
    use std::fs;
    use tempfile::TempDir;

    fn make_test_db() -> AppDb {
        let dir = TempDir::new().unwrap();
        let db = AppDb::new(dir.path().join("test.db")).unwrap();
        seed_bundled_templates(&db).unwrap();
        db
    }

    #[test]
    fn test_generate_project_md_standard_with_repos() {
        let db = make_test_db();
        let proj = db
            .create_project("WebApp", Some("My web application"), "standard")
            .unwrap();
        let repo = db
            .upsert_repository("owner/web-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(repo.id, Some(proj.id), Some("server"))
            .unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, proj.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("# WebApp"));
        assert!(content.contains("My web application"));
        assert!(content.contains("📁 Standard"));
        // B-000001: display_name() returns last segment, not full owner/repo.
        assert!(content.contains("web-backend"));
        assert!(!content.contains("owner/web-backend"));
        assert!(content.contains("server"));
    }

    #[test]
    fn test_generate_project_md_microservice_with_parents() {
        let db = make_test_db();
        let parent = db.create_project("Parent", None, "standard").unwrap();
        let ms = db.create_project("AuthMS", None, "microservice").unwrap();
        let ms_repo = db
            .upsert_repository("owner/auth-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(ms_repo.id, Some(ms.id), Some("server"))
            .unwrap();
        db.connect_microservice(parent.id, ms.id).unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, ms.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("# AuthMS"));
        assert!(content.contains("⚙ Microservice"));
        assert!(content.contains("Parent"), "parent project listed");
    }

    /// F-000040: Parent projects section must include parent server-repo's
    /// local path so MS-LLM can write announcements directly into that filesystem.
    #[test]
    fn test_generate_project_md_microservice_parent_includes_server_path() {
        let db = make_test_db();
        let parent = db.create_project("WebApp", None, "standard").unwrap();
        let parent_srv = db
            .upsert_repository("owner/web-app-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(parent_srv.id, Some(parent.id), Some("server"))
            .unwrap();
        db.set_repo_local_path(parent_srv.id, Some("/home/dev/web-app-backend"))
            .unwrap();

        let ms = db.create_project("Storage", None, "microservice").unwrap();
        let ms_repo = db
            .upsert_repository("owner/storage-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(ms_repo.id, Some(ms.id), Some("server"))
            .unwrap();
        db.connect_microservice(parent.id, ms.id).unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, ms.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        // MS-LLM consumes path from this row to write announcements to parent.
        // B-000013: unified table layout — parent row carries server repo + path columns.
        assert!(
            content.contains("| WebApp | web-app-backend | /home/dev/web-app-backend | ✓ |"),
            "parent rendering should include canonical server-repo name + local path in table row; got:\n{}",
            content
        );
    }

    /// F-000040: Parent server repo without local_path renders graceful placeholder
    /// (announcement-write is impossible in that state, but project.md still renders).
    #[test]
    fn test_generate_project_md_microservice_parent_without_server_path() {
        let db = make_test_db();
        let parent = db.create_project("WebApp", None, "standard").unwrap();
        let parent_srv = db
            .upsert_repository("owner/web-app-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(parent_srv.id, Some(parent.id), Some("server"))
            .unwrap();
        // intentionally no set_repo_local_path

        let ms = db.create_project("Storage", None, "microservice").unwrap();
        let ms_repo = db
            .upsert_repository("owner/storage-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(ms_repo.id, Some(ms.id), Some("server"))
            .unwrap();
        db.connect_microservice(parent.id, ms.id).unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, ms.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        // B-000013: marker string `_no local path configured_` survives the format
        // change so the announcement-LLM grep stays unchanged.
        assert!(
            content.contains("| WebApp | web-app-backend | _no local path configured_ | ✓ |"),
            "should signal missing path explicitly in table row; got:\n{}",
            content
        );
    }

    /// T-000193: the `## Peer channels` section is what tells the agent whom it
    /// may exchange with. A shared role must render nothing — only a declared
    /// link produces a row.
    #[test]
    fn test_generate_project_md_peer_channels_section() {
        let db = make_test_db();
        let proj = db.create_project("Peers", None, "standard").unwrap();
        let a = db
            .insert_local_repository("/tmp/pm-a", "alpha", Some(proj.id), Some("client"))
            .unwrap();
        let b = db
            .insert_local_repository("/tmp/pm-b", "beta", Some(proj.id), Some("client"))
            .unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, proj.id, tmp.path()).unwrap();
        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("## Peer channels"));
        assert!(
            content.contains("_No peer channels."),
            "two clients of one project are not peers until linked; got:\n{}",
            content
        );

        db.add_peer_link(a.id, b.id).unwrap();
        generate_project_md(&db, proj.id, tmp.path()).unwrap();
        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(
            content.contains("| alpha | beta | Peers | docs/InOutBox/…/beta/ |"),
            "a declared link must render a row; got:\n{}",
            content
        );
        // Symmetric: the pair shows from both ends.
        assert!(content.contains("| beta | alpha | Peers | docs/InOutBox/…/alpha/ |"));
    }

    /// T-000166: a project of type `codex` renders the 📚 Codex type label.
    #[test]
    fn test_generate_project_md_codex_type_label() {
        let db = make_test_db();
        let codex = db.create_project("RulesCodex", None, "codex").unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, codex.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("# RulesCodex"));
        assert!(content.contains("📚 Codex"));
    }

    /// T-000166: a consumer project with a connected codex that publishes on
    /// both `server` and `admin_client` lanes renders one Connected Codices row
    /// per lane, and the codex stays OUT of the Connected microservices section.
    #[test]
    fn test_generate_project_md_connected_codices_section() {
        let db = make_test_db();
        let consumer = db.create_project("WebApp", None, "standard").unwrap();

        let codex = db.create_project("DesignCodex", None, "codex").unwrap();
        let codex_srv = db
            .upsert_repository("owner/design-codex-server", None, None, None, None, None)
            .unwrap();
        db.assign_repository(codex_srv.id, Some(codex.id), Some("server"))
            .unwrap();
        db.set_repo_local_path(codex_srv.id, Some("/home/dev/design-codex-server"))
            .unwrap();

        let codex_admin = db
            .upsert_repository("owner/design-codex-admin", None, None, None, None, None)
            .unwrap();
        db.assign_repository(codex_admin.id, Some(codex.id), Some("admin_client"))
            .unwrap();
        db.set_repo_local_path(codex_admin.id, Some("/home/dev/design-codex-admin"))
            .unwrap();

        // connect_microservice gates on target type='microservice'; a codex
        // target is rejected, so insert the project→project edge directly.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO project_microservices (project_id, microservice_project_id) VALUES (?1, ?2)",
                rusqlite::params![consumer.id, codex.id],
            )
            .unwrap();
        }

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, consumer.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("## Connected Codices"));
        // Codex name + inbox path.
        assert!(content.contains("DesignCodex"));
        assert!(content.contains("docs/codex/DesignCodex/"));
        // One row per lane, with the source-repo display name.
        assert!(
            content.contains("| DesignCodex | server | design-codex-server | /home/dev/design-codex-server | docs/codex/DesignCodex/ |"),
            "server lane row expected; got:\n{}",
            content
        );
        assert!(
            content.contains("| DesignCodex | admin_client | design-codex-admin | /home/dev/design-codex-admin | docs/codex/DesignCodex/ |"),
            "admin_client lane row expected; got:\n{}",
            content
        );
        // Codex must NOT bleed into the microservices section.
        assert!(
            content.contains("_No connected microservices._"),
            "codex must stay out of the microservices section; got:\n{}",
            content
        );
    }

    /// T-000166: a project with no connected codices still renders the section
    /// with the empty-state placeholder.
    #[test]
    fn test_generate_project_md_no_connected_codices_placeholder() {
        let db = make_test_db();
        let proj = db.create_project("Plain", None, "standard").unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, proj.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("## Connected Codices"));
        assert!(content.contains("_No connected codices._"));
    }

    /// T-000175: a codex's own project.md resolves each consumer by the codex's
    /// audience lane, not the consumer's server repo. A codex reaching admins
    /// must point at the consumer's `admin_client` repo, and its server repo
    /// path must NOT leak into the Parent projects section.
    #[test]
    fn test_generate_project_md_codex_parents_show_matching_lane_repo() {
        let db = make_test_db();

        let codex = db.create_project("AdminCodex", None, "codex").unwrap();
        let codex_admin = db
            .upsert_repository("owner/admin-codex-admin", None, None, None, None, None)
            .unwrap();
        db.assign_repository(codex_admin.id, Some(codex.id), Some("admin_client"))
            .unwrap();
        db.set_repo_local_path(codex_admin.id, Some("/home/dev/admin-codex-admin"))
            .unwrap();

        let consumer = db.create_project("ShopAdmin", None, "standard").unwrap();
        let consumer_srv = db
            .upsert_repository("owner/shop-admin-server", None, None, None, None, None)
            .unwrap();
        db.assign_repository(consumer_srv.id, Some(consumer.id), Some("server"))
            .unwrap();
        db.set_repo_local_path(consumer_srv.id, Some("/home/dev/shop-admin-server"))
            .unwrap();
        let consumer_admin = db
            .upsert_repository("owner/shop-admin-panel", None, None, None, None, None)
            .unwrap();
        db.assign_repository(consumer_admin.id, Some(consumer.id), Some("admin_client"))
            .unwrap();
        db.set_repo_local_path(consumer_admin.id, Some("/home/dev/shop-admin-panel"))
            .unwrap();

        // connect_microservice gates on target type='microservice'; a codex
        // target is rejected, so insert the project→project edge directly.
        // Consumer is the "parent" of the codex in this table's direction.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO project_microservices (project_id, microservice_project_id) VALUES (?1, ?2)",
                rusqlite::params![consumer.id, codex.id],
            )
            .unwrap();
        }

        let tmp = TempDir::new().unwrap();
        // Generate the CODEX's own project.md.
        generate_project_md(&db, codex.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        // Heading kept verbatim for the announce workflow grep.
        assert!(content.contains("## Parent projects"));
        // Consumer resolved by the codex's admin_client lane.
        assert!(
            content.contains("| ShopAdmin | admin_client | shop-admin-panel | /home/dev/shop-admin-panel | docs/codex/AdminCodex/ |"),
            "consumer admin_client lane row expected; got:\n{}",
            content
        );
        // The consumer's server repo path must NOT appear in this section.
        assert!(
            !content.contains("/home/dev/shop-admin-server"),
            "consumer's server repo path must not leak into codex Parent projects section; got:\n{}",
            content
        );
    }

    #[test]
    fn test_generate_project_md_fallback_placeholders_when_empty() {
        let db = make_test_db();
        let proj = db.create_project("Empty", None, "standard").unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, proj.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("_No repositories._"));
        assert!(content.contains("_No connected microservices._"));
        assert!(content.contains("_No parent projects._"));
        assert!(!content.contains("{{"));
    }
}
