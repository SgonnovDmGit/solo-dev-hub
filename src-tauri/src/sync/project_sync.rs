// T-000098b: `sync_project` orchestration extracted out of the command layer
// into the sync domain module. The Tauri command `commands::sync::sync_project`
// is now a thin wrapper delegating to `run_project_sync`. Pure structural
// decomposition — zero behavior change vs the pre-split handler.
//
// Layout:
// - `SyncCounters`  — accumulator threaded `&mut` through every helper.
// - `SkeletonTemplates` + `load_skeleton_templates` — the four `_global`
//   doc templates, loaded once and reused across the Phase-0 loops.
// - `write_repo_skeletons` — the per-repo 6-file skeleton write (collapses the
//   two previously-duplicated Phase-0 loop bodies).
// - `sync_client_to_server` / `sync_server_to_microservice` /
//   `sync_microservice_to_parents` — the three directional copy blocks.

use std::path::{Path, PathBuf};

use crate::db::AppDb;
use crate::models::{Repository, SyncResult};
use crate::sync;

/// Mutable accumulator for one `run_project_sync` pass. `copied` counts REQ /
/// api / handlers copies into a recipient, `responses` counts `.response.md`
/// copy-backs, `migrated` counts one-time folder/file migrations + rename
/// replays, `errors` collects non-fatal per-item failures surfaced to the user.
struct SyncCounters {
    copied: usize,
    responses: usize,
    migrated: usize,
    errors: Vec<String>,
}

impl SyncCounters {
    fn new() -> Self {
        Self {
            copied: 0,
            responses: 0,
            migrated: 0,
            errors: Vec::new(),
        }
    }

    fn into_result(self) -> SyncResult {
        SyncResult {
            copied: self.copied,
            responses: self.responses,
            migrated: self.migrated,
            errors: self.errors,
        }
    }
}

/// The four `_global` doc templates written to every repo during Phase-0.
/// Empty string when a template is absent — downstream helpers no-op on empty.
struct SkeletonTemplates {
    gitignore: String,
    gitattributes: String,
    todo: String,
    bug_reports: String,
}

fn load_skeleton_templates(db: &AppDb) -> SkeletonTemplates {
    let load = |name: &str| -> String {
        db.get_template_file("_global", name)
            .ok()
            .flatten()
            .map(|t| t.content)
            .unwrap_or_default()
    };
    SkeletonTemplates {
        gitignore: load(".gitignore.tmpl"),
        gitattributes: load(".gitattributes.tmpl"),
        todo: load("todo.md.tmpl"),
        bug_reports: load("bug-reports.md.tmpl"),
    }
}

/// Write the Phase-0 skeleton files (project.md + CLAUDE.md section +
/// .gitignore + .gitattributes + todo.md + bug-reports.md) into a single repo.
/// Each failure is surfaced as a per-file error keyed by `label`; callers own
/// the no-local_path / ensure_root_exists guards (their wording differs per
/// caller). `md_project_id` is the project whose project.md is generated —
/// the current project for repos, the microservice-project for ms-servers.
fn write_repo_skeletons(
    db: &AppDb,
    md_project_id: i64,
    base: &Path,
    role: Option<&str>,
    label: &str,
    tpl: &SkeletonTemplates,
    c: &mut SyncCounters,
) {
    if let Err(e) = sync::generate_project_md(db, md_project_id, base) {
        c.errors.push(format!("project.md for {}: {}", label, e));
    }
    if let Err(e) =
        sync::update_claude_md_section(db, Some(md_project_id), role, &base.join("CLAUDE.md"))
    {
        c.errors.push(format!("CLAUDE.md for {}: {}", label, e));
    }
    if let Err(e) = sync::sync_gitignore_section(&tpl.gitignore, base) {
        c.errors.push(format!(".gitignore for {}: {}", label, e));
    }
    if let Err(e) = sync::sync_gitattributes_section(&tpl.gitattributes, base) {
        c.errors
            .push(format!(".gitattributes for {}: {}", label, e));
    }
    if let Err(e) = sync::copy_doc_skeleton_if_missing(&tpl.todo, base, "todo.md") {
        c.errors.push(format!("todo.md for {}: {}", label, e));
    }
    if let Err(e) = sync::copy_doc_skeleton_if_missing(&tpl.bug_reports, base, "bug-reports.md") {
        c.errors
            .push(format!("bug-reports.md for {}: {}", label, e));
    }
}

