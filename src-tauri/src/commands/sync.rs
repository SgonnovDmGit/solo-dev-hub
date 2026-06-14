use crate::db::AppDb;
use crate::models::*;
use crate::sync;
use chrono;
use std::path::{Path, PathBuf};
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
    // F-012: microservice_ids are now microservice-PROJECT ids (not repo ids).
    // For each, resolve its single server-repo at sync-time.
    let mut copied = 0usize;
    let mut responses = 0usize;
    let mut migrated = 0usize;
    let mut errors: Vec<String> = vec![];

    // 0.10.0 pre-phase: write project.md + CLAUDE.md section + .gitignore + .gitattributes to all repos.
    // 0.11.0 extends with todo.md + bug-reports.md skeletons.
    let gitignore_template = db
        .get_template_file("_global", ".gitignore.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();
    let gitattributes_template = db
        .get_template_file("_global", ".gitattributes.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();
    let todo_template = db
        .get_template_file("_global", "todo.md.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();
    let bug_reports_template = db
        .get_template_file("_global", "bug-reports.md.tmpl")
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();

    for repo in &all_repos {
        let label = repo.display_name();
        let Some(path) = repo.local_path.as_deref() else {
            // B-000002: surface silently-skipped repos so user sees why project.md /
            // CLAUDE.md / .gitignore weren't written for this repo.
            errors.push(format!(
                "Repo {}: no local_path set — project.md / CLAUDE.md / .gitignore skipped",
                label
            ));
            continue;
        };
        let base = Path::new(path);
        if let Err(e) = sync::ensure_root_exists(base) {
            errors.push(format!(
                "Repo {}: {} — project.md / CLAUDE.md / .gitignore skipped",
                label, e
            ));
            continue;
        }
        if let Err(e) = sync::generate_project_md(&db, project_id, base) {
            errors.push(format!("project.md for {}: {}", label, e));
        }
        if let Err(e) = sync::update_claude_md_section(
            &db,
            Some(project_id),
            repo.role.as_deref(),
            &base.join("CLAUDE.md"),
        ) {
            errors.push(format!("CLAUDE.md for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitignore_section(&gitignore_template, base) {
            errors.push(format!(".gitignore for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitattributes_section(&gitattributes_template, base) {
            errors.push(format!(".gitattributes for {}: {}", label, e));
        }
        if let Err(e) = sync::copy_doc_skeleton_if_missing(&todo_template, base, "todo.md") {
            errors.push(format!("todo.md for {}: {}", label, e));
        }
        if let Err(e) =
            sync::copy_doc_skeleton_if_missing(&bug_reports_template, base, "bug-reports.md")
        {
            errors.push(format!("bug-reports.md for {}: {}", label, e));
        }
    }
    for ms_id in &microservice_ids {
        let Ok(ms_server) = db.server_repo_of_microservice(*ms_id) else {
            errors.push(format!(
                "Microservice project {}: no server-repo resolved — project.md skipped",
                ms_id
            ));
            continue;
        };
        let label = ms_server.display_name();
        let Some(path) = ms_server.local_path.as_deref() else {
            errors.push(format!(
                "Microservice {}: no local_path set — project.md skipped",
                label
            ));
            continue;
        };
        let base = Path::new(path);
        if let Err(e) = sync::ensure_root_exists(base) {
            errors.push(format!(
                "Microservice {}: {} — project.md skipped",
                label, e
            ));
            continue;
        }
        if let Err(e) = sync::generate_project_md(&db, *ms_id, base) {
            errors.push(format!("project.md for {}: {}", label, e));
        }
        if let Err(e) = sync::update_claude_md_section(
            &db,
            Some(*ms_id),
            ms_server.role.as_deref(),
            &base.join("CLAUDE.md"),
        ) {
            errors.push(format!("CLAUDE.md for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitignore_section(&gitignore_template, base) {
            errors.push(format!(".gitignore for {}: {}", label, e));
        }
        if let Err(e) = sync::sync_gitattributes_section(&gitattributes_template, base) {
            errors.push(format!(".gitattributes for {}: {}", label, e));
        }
        if let Err(e) = sync::copy_doc_skeleton_if_missing(&todo_template, base, "todo.md") {
            errors.push(format!("todo.md for {}: {}", label, e));
        }
        if let Err(e) =
            sync::copy_doc_skeleton_if_missing(&bug_reports_template, base, "bug-reports.md")
        {
            errors.push(format!("bug-reports.md for {}: {}", label, e));
        }
    }

    // Server/client checks only matter for `standard` project type. A
    // `microservice` project intentionally has neither — surfacing those as
    // errors produces false-positive warning toasts on every sync.
    let project_type = db
        .get_project(project_id)
        .ok()
        .map(|p| p.project_type)
        .unwrap_or_else(|| "standard".to_string());
    if project_type == "standard" {
        // P6 review-fix: "No clients" is only relevant once the server is
        // in place. Server-only build-out phase (server first, clients
        // later) shouldn't generate a warning toast on every sync — that
        // trains the user to ignore them. The "No server" case stays a
        // warning because the server is the core of the standard flow.
        if server.is_none() {
            errors.push("No server found in project".to_string());
        } else if clients.is_empty() {
            errors.push("No clients found in project".to_string());
        }
    }

    if let Some(srv) = server {
        if srv.local_path.is_none() {
            errors.push(format!("Server {} has no local_path", srv.display_name()));
        }
        if let Some(ref srv_path) = srv.local_path {
            let srv_base = Path::new(srv_path);

            // B-001: guard — do not silently recreate a moved/deleted server folder.
            if let Err(e) = sync::ensure_root_exists(srv_base) {
                errors.push(format!("Server {}: {}", srv.display_name(), e));
                return Ok(SyncResult {
                    copied,
                    responses,
                    migrated,
                    errors,
                });
            }

            // Client → Server sync
            for client in &clients {
                if let Some(ref client_path) = client.local_path {
                    let client_base = Path::new(client_path);
                    // B-001: skip clients whose folder was moved/deleted.
                    if let Err(e) = sync::ensure_root_exists(client_base) {
                        errors.push(format!("Client {}: {}", client.display_name(), e));
                        continue;
                    }
                    let client_req_dir = client_base.join("docs").join("backend-requirements");
                    // F-033: canonical_folder_name() is the single source of truth.
                    let client_name = client.canonical_folder_name();
                    let client_requirements_parent =
                        srv_base.join("docs").join("client-requirements");
                    // F-033 Stage 1e: replay client renames on server side before sync.
                    match db.list_renames_for_repo(client.id) {
                        Ok(renames) => {
                            for r in renames {
                                match sync::replay_rename_in_dir(
                                    &client_requirements_parent,
                                    &r.old_canonical,
                                    &r.new_canonical,
                                ) {
                                    Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                    Ok(sync::RenameOutcome::NoOp) => {}
                                    Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                        "Rename collision on server side: both {}/ and {}/ exist under client-requirements — manual intervention needed",
                                        r.old_canonical, r.new_canonical
                                    )),
                                    Err(e) => errors.push(format!(
                                        "Rename replay {} → {} on server: {}",
                                        r.old_canonical, r.new_canonical, e
                                    )),
                                }
                            }
                        }
                        Err(e) => {
                            errors.push(format!("List renames for client {}: {}", client.id, e))
                        }
                    }
                    let srv_client_dir = client_requirements_parent.join(&client_name);

                    // Copy REQ-*.md from client (source of truth) to server.
                    // Overwrite on change so sender edits propagate to the recipient.
                    for req_file in sync::scan_requirements(&client_req_dir) {
                        let src = client_req_dir.join(&req_file);
                        let dst = srv_client_dir.join(&req_file);
                        match sync::copy_file_if_changed(&src, &dst) {
                            Ok(true) => copied += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!("Copy {} -> server: {}", req_file, e)),
                        }
                    }

                    // Copy *.response.md from server (source of truth) back to client.
                    // Overwrite on change so recipient edits propagate to the sender.
                    for resp_file in sync::scan_responses(&srv_client_dir) {
                        let src = srv_client_dir.join(&resp_file);
                        let dst = client_req_dir.join(&resp_file);
                        match sync::copy_file_if_changed(&src, &dst) {
                            Ok(true) => responses += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!("Copy {} -> client: {}", resp_file, e)),
                        }
                    }

                    // 0.9.0: api.md + handlers.md target moved to docs/server-api/
                    // Auto-migrate old docs/api.md to docs/server-api/api.md.
                    let old_api = client_base.join("docs").join("api.md");
                    let new_api = client_base.join("docs").join("server-api").join("api.md");
                    if old_api.exists() && !new_api.exists() {
                        match sync::migrate_file(&old_api, &new_api) {
                            Ok(()) => migrated += 1,
                            Err(e) => errors.push(format!(
                                "Migrate api.md on {}: {}",
                                client.display_name(),
                                e
                            )),
                        }
                    }

                    // Copy server's docs/api.md to client's docs/server-api/api.md
                    let srv_api = srv_base.join("docs").join("api.md");
                    if srv_api.exists() {
                        match sync::copy_file_if_changed(&srv_api, &new_api) {
                            Ok(true) => copied += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!(
                                "Copy api.md -> {}: {}",
                                client.display_name(),
                                e
                            )),
                        }
                    }
                    // M5 review-fix: `api.md` absent → silent skip, symmetric
                    // with `handlers.md`. Freshly scaffolded servers that haven't
                    // yet written their contract were generating an error toast
                    // on every sync. Once the server writes api.md the next sync
                    // picks it up normally; the "missing api.md" condition is
                    // reported in the client's pre-flight check (see global
                    // CLAUDE.md `# API contract sync`), not via sync errors.

                    // Copy server's docs/handlers.md to client's docs/server-api/handlers.md
                    // (optional — silent skip if missing)
                    let srv_handlers = srv_base.join("docs").join("handlers.md");
                    let client_handlers = client_base
                        .join("docs")
                        .join("server-api")
                        .join("handlers.md");
                    if srv_handlers.exists() {
                        match sync::copy_file_if_changed(&srv_handlers, &client_handlers) {
                            Ok(true) => copied += 1,
                            Ok(false) => {}
                            Err(e) => errors.push(format!(
                                "Copy handlers.md -> {}: {}",
                                client.display_name(),
                                e
                            )),
                        }
                    }
                }
            }

            // F-012: Server → Microservice sync.
            // For each connected microservice-project, resolve its single server-repo
            // and sync REQ-*.md into that repo's docs/server-requirements/.
            for ms_project_id in &microservice_ids {
                let ms_server_repo = match db.server_repo_of_microservice(*ms_project_id) {
                    Ok(r) => r,
                    Err(e) => {
                        errors.push(format!("Microservice project {}: {}", ms_project_id, e));
                        continue;
                    }
                };
                let ms_path = match ms_server_repo.local_path.as_deref() {
                    Some(p) => p,
                    None => {
                        errors.push(format!(
                            "Microservice server-repo {} has no local_path",
                            ms_server_repo.display_name()
                        ));
                        continue;
                    }
                };
                let ms_base = Path::new(ms_path);
                // B-001: skip microservices whose folder was moved/deleted.
                if let Err(e) = sync::ensure_root_exists(ms_base) {
                    errors.push(format!(
                        "Microservice {}: {}",
                        ms_server_repo.display_name(),
                        e
                    ));
                    continue;
                }
                // Resolve microservice-project name for subfolder on server side.
                let ms_project = match db.get_project(*ms_project_id) {
                    Ok(p) => p,
                    Err(e) => {
                        errors.push(format!(
                            "Microservice project {} lookup: {}",
                            ms_project_id, e
                        ));
                        continue;
                    }
                };
                let ms_name = ms_project.name; // retained for microservice-api/<ms_name>/ path
                                               // F-033: REQ sync folders use canonical repo name, not project name.
                let ms_canonical = ms_server_repo.canonical_folder_name();
                let parent_folder = srv.canonical_folder_name();
                let ms_req_parent = srv_base.join("docs").join("microservice-requirements");
                let ms_srv_parent = ms_base.join("docs").join("server-requirements");

                // F-033 Stage 1f Case B: rename server-side folder <project-name>/ → <ms_canonical>/
                // for existing installations (one-time migration; idempotent on subsequent syncs).
                if ms_name != ms_canonical {
                    let mut case_b_warnings = Vec::new();
                    match sync::migrate_subfolder_rename(
                        &ms_req_parent,
                        &ms_name,
                        &ms_canonical,
                        &mut case_b_warnings,
                    ) {
                        Ok(true) => migrated += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!(
                            "Case B migrate {} → {}: {}",
                            ms_name, ms_canonical, e
                        )),
                    }
                    errors.append(&mut case_b_warnings);
                }

                // F-033 Stage 1e: replay ms-server-repo renames on parent side (microservice-requirements/).
                match db.list_renames_for_repo(ms_server_repo.id) {
                    Ok(renames) => {
                        for r in renames {
                            match sync::replay_rename_in_dir(
                                &ms_req_parent,
                                &r.old_canonical,
                                &r.new_canonical,
                            ) {
                                Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                Ok(sync::RenameOutcome::NoOp) => {}
                                Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                    "Rename collision on server side: both {}/ and {}/ exist under microservice-requirements — manual intervention needed",
                                    r.old_canonical, r.new_canonical
                                )),
                                Err(e) => errors.push(format!(
                                    "Rename replay {} → {} on server: {}",
                                    r.old_canonical, r.new_canonical, e
                                )),
                            }
                        }
                    }
                    Err(e) => errors.push(format!(
                        "List renames for ms-server-repo {}: {}",
                        ms_server_repo.id, e
                    )),
                }

                // T-000092: replay ms-PROJECT renames on parent side
                // (microservice-api/<ms-project-name>/). repo_renames doesn't
                // cover project renames because the folder is keyed by project
                // name, not repo canonical name.
                let ms_api_parent = srv_base.join("docs").join("microservice-api");
                match db.list_renames_for_project(*ms_project_id) {
                    Ok(renames) => {
                        for r in renames {
                            match sync::replay_rename_in_dir(
                                &ms_api_parent,
                                &r.old_name,
                                &r.new_name,
                            ) {
                                Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                Ok(sync::RenameOutcome::NoOp) => {}
                                Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                    "Rename collision on parent side: both {}/ and {}/ exist under microservice-api — manual intervention needed",
                                    r.old_name, r.new_name
                                )),
                                Err(e) => errors.push(format!(
                                    "Project rename replay {} → {} on parent: {}",
                                    r.old_name, r.new_name, e
                                )),
                            }
                        }
                    }
                    Err(e) => errors.push(format!(
                        "List renames for ms-project {}: {}",
                        ms_project_id, e
                    )),
                }

                // F-033 Stage 1e: replay server renames on ms side (server-requirements/).
                match db.list_renames_for_repo(srv.id) {
                    Ok(renames) => {
                        for r in renames {
                            match sync::replay_rename_in_dir(
                                &ms_srv_parent,
                                &r.old_canonical,
                                &r.new_canonical,
                            ) {
                                Ok(sync::RenameOutcome::Renamed) => migrated += 1,
                                Ok(sync::RenameOutcome::NoOp) => {}
                                Ok(sync::RenameOutcome::Collision) => errors.push(format!(
                                    "Rename collision on microservice side: both {}/ and {}/ exist under server-requirements — manual intervention needed",
                                    r.old_canonical, r.new_canonical
                                )),
                                Err(e) => errors.push(format!(
                                    "Rename replay {} → {} on microservice: {}",
                                    r.old_canonical, r.new_canonical, e
                                )),
                            }
                        }
                    }
                    Err(e) => errors.push(format!("List renames for server {}: {}", srv.id, e)),
                }

                // F-033 Stage 1f Case C: migrate flat server-requirements/*.md to nested
                // server-requirements/<parent-canonical>/<filename>. Runs per (parent, ms) iteration
                // but idempotent — first parent's pass does the heavy lift, later parents see empty.
                let parents_for_ms = db
                    .list_parents_of_microservice(*ms_project_id)
                    .unwrap_or_default();
                let mut parent_candidates: Vec<(String, PathBuf)> = Vec::new();
                for parent_proj in &parents_for_ms {
                    // server_repo_of_microservice works for any project_id — returns role=server repo.
                    if let Ok(p_srv_repo) = db.server_repo_of_microservice(parent_proj.id) {
                        if let Some(ref lp) = p_srv_repo.local_path {
                            let parent_req_dir = Path::new(lp)
                                .join("docs")
                                .join("microservice-requirements")
                                .join(&ms_canonical);
                            parent_candidates
                                .push((p_srv_repo.canonical_folder_name(), parent_req_dir));
                        }
                    }
                }
                if !parent_candidates.is_empty() {
                    let lookup = |name: &str| -> Vec<(String, PathBuf)> {
                        parent_candidates
                            .iter()
                            .map(|(c, d)| (c.clone(), d.join(name)))
                            .collect()
                    };
                    let mut case_c_warnings = Vec::new();
                    match sync::migrate_flat_to_nested(&ms_srv_parent, lookup, &mut case_c_warnings)
                    {
                        Ok(n) => migrated += n,
                        Err(e) => errors.push(format!("Case C migration: {}", e)),
                    }
                    errors.append(&mut case_c_warnings);
                }

                let srv_ms_dir = ms_req_parent.join(&ms_canonical);
                // F-033: nested per parent-server so multi-parent microservices don't collide.
                let ms_srv_dir = ms_srv_parent.join(&parent_folder);

                // Copy REQ-*.md from server (source of truth) to microservice server-repo.
                // Overwrite on change so sender edits propagate to the recipient.
                for req_file in sync::scan_requirements(&srv_ms_dir) {
                    let src = srv_ms_dir.join(&req_file);
                    let dst = ms_srv_dir.join(&req_file);
                    match sync::copy_file_if_changed(&src, &dst) {
                        Ok(true) => copied += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!("Copy {} -> microservice: {}", req_file, e)),
                    }
                }

                // Copy *.response.md from microservice (source of truth) back to server.
                // Overwrite on change so recipient edits propagate to the sender.
                for resp_file in sync::scan_responses(&ms_srv_dir) {
                    let src = ms_srv_dir.join(&resp_file);
                    let dst = srv_ms_dir.join(&resp_file);
                    match sync::copy_file_if_changed(&src, &dst) {
                        Ok(true) => responses += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!("Copy {} -> server: {}", resp_file, e)),
                    }
                }

                // 0.9.0: Microservice → Parent server — api.md + handlers.md
                // Target on parent: docs/microservice-api/<ms-project-name>/{api,handlers}.md
                let ms_api_src = ms_base.join("docs").join("api.md");
                let ms_api_dst = srv_base
                    .join("docs")
                    .join("microservice-api")
                    .join(&ms_name)
                    .join("api.md");
                if ms_api_src.exists() {
                    match sync::copy_file_if_changed(&ms_api_src, &ms_api_dst) {
                        Ok(true) => copied += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!(
                            "Copy ms api.md from {}: {}",
                            ms_server_repo.display_name(),
                            e
                        )),
                    }
                }
                let ms_handlers_src = ms_base.join("docs").join("handlers.md");
                let ms_handlers_dst = srv_base
                    .join("docs")
                    .join("microservice-api")
                    .join(&ms_name)
                    .join("handlers.md");
                if ms_handlers_src.exists() {
                    match sync::copy_file_if_changed(&ms_handlers_src, &ms_handlers_dst) {
                        Ok(true) => copied += 1,
                        Ok(false) => {}
                        Err(e) => errors.push(format!(
                            "Copy ms handlers.md from {}: {}",
                            ms_server_repo.display_name(),
                            e
                        )),
                    }
                }
            }
        }
    }

    // B-000019/B-000020: MS-driven sync — when the current project is a
    // microservice, fan out to each connected parent server. Mirrors the
    // parent-driven block above but with the MS as initiator. Without this,
    // pressing Sync on an MS project is a no-op (its clients/microservices
    // loops are empty), so api.md edits never propagate and parents stay out
    // of sync until they happen to trigger Sync themselves. Rename-replay is
    // intentionally not duplicated here — parent-driven sync remains the
    // authority for that, this block does the steady-state file copies only.
    if project_type == "microservice" {
        if let Some(ms_server) = server {
            if let Some(ref ms_path) = ms_server.local_path {
                let ms_base = Path::new(ms_path);
                if let Err(e) = sync::ensure_root_exists(ms_base) {
                    errors.push(format!("Microservice {}: {}", ms_server.display_name(), e));
                } else {
                    let ms_canonical = ms_server.canonical_folder_name();
                    let ms_project_name = db
                        .get_project(project_id)
                        .map(|p| p.name)
                        .unwrap_or_default();
                    let parents = db
                        .list_parents_of_microservice(project_id)
                        .unwrap_or_default();
                    for parent_project in &parents {
                        let parent_repos = match db.list_repos_by_project(Some(parent_project.id)) {
                            Ok(r) => r,
                            Err(e) => {
                                errors.push(format!(
                                    "Parent {} list repos: {}",
                                    parent_project.name, e
                                ));
                                continue;
                            }
                        };
                        let Some(parent_server) = parent_repos
                            .iter()
                            .find(|r| r.role.as_deref() == Some("server"))
                        else {
                            errors.push(format!(
                                "Parent {}: no server-repo found",
                                parent_project.name
                            ));
                            continue;
                        };
                        let Some(ref parent_local) = parent_server.local_path else {
                            errors.push(format!(
                                "Parent {} ({}): server-repo has no local_path",
                                parent_project.name,
                                parent_server.display_name()
                            ));
                            continue;
                        };
                        let parent_base = Path::new(parent_local);
                        if let Err(e) = sync::ensure_root_exists(parent_base) {
                            errors.push(format!(
                                "Parent {} ({}): {}",
                                parent_project.name,
                                parent_server.display_name(),
                                e
                            ));
                            continue;
                        }
                        let parent_canonical = parent_server.canonical_folder_name();

                        // MS → parent: api.md + handlers.md
                        for filename in ["api.md", "handlers.md"] {
                            let src = ms_base.join("docs").join(filename);
                            if !src.exists() {
                                continue;
                            }
                            let dst = parent_base
                                .join("docs")
                                .join("microservice-api")
                                .join(&ms_project_name)
                                .join(filename);
                            match sync::copy_file_if_changed(&src, &dst) {
                                Ok(true) => copied += 1,
                                Ok(false) => {}
                                Err(e) => errors.push(format!(
                                    "Copy {} -> parent {}: {}",
                                    filename, parent_project.name, e
                                )),
                            }
                        }

                        // parent → MS: REQ-*.md (source of truth on parent side)
                        let parent_ms_dir = parent_base
                            .join("docs")
                            .join("microservice-requirements")
                            .join(&ms_canonical);
                        let ms_parent_dir = ms_base
                            .join("docs")
                            .join("server-requirements")
                            .join(&parent_canonical);
                        for req_file in sync::scan_requirements(&parent_ms_dir) {
                            let src = parent_ms_dir.join(&req_file);
                            let dst = ms_parent_dir.join(&req_file);
                            match sync::copy_file_if_changed(&src, &dst) {
                                Ok(true) => copied += 1,
                                Ok(false) => {}
                                Err(e) => errors.push(format!(
                                    "Copy {} from parent {} to MS: {}",
                                    req_file, parent_project.name, e
                                )),
                            }
                        }

                        // MS → parent: *.response.md (source of truth on MS side)
                        for resp_file in sync::scan_responses(&ms_parent_dir) {
                            let src = ms_parent_dir.join(&resp_file);
                            let dst = parent_ms_dir.join(&resp_file);
                            match sync::copy_file_if_changed(&src, &dst) {
                                Ok(true) => responses += 1,
                                Ok(false) => {}
                                Err(e) => errors.push(format!(
                                    "Copy {} from MS to parent {}: {}",
                                    resp_file, parent_project.name, e
                                )),
                            }
                        }
                    }
                }
            }
        }
    }

    // v0.20.0: record sync event. SyncResult fields per models.rs:282 — copied + responses + migrated
    let total_changes = (copied + responses + migrated) as i64;
    let _ = db.insert_sync_event(
        None,
        "project_sync",
        &chrono::Utc::now().to_rfc3339(),
        total_changes,
        Some(&format!(r#"{{"project_id":{}}}"#, project_id)),
    );

    Ok(SyncResult {
        copied,
        responses,
        migrated,
        errors,
    })
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
