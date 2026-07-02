use crate::db::AppDb;
use crate::models::*;
use crate::sync;
use chrono;
use std::path::Path;
use tauri::State;

// ── Requirements sync commands ───────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct SyncGlobalClaudeResult {
    path: String,
    synced_at: String,
}

#[tauri::command]
pub fn sync_global_claude_md(db: State<AppDb>) -> Result<SyncGlobalClaudeResult, String> {
    let home = dirs::home_dir().ok_or("Cannot determine home directory")?;
    let claude_path = home.join(".claude").join("CLAUDE.md");
    sync::update_claude_md_global(&db, &claude_path)?;
    sync::render_skills_global(&db, &home.join(".claude"))?;
    let now = chrono::Utc::now().to_rfc3339();
    db.set_setting("ai_rules_last_sync_at", &now)
        .map_err(|e| e.to_string())?;
    Ok(SyncGlobalClaudeResult {
        path: claude_path.display().to_string(),
        synced_at: now,
    })
}

/// Init skeletons for a single repo — F-016 manual trigger.
/// Copies-if-missing: docs/todo.md, docs/bug-reports.md; section-merges .gitignore + .gitattributes.
/// Returns list of filenames actually created (empty list = all already exist).
#[tauri::command]
pub fn init_docs_for_repo(db: State<AppDb>, repo_id: i64) -> Result<Vec<String>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let path = repo
        .local_path
        .as_deref()
        .ok_or_else(|| "Repository has no local_path".to_string())?;
    let base = Path::new(path);
    sync::ensure_root_exists(base)?;

    let todo_template = db
        .get_template_file("_global", "todo.md.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();
    let bug_reports_template = db
        .get_template_file("_global", "bug-reports.md.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();
    let gitignore_template = db
        .get_template_file("_global", ".gitignore.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();
    let gitattributes_template = db
        .get_template_file("_global", ".gitattributes.tmpl")
        .map_err(|e| e.to_string())?
        .map(|t| t.content)
        .unwrap_or_default();

    let mut updated = Vec::new();
    if sync::copy_doc_skeleton_if_missing(&todo_template, base, "todo.md")? {
        updated.push("docs/todo.md".to_string());
    }
    if sync::copy_doc_skeleton_if_missing(&bug_reports_template, base, "bug-reports.md")? {
        updated.push("docs/bug-reports.md".to_string());
    }
    if sync::sync_gitignore_section(&gitignore_template, base)? {
        updated.push(".gitignore (section)".to_string());
    }
    if sync::sync_gitattributes_section(&gitattributes_template, base)? {
        updated.push(".gitattributes (section)".to_string());
    }
    // App-owned files (project.md + CLAUDE.md section) — always overwritten when
    // the repo is attached to a project. Orphan repos (project_id=None) skip this
    // since project-context wouldn't render meaningfully.
    sync::render_skills_to_repo(&db, base)?;
    updated.push("docs/sdh_skills/".to_string());
    if let Some(pid) = repo.project_id {
        sync::generate_project_md(&db, pid, base)?;
        updated.push("docs/project.md".to_string());
        sync::update_claude_md_section(
            &db,
            Some(pid),
            repo.role.as_deref(),
            &base.join("CLAUDE.md"),
        )?;
        updated.push("CLAUDE.md (section)".to_string());
    } else {
        // M6 review-fix: surface that the repo is orphan and the app-owned
        // files were intentionally skipped. Without this entry the success
        // toast lists what was actually written, leaving the user wondering
        // why project.md / CLAUDE.md weren't updated.
        updated.push("(project.md + CLAUDE.md skipped — repo has no project assigned)".to_string());
    }
    Ok(updated)
}

#[tauri::command]
pub fn sync_project(db: State<AppDb>, project_id: i64) -> Result<SyncResult, String> {
    sync::run_project_sync(&db, project_id)
}

#[tauri::command]
pub fn list_project_requirements(
    db: State<AppDb>,
    project_id: i64,
) -> Result<Vec<RequirementInfo>, String> {
    let all_repos = db
        .list_repos_by_project(Some(project_id))
        .map_err(|e| e.to_string())?;
    let microservice_ids = db
        .list_project_microservices(project_id)
        .map_err(|e| e.to_string())?;

    let server = all_repos
        .iter()
        .find(|r| r.role.as_deref() == Some("server"));
    let clients: Vec<&Repository> = all_repos
        .iter()
        .filter(|r| {
            matches!(
                r.role.as_deref(),
                Some("client") | Some("admin_client") | Some("test_client")
            )
        })
        .collect();
    // F-012: resolve microservice-projects and their single server-repos.
    let mut ms_entries: Vec<(String, Repository)> = vec![]; // (ms-project name, server-repo)
    for ms_project_id in &microservice_ids {
        let Ok(ms_project) = db.get_project(*ms_project_id) else {
            continue;
        };
        let Ok(ms_server_repo) = db.server_repo_of_microservice(*ms_project_id) else {
            continue;
        };
        ms_entries.push((ms_project.name, ms_server_repo));
    }

    let mut result: Vec<RequirementInfo> = vec![];

    if let Some(srv) = server {
        if let Some(ref srv_path) = srv.local_path {
            let srv_base = Path::new(srv_path);

            // Client → Server requirements
            for client in &clients {
                if let Some(ref client_path) = client.local_path {
                    let client_base = Path::new(client_path);
                    let client_req_dir = client_base.join("docs").join("backend-requirements");
                    // F-033: canonical_folder_name() is the single source of truth.
                    let client_name = client.canonical_folder_name();
                    let srv_client_dir = srv_base
                        .join("docs")
                        .join("client-requirements")
                        .join(&client_name);

                    for req_file in sync::scan_requirements(&client_req_dir) {
                        let on_server = srv_client_dir.join(&req_file).exists();
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = srv_client_dir.join(&response_name).exists()
                            || client_req_dir.join(&response_name).exists();
                        // F-000039: impl-ack lives in the sender's (client) outgoing dir.
                        let impl_name = req_file.replace(".md", ".impl.md");
                        let has_impl = client_req_dir.join(&impl_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else if on_server {
                            "sent".to_string()
                        } else {
                            "new".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "client_to_server".to_string(),
                            status,
                            source_repo: client.display_name().clone(),
                            target_repo: srv.display_name().clone(),
                            is_reverse_lookup: false,
                            has_impl,
                        });
                    }

                    // Show api.md + handlers.md sync status (server → client, docs/server-api/)
                    for (name, src_filename) in
                        [("api.md", "api.md"), ("handlers.md", "handlers.md")]
                    {
                        let srv_file = srv_base.join("docs").join(src_filename);
                        if !srv_file.exists() {
                            continue;
                        }
                        let client_file = client_base
                            .join("docs")
                            .join("server-api")
                            .join(src_filename);
                        let status = if !client_file.exists() {
                            "new".to_string()
                        } else {
                            let src = std::fs::read(&srv_file).unwrap_or_default();
                            let dst = std::fs::read(&client_file).unwrap_or_default();
                            if src == dst {
                                "sent".to_string()
                            } else {
                                "new".to_string()
                            }
                        };
                        result.push(RequirementInfo {
                            filename: name.to_string(),
                            direction: "server_to_client".to_string(),
                            status,
                            source_repo: srv.display_name().clone(),
                            target_repo: client.display_name().clone(),
                            is_reverse_lookup: false,
                            has_impl: false,
                        });
                    }

                    // Also check server side for reqs that may only exist there
                    for req_file in sync::scan_requirements(&srv_client_dir) {
                        if result.iter().any(|r| {
                            r.filename == req_file && r.source_repo == client.display_name()
                        }) {
                            continue;
                        }
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = srv_client_dir.join(&response_name).exists()
                            || client_req_dir.join(&response_name).exists();
                        // F-000039: impl-ack lives in the sender's (client) outgoing dir.
                        let impl_name = req_file.replace(".md", ".impl.md");
                        let has_impl = client_req_dir.join(&impl_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else {
                            "sent".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "client_to_server".to_string(),
                            status,
                            source_repo: client.display_name().clone(),
                            target_repo: srv.display_name().clone(),
                            is_reverse_lookup: false,
                            has_impl,
                        });
                    }
                }
            }

            // F-012: Server → Microservice requirements
            for (ms_name, ms_server_repo) in &ms_entries {
                if let Some(ref ms_path) = ms_server_repo.local_path {
                    let ms_base = Path::new(ms_path);
                    // F-033: REQ folders use canonical repo names; nested per parent on ms side.
                    let ms_canonical = ms_server_repo.canonical_folder_name();
                    let parent_folder = srv.canonical_folder_name();
                    let srv_ms_dir = srv_base
                        .join("docs")
                        .join("microservice-requirements")
                        .join(&ms_canonical);
                    let ms_srv_dir = ms_base
                        .join("docs")
                        .join("server-requirements")
                        .join(&parent_folder);

                    for req_file in sync::scan_requirements(&srv_ms_dir) {
                        let on_ms = ms_srv_dir.join(&req_file).exists();
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = ms_srv_dir.join(&response_name).exists()
                            || srv_ms_dir.join(&response_name).exists();
                        // F-000039: impl-ack lives in the sender's (server) outgoing dir.
                        let impl_name = req_file.replace(".md", ".impl.md");
                        let has_impl = srv_ms_dir.join(&impl_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else if on_ms {
                            "sent".to_string()
                        } else {
                            "new".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: srv.display_name().clone(),
                            target_repo: ms_server_repo.display_name().clone(),
                            is_reverse_lookup: false,
                            has_impl,
                        });
                    }

                    // Also check ms side
                    for req_file in sync::scan_requirements(&ms_srv_dir) {
                        if result.iter().any(|r| {
                            r.filename == req_file && r.target_repo == ms_server_repo.display_name()
                        }) {
                            continue;
                        }
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = ms_srv_dir.join(&response_name).exists()
                            || srv_ms_dir.join(&response_name).exists();
                        // F-000039: impl-ack lives in the sender's (server) outgoing dir.
                        let impl_name = req_file.replace(".md", ".impl.md");
                        let has_impl = srv_ms_dir.join(&impl_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else {
                            "sent".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: srv.display_name().clone(),
                            target_repo: ms_server_repo.display_name().clone(),
                            is_reverse_lookup: false,
                            has_impl,
                        });
                    }

                    // 0.9.0: Microservice → Parent server — api.md + handlers.md
                    for (filename, direction) in [
                        ("api.md", "microservice_to_server_api"),
                        ("handlers.md", "microservice_to_server_handlers"),
                    ] {
                        let ms_src = ms_base.join("docs").join(filename);
                        if !ms_src.exists() {
                            continue;
                        }
                        let parent_dst = srv_base
                            .join("docs")
                            .join("microservice-api")
                            .join(ms_name)
                            .join(filename);
                        let status = if !parent_dst.exists() {
                            "new".to_string()
                        } else {
                            let src = std::fs::read(&ms_src).unwrap_or_default();
                            let dst = std::fs::read(&parent_dst).unwrap_or_default();
                            if src == dst {
                                "sent".to_string()
                            } else {
                                "new".to_string()
                            }
                        };
                        result.push(RequirementInfo {
                            filename: filename.to_string(),
                            direction: direction.to_string(),
                            status,
                            source_repo: ms_server_repo.display_name().clone(),
                            target_repo: srv.display_name().clone(),
                            is_reverse_lookup: false,
                            has_impl: false,
                        });
                    }
                }
            }
        }
    }

    // B-000018 reverse-lookup: открывая ms-проект, показать requirements от parent серверов.
    // Sender = parent server, recipient = текущий ms. Confirm-✓ скрыт в UI — sender'у проще
    // подтвердить из своего собственного SyncScreen (project_microservices direct view).
    let current_project = db.get_project(project_id).map_err(|e| e.to_string())?;
    if current_project.project_type == "microservice" {
        if let Some(ms_server) = server {
            if let Some(ref ms_local) = ms_server.local_path {
                let ms_base = Path::new(ms_local);
                let ms_canonical = ms_server.canonical_folder_name();
                let parents = db
                    .list_parents_of_microservice(project_id)
                    .map_err(|e| e.to_string())?;

                for parent_project in &parents {
                    let Ok(parent_repos) = db.list_repos_by_project(Some(parent_project.id)) else {
                        continue;
                    };
                    let Some(parent_server) = parent_repos
                        .iter()
                        .find(|r| r.role.as_deref() == Some("server"))
                    else {
                        continue;
                    };
                    let Some(ref parent_local) = parent_server.local_path else {
                        continue;
                    };
                    let parent_base = Path::new(parent_local);
                    let parent_canonical = parent_server.canonical_folder_name();

                    let parent_ms_dir = parent_base
                        .join("docs")
                        .join("microservice-requirements")
                        .join(&ms_canonical);
                    let ms_parent_dir = ms_base
                        .join("docs")
                        .join("server-requirements")
                        .join(&parent_canonical);

                    // REQ files parent → this ms (server_to_microservice direction)
                    for req_file in sync::scan_requirements(&parent_ms_dir) {
                        let response_name = req_file.replace(".md", ".response.md");
                        let on_ms = ms_parent_dir.join(&req_file).exists();
                        let has_response = ms_parent_dir.join(&response_name).exists()
                            || parent_ms_dir.join(&response_name).exists();
                        // F-000039: impl-ack lives in the sender's (parent server) outgoing dir.
                        let impl_name = req_file.replace(".md", ".impl.md");
                        let has_impl = parent_ms_dir.join(&impl_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else if on_ms {
                            "sent".to_string()
                        } else {
                            "new".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: parent_server.display_name().clone(),
                            target_repo: ms_server.display_name().clone(),
                            is_reverse_lookup: true,
                            has_impl,
                        });
                    }

                    // ms-side files без зеркала на parent (например ms ответил, до next sync)
                    for req_file in sync::scan_requirements(&ms_parent_dir) {
                        if result.iter().any(|r| {
                            r.filename == req_file
                                && r.direction == "server_to_microservice"
                                && r.source_repo == parent_server.display_name()
                                && r.target_repo == ms_server.display_name()
                        }) {
                            continue;
                        }
                        let response_name = req_file.replace(".md", ".response.md");
                        let has_response = ms_parent_dir.join(&response_name).exists()
                            || parent_ms_dir.join(&response_name).exists();
                        // F-000039: impl-ack lives in the sender's (parent server) outgoing dir.
                        let impl_name = req_file.replace(".md", ".impl.md");
                        let has_impl = parent_ms_dir.join(&impl_name).exists();

                        let status = if has_response {
                            "responded".to_string()
                        } else {
                            "sent".to_string()
                        };

                        result.push(RequirementInfo {
                            filename: req_file,
                            direction: "server_to_microservice".to_string(),
                            status,
                            source_repo: parent_server.display_name().clone(),
                            target_repo: ms_server.display_name().clone(),
                            is_reverse_lookup: true,
                            has_impl,
                        });
                    }

                    // ms api.md / handlers.md going to each parent
                    for (filename, direction) in [
                        ("api.md", "microservice_to_server_api"),
                        ("handlers.md", "microservice_to_server_handlers"),
                    ] {
                        let ms_src = ms_base.join("docs").join(filename);
                        if !ms_src.exists() {
                            continue;
                        }
                        let parent_dst = parent_base
                            .join("docs")
                            .join("microservice-api")
                            .join(&current_project.name)
                            .join(filename);
                        let status = if !parent_dst.exists() {
                            "new".to_string()
                        } else {
                            let src = std::fs::read(&ms_src).unwrap_or_default();
                            let dst = std::fs::read(&parent_dst).unwrap_or_default();
                            if src == dst {
                                "sent".to_string()
                            } else {
                                "new".to_string()
                            }
                        };
                        result.push(RequirementInfo {
                            filename: filename.to_string(),
                            direction: direction.to_string(),
                            status,
                            source_repo: ms_server.display_name().clone(),
                            target_repo: parent_server.display_name().clone(),
                            is_reverse_lookup: true,
                            has_impl: false,
                        });
                    }
                }
            }
        }
    }

    Ok(result)
}

#[tauri::command]
pub fn confirm_requirement(
    db: State<AppDb>,
    project_id: i64,
    filename: String,
    source_repo_id: i64,
    target_repo_id: i64,
) -> Result<(), String> {
    // B-000021: paths are derived from source_repo + target_repo directly via
    // sync::confirm_pair. `project_id` is kept in the signature for frontend
    // compatibility and audit-trail purposes but plays no role in path
    // resolution — confirm works symmetrically from either side's SyncScreen.
    let _ = project_id;
    let source_repo = db
        .get_repository(source_repo_id)
        .map_err(|e| e.to_string())?;
    let target_repo = db
        .get_repository(target_repo_id)
        .map_err(|e| e.to_string())?;
    sync::confirm_pair(&source_repo, &target_repo, &filename)
}

// ── Rename log (F-033) ────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_rename_history(db: State<AppDb>) -> Result<Vec<RepoRename>, String> {
    db.list_all_renames().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_renames_for_repo(db: State<AppDb>, repo_id: i64) -> Result<Vec<RepoRename>, String> {
    db.list_renames_for_repo(repo_id).map_err(|e| e.to_string())
}

// ── T-000137: per-repo auto-commit branch selector ────────────────────────────

#[tauri::command]
pub fn get_autocommit_branch(db: State<AppDb>, repo_id: i64) -> Result<Option<String>, String> {
    db.get_autocommit_branch(repo_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autocommit_branch(
    db: State<AppDb>,
    repo_id: i64,
    branch: Option<String>,
) -> Result<(), String> {
    db.set_autocommit_branch(repo_id, branch.as_deref())
        .map_err(|e| e.to_string())
}

// ── v0.20.0: Task sync commands ───────────────────────────────────────────────

#[tauri::command]
pub fn sync_tasks_for_repo_cmd(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<crate::sync::SyncTasksReport, String> {
    let result = crate::sync::sync_tasks_for_repo(&db, repo_id)?;
    if result.events_emitted > 0 || result.imported > 0 {
        let _ = db.insert_sync_event(
            Some(repo_id),
            "tasks",
            &chrono::Utc::now().to_rfc3339(),
            (result.events_emitted + result.imported) as i64,
            None,
        );
    }
    Ok(result)
}

#[tauri::command]
pub fn read_tasks_from_db(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<crate::models::Task>, String> {
    db.list_tasks_by_repo(repo_id, "todo")
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn read_done_from_db(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<crate::models::Task>, String> {
    db.list_tasks_by_repo(repo_id, "done")
        .map_err(|e| e.to_string())
}