/// Client → Server REQ sync + Server → Client response copy-back + api.md /
/// handlers.md propagation. `srv_base` is the server repo root.
fn sync_client_to_server(
    db: &AppDb,
    srv_base: &Path,
    clients: &[&Repository],
    c: &mut SyncCounters,
) {
    for client in clients {
        if let Some(ref client_path) = client.local_path {
            let client_base = Path::new(client_path);
            // B-001: skip clients whose folder was moved/deleted.
            if let Err(e) = sync::ensure_root_exists(client_base) {
                c.errors
                    .push(format!("Client {}: {}", client.display_name(), e));
                continue;
            }
            let client_req_dir = client_base.join("docs").join("backend-requirements");
            // F-033: canonical_folder_name() is the single source of truth.
            let client_name = client.canonical_folder_name();
            let client_requirements_parent = srv_base.join("docs").join("client-requirements");
            // F-033 Stage 1e: replay client renames on server side before sync.
            match db.list_renames_for_repo(client.id) {
                Ok(renames) => {
                    for r in renames {
                        match sync::replay_rename_in_dir(
                            &client_requirements_parent,
                            &r.old_canonical,
                            &r.new_canonical,
                        ) {
                            Ok(sync::RenameOutcome::Renamed) => c.migrated += 1,
                            Ok(sync::RenameOutcome::NoOp) => {}
                            Ok(sync::RenameOutcome::Collision) => c.errors.push(format!(
                                "Rename collision on server side: both {}/ and {}/ exist under client-requirements — manual intervention needed",
                                r.old_canonical, r.new_canonical
                            )),
                            Err(e) => c.errors.push(format!(
                                "Rename replay {} → {} on server: {}",
                                r.old_canonical, r.new_canonical, e
                            )),
                        }
                    }
                }
                Err(e) => c
                    .errors
                    .push(format!("List renames for client {}: {}", client.id, e)),
            }
            let srv_client_dir = client_requirements_parent.join(&client_name);

            // Copy REQ-*.md from client (source of truth) to server.
            // Overwrite on change so sender edits propagate to the recipient.
            for req_file in sync::scan_requirements(&client_req_dir) {
                let src = client_req_dir.join(&req_file);
                let dst = srv_client_dir.join(&req_file);
                match sync::copy_file_if_changed(&src, &dst) {
                    Ok(true) => c.copied += 1,
                    Ok(false) => {}
                    Err(e) => c.errors.push(format!("Copy {} -> server: {}", req_file, e)),
                }
            }

            // Copy *.response.md from server (source of truth) back to client.
            // Overwrite on change so recipient edits propagate to the sender.
            for resp_file in sync::scan_responses(&srv_client_dir) {
                let src = srv_client_dir.join(&resp_file);
                let dst = client_req_dir.join(&resp_file);
                match sync::copy_file_if_changed(&src, &dst) {
                    Ok(true) => c.responses += 1,
                    Ok(false) => {}
                    Err(e) => c
                        .errors
                        .push(format!("Copy {} -> client: {}", resp_file, e)),
                }
            }

            // 0.9.0: api.md + handlers.md target moved to docs/server-api/
            // Auto-migrate old docs/api.md to docs/server-api/api.md.
            let old_api = client_base.join("docs").join("api.md");
            let new_api = client_base.join("docs").join("server-api").join("api.md");
            if old_api.exists() && !new_api.exists() {
                match sync::migrate_file(&old_api, &new_api) {
                    Ok(()) => c.migrated += 1,
                    Err(e) => c.errors.push(format!(
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
                    Ok(true) => c.copied += 1,
                    Ok(false) => {}
                    Err(e) => {
                        c.errors
                            .push(format!("Copy api.md -> {}: {}", client.display_name(), e))
                    }
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
                    Ok(true) => c.copied += 1,
                    Ok(false) => {}
                    Err(e) => c.errors.push(format!(
                        "Copy handlers.md -> {}: {}",
                        client.display_name(),
                        e
                    )),
                }
            }

            // T-000139: blind-copy sender's docs/my_api/*.md into the client's server-api inbox.
            match sync::copy_my_api_dir(
                &srv_base.join("docs"),
                &client_base.join("docs").join("server-api"),
            ) {
                Ok(n) => c.copied += n,
                Err(e) => c
                    .errors
                    .push(format!("Copy my_api -> {}: {}", client.display_name(), e)),
            }
        }
    }
}

/// F-012: Server → Microservice sync. For each connected microservice-project,
/// resolve its single server-repo and sync REQ-*.md into that repo's
/// docs/server-requirements/, plus rename replays, Case B/C migrations, and
/// api.md / handlers.md propagation onto the parent.
fn sync_server_to_microservice(
    db: &AppDb,
    srv: &Repository,
    srv_base: &Path,
    microservice_ids: &[i64],
    c: &mut SyncCounters,
) {
    for ms_project_id in microservice_ids {
        let ms_server_repo = match db.server_repo_of_microservice(*ms_project_id) {
            Ok(r) => r,
            Err(e) => {
                c.errors
                    .push(format!("Microservice project {}: {}", ms_project_id, e));
                continue;
            }
        };
        let ms_path = match ms_server_repo.local_path.as_deref() {
            Some(p) => p,
            None => {
                c.errors.push(format!(
                    "Microservice server-repo {} has no local_path",
                    ms_server_repo.display_name()
                ));
                continue;
            }
        };
        let ms_base = Path::new(ms_path);
        // B-001: skip microservices whose folder was moved/deleted.
        if let Err(e) = sync::ensure_root_exists(ms_base) {
            c.errors.push(format!(
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
                c.errors.push(format!(
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
                Ok(true) => c.migrated += 1,
                Ok(false) => {}
                Err(e) => c.errors.push(format!(
                    "Case B migrate {} → {}: {}",
                    ms_name, ms_canonical, e
                )),
            }
            c.errors.append(&mut case_b_warnings);
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
                        Ok(sync::RenameOutcome::Renamed) => c.migrated += 1,
                        Ok(sync::RenameOutcome::NoOp) => {}
                        Ok(sync::RenameOutcome::Collision) => c.errors.push(format!(
                            "Rename collision on server side: both {}/ and {}/ exist under microservice-requirements — manual intervention needed",
                            r.old_canonical, r.new_canonical
                        )),
                        Err(e) => c.errors.push(format!(
                            "Rename replay {} → {} on server: {}",
                            r.old_canonical, r.new_canonical, e
                        )),
                    }
                }
            }
            Err(e) => c.errors.push(format!(
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
                    match sync::replay_rename_in_dir(&ms_api_parent, &r.old_name, &r.new_name) {
                        Ok(sync::RenameOutcome::Renamed) => c.migrated += 1,
                        Ok(sync::RenameOutcome::NoOp) => {}
                        Ok(sync::RenameOutcome::Collision) => c.errors.push(format!(
                            "Rename collision on parent side: both {}/ and {}/ exist under microservice-api — manual intervention needed",
                            r.old_name, r.new_name
                        )),
                        Err(e) => c.errors.push(format!(
                            "Project rename replay {} → {} on parent: {}",
                            r.old_name, r.new_name, e
                        )),
                    }
                }
            }
            Err(e) => c.errors.push(format!(
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
                        Ok(sync::RenameOutcome::Renamed) => c.migrated += 1,
                        Ok(sync::RenameOutcome::NoOp) => {}
                        Ok(sync::RenameOutcome::Collision) => c.errors.push(format!(
                            "Rename collision on microservice side: both {}/ and {}/ exist under server-requirements — manual intervention needed",
                            r.old_canonical, r.new_canonical
                        )),
                        Err(e) => c.errors.push(format!(
                            "Rename replay {} → {} on microservice: {}",
                            r.old_canonical, r.new_canonical, e
                        )),
                    }
                }
            }
            Err(e) => c
                .errors
                .push(format!("List renames for server {}: {}", srv.id, e)),
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
                    parent_candidates.push((p_srv_repo.canonical_folder_name(), parent_req_dir));
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
            match sync::migrate_flat_to_nested(&ms_srv_parent, lookup, &mut case_c_warnings) {
                Ok(n) => c.migrated += n,
                Err(e) => c.errors.push(format!("Case C migration: {}", e)),
            }
            c.errors.append(&mut case_c_warnings);
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
                Ok(true) => c.copied += 1,
                Ok(false) => {}
                Err(e) => c
                    .errors
                    .push(format!("Copy {} -> microservice: {}", req_file, e)),
            }
        }

        // Copy *.response.md from microservice (source of truth) back to server.
        // Overwrite on change so recipient edits propagate to the sender.
        for resp_file in sync::scan_responses(&ms_srv_dir) {
            let src = ms_srv_dir.join(&resp_file);
            let dst = srv_ms_dir.join(&resp_file);
            match sync::copy_file_if_changed(&src, &dst) {
                Ok(true) => c.responses += 1,
                Ok(false) => {}
                Err(e) => c
                    .errors
                    .push(format!("Copy {} -> server: {}", resp_file, e)),
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
                Ok(true) => c.copied += 1,
                Ok(false) => {}
                Err(e) => c.errors.push(format!(
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
                Ok(true) => c.copied += 1,
                Ok(false) => {}
                Err(e) => c.errors.push(format!(
                    "Copy ms handlers.md from {}: {}",
                    ms_server_repo.display_name(),
                    e
                )),
            }
        }

        // T-000139: blind-copy the microservice's docs/my_api/*.md into the parent's microservice-api/<ms_name> inbox.
        match sync::copy_my_api_dir(
            &ms_base.join("docs"),
            &srv_base
                .join("docs")
                .join("microservice-api")
                .join(&ms_name),
        ) {
            Ok(n) => c.copied += n,
            Err(e) => c.errors.push(format!(
                "Copy ms my_api from {}: {}",
                ms_server_repo.display_name(),
                e
            )),
        }
    }
}

/// B-000019/B-000020: MS-driven sync — when the current project is a
/// microservice, fan out to each connected parent server. Mirrors the
/// parent-driven block but with the MS as initiator. Without this, pressing
/// Sync on an MS project is a no-op (its clients/microservices loops are
/// empty), so api.md edits never propagate and parents stay out of sync until
/// they happen to trigger Sync themselves. Rename-replay is intentionally not
/// duplicated here — parent-driven sync remains the authority for that, this
/// block does the steady-state file copies only.
fn sync_microservice_to_parents(
    db: &AppDb,
    project_id: i64,
    ms_server: &Repository,
    c: &mut SyncCounters,
) {
    if let Some(ref ms_path) = ms_server.local_path {
        let ms_base = Path::new(ms_path);
        if let Err(e) = sync::ensure_root_exists(ms_base) {
            c.errors
                .push(format!("Microservice {}: {}", ms_server.display_name(), e));
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
                        c.errors
                            .push(format!("Parent {} list repos: {}", parent_project.name, e));
                        continue;
                    }
                };
                let Some(parent_server) = parent_repos
                    .iter()
                    .find(|r| r.role.as_deref() == Some("server"))
                else {
                    c.errors.push(format!(
                        "Parent {}: no server-repo found",
                        parent_project.name
                    ));
                    continue;
                };
                let Some(ref parent_local) = parent_server.local_path else {
                    c.errors.push(format!(
                        "Parent {} ({}): server-repo has no local_path",
                        parent_project.name,
                        parent_server.display_name()
                    ));
                    continue;
                };
                let parent_base = Path::new(parent_local);
                if let Err(e) = sync::ensure_root_exists(parent_base) {
                    c.errors.push(format!(
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
                        Ok(true) => c.copied += 1,
                        Ok(false) => {}
                        Err(e) => c.errors.push(format!(
                            "Copy {} -> parent {}: {}",
                            filename, parent_project.name, e
                        )),
                    }
                }

                // T-000139: blind-copy the ms's docs/my_api/*.md into the parent's microservice-api/<ms_project_name> inbox.
                match sync::copy_my_api_dir(
                    &ms_base.join("docs"),
                    &parent_base
                        .join("docs")
                        .join("microservice-api")
                        .join(&ms_project_name),
                ) {
                    Ok(n) => c.copied += n,
                    Err(e) => c.errors.push(format!("Copy ms my_api -> parent: {}", e)),
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
                        Ok(true) => c.copied += 1,
                        Ok(false) => {}
                        Err(e) => c.errors.push(format!(
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
                        Ok(true) => c.responses += 1,
                        Ok(false) => {}
                        Err(e) => c.errors.push(format!(
                            "Copy {} from MS to parent {}: {}",
                            resp_file, parent_project.name, e
                        )),
                    }
                }
            }
        }
    }
}

/// Orchestrator for a full project sync — the body of the former
/// `sync_project` Tauri command. Writes Phase-0 skeletons to every repo, then
/// runs the directional REQ / api / handlers copies based on project topology.
pub fn run_project_sync(db: &AppDb, project_id: i64) -> Result<SyncResult, String> {
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

    let mut c = SyncCounters::new();

    // 0.10.0 pre-phase: write project.md + CLAUDE.md section + .gitignore + .gitattributes to all repos.
    // 0.11.0 extends with todo.md + bug-reports.md skeletons.
    let tpl = load_skeleton_templates(db);

    for repo in &all_repos {
        let label = repo.display_name();
        let Some(path) = repo.local_path.as_deref() else {
            // B-000002: surface silently-skipped repos so user sees why project.md /
            // CLAUDE.md / .gitignore weren't written for this repo.
            c.errors.push(format!(
                "Repo {}: no local_path set — project.md / CLAUDE.md / .gitignore skipped",
                label
            ));
            continue;
        };
        let base = Path::new(path);
        if let Err(e) = sync::ensure_root_exists(base) {
            c.errors.push(format!(
                "Repo {}: {} — project.md / CLAUDE.md / .gitignore skipped",
                label, e
            ));
            continue;
        }
        write_repo_skeletons(
            db,
            project_id,
            base,
            repo.role.as_deref(),
            &label,
            &tpl,
            &mut c,
        );
    }
    for ms_id in &microservice_ids {
        let Ok(ms_server) = db.server_repo_of_microservice(*ms_id) else {
            c.errors.push(format!(
                "Microservice project {}: no server-repo resolved — project.md skipped",
                ms_id
            ));
            continue;
        };
        let label = ms_server.display_name();
        let Some(path) = ms_server.local_path.as_deref() else {
            c.errors.push(format!(
                "Microservice {}: no local_path set — project.md skipped",
                label
            ));
            continue;
        };
        let base = Path::new(path);
        if let Err(e) = sync::ensure_root_exists(base) {
            c.errors.push(format!(
                "Microservice {}: {} — project.md skipped",
                label, e
            ));
            continue;
        }
        write_repo_skeletons(
            db,
            *ms_id,
            base,
            ms_server.role.as_deref(),
            &label,
            &tpl,
            &mut c,
        );
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
            c.errors.push("No server found in project".to_string());
        } else if clients.is_empty() {
            c.errors.push("No clients found in project".to_string());
        }
    }

    if let Some(srv) = server {
        if srv.local_path.is_none() {
            c.errors
                .push(format!("Server {} has no local_path", srv.display_name()));
        }
        if let Some(ref srv_path) = srv.local_path {
            let srv_base = Path::new(srv_path);

            // B-001: guard — do not silently recreate a moved/deleted server folder.
            if let Err(e) = sync::ensure_root_exists(srv_base) {
                c.errors
                    .push(format!("Server {}: {}", srv.display_name(), e));
                return Ok(c.into_result());
            }

            sync_client_to_server(db, srv_base, &clients, &mut c);
            sync_server_to_microservice(db, srv, srv_base, &microservice_ids, &mut c);
        }
    }

    if project_type == "microservice" {
        if let Some(ms_server) = server {
            sync_microservice_to_parents(db, project_id, ms_server, &mut c);
        }
    }

    // v0.20.0: record sync event. SyncResult fields per models.rs — copied + responses + migrated
    let total_changes = (c.copied + c.responses + c.migrated) as i64;
    let _ = db.insert_sync_event(
        None,
        "project_sync",
        &chrono::Utc::now().to_rfc3339(),
        total_changes,
        Some(&format!(r#"{{"project_id":{}}}"#, project_id)),
    );

    Ok(c.into_result())
}
