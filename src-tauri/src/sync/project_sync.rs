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
    /// F-000039: base REQ filenames whose pair was auto-closed via `.impl.md`.
    auto_closed: Vec<String>,
    /// T-000137: display names of repos that received an SDH auto-commit of
    /// synced cross-repo files this pass.
    committed: Vec<String>,
}

impl SyncCounters {
    fn new() -> Self {
        Self {
            copied: 0,
            responses: 0,
            migrated: 0,
            errors: Vec::new(),
            auto_closed: Vec::new(),
            committed: Vec::new(),
        }
    }

    fn into_result(self) -> SyncResult {
        SyncResult {
            copied: self.copied,
            responses: self.responses,
            migrated: self.migrated,
            errors: self.errors,
            auto_closed: self.auto_closed,
            committed: self.committed,
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
    if let Err(e) = sync::render_skills_to_repo(db, base) {
        c.errors.push(format!("sdh_skills for {}: {}", label, e));
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

            // F-000039: auto-close pairs the client (sender) acknowledged via .impl.md.
            match sync::auto_close_impl_pairs(&client_req_dir, &srv_client_dir) {
                Ok(closed) => c.auto_closed.extend(closed),
                Err(e) => c.errors.push(format!(
                    "Auto-close impl -> {}: {}",
                    client.display_name(),
                    e
                )),
            }

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

/// F-012: Server → Microservice sync. For each connected child-project, resolve
/// its single server-repo and sync REQ-*.md into that repo's
/// docs/server-requirements/, plus rename replays, Case B/C migrations, and
/// api.md / handlers.md propagation onto the parent.
///
/// T-000172: reused for connected **codices** too — the consumer↔codex REQ pair
/// is the same shape as parent↔microservice (anchored on each side's server
/// repo). `skip_api_contract` is set for codices: a svod has no `docs/api.md` /
/// `docs/my_api/`, so the api-contract propagation is skipped while the REQ /
/// response / rename-replay machinery runs identically.
fn sync_server_to_microservice(
    db: &AppDb,
    srv: &Repository,
    srv_base: &Path,
    microservice_ids: &[i64],
    skip_api_contract: bool,
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

        // F-000039: auto-close pairs the server (sender) acknowledged via .impl.md.
        match sync::auto_close_impl_pairs(&srv_ms_dir, &ms_srv_dir) {
            Ok(closed) => c.auto_closed.extend(closed),
            Err(e) => c
                .errors
                .push(format!("Auto-close impl (server->ms): {}", e)),
        }

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

        // 0.9.0: Microservice → Parent server — api.md + handlers.md + my_api.
        // Target on parent: docs/microservice-api/<ms-project-name>/{api,handlers}.md.
        // T-000172: skipped for codices — a svod has no api contract, only its
        // practices (delivered by the separate broadcast pass).
        if !skip_api_contract {
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
}

/// B-000019/B-000020: MS-driven sync — when the current project is a
/// microservice, fan out to each connected parent server. Mirrors the
/// parent-driven block but with the MS as initiator. Without this, pressing
/// Sync on an MS project is a no-op (its clients/microservices loops are
/// empty), so api.md edits never propagate and parents stay out of sync until
/// they happen to trigger Sync themselves. Rename-replay is intentionally not
/// duplicated here — parent-driven sync remains the authority for that, this
/// block does the steady-state file copies only.
///
/// T-000172: also driven for a **codex** initiator (its consumers are the
/// "parents"). `skip_api` is set for codices — a svod pushes no api.md/my_api to
/// consumers (that's the broadcast pass), only its REQ responses.
fn sync_microservice_to_parents(
    db: &AppDb,
    project_id: i64,
    ms_server: &Repository,
    skip_api: bool,
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

                // MS → parent: api.md + handlers.md + my_api. T-000172: skipped
                // for a codex initiator — the svod carries no api contract.
                if !skip_api {
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

                // F-000039: auto-close pairs the parent server (sender) acknowledged via .impl.md.
                match sync::auto_close_impl_pairs(&parent_ms_dir, &ms_parent_dir) {
                    Ok(closed) => c.auto_closed.extend(closed),
                    Err(e) => c
                        .errors
                        .push(format!("Auto-close impl (parent->ms): {}", e)),
                }

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

/// Codex broadcast pass. Two directions, both idempotent:
/// - Consumer-driven: this project pulls every connected codex's svod into its
///   own same-role repos.
/// - Codex-driven: if this project IS a codex, it pushes its svod into every
///   connected consumer.
///
/// Folds each per-(codex,consumer) outcome into the shared counters — mirror
/// writes into `copied`, prunes into `migrated` (a structural change to the
/// consumer's mirror), errors appended.
fn run_codex_broadcast(db: &AppDb, project_id: i64, project_type: &str, c: &mut SyncCounters) {
    let mut fold = |o: sync::CodexBroadcastOutcome| {
        c.copied += o.copied;
        c.migrated += o.pruned;
        c.errors.extend(o.errors);
    };
    // Consumer-driven: pull each connected codex into this project.
    for codex_id in db.list_connected_codices(project_id).unwrap_or_default() {
        fold(sync::broadcast_codex_to_consumer(db, codex_id, project_id));
    }
    // Codex-driven: push this codex's svod into each connected consumer.
    if project_type == "codex" {
        for consumer in db
            .list_parents_of_microservice(project_id)
            .unwrap_or_default()
        {
            fold(sync::broadcast_codex_to_consumer(
                db,
                project_id,
                consumer.id,
            ));
        }
    }
}

/// T-000175: role-agnostic single consumer→codex REQ edge. `sender` is always
/// the consumer (raises the REQ), `recipient` is always the codex (answers with
/// a `.response.md`). Caller guarantees both repos have a `local_path` and their
/// roots exist. Mirrors the steady-state part of `sync_server_to_microservice`
/// (rename replay → auto-close → REQ copy → response copy-back).
///
/// Case B (flat→canonical server-side folder) and Case C (flat→nested ms-side
/// folder) migrations, plus api/handlers/my_api propagation, are intentionally
/// omitted here: codex REQ folders are born nested/canonical (no legacy flat
/// layout to migrate), and a svod carries no api contract (its practices ship
/// via the separate broadcast pass).
fn sync_req_pair(db: &AppDb, sender: &Repository, recipient: &Repository, c: &mut SyncCounters) {
    let sender_base = Path::new(sender.local_path.as_deref().unwrap());
    let recipient_base = Path::new(recipient.local_path.as_deref().unwrap());

    let sender_req_parent = sender_base.join("docs").join("microservice-requirements");
    let recipient_srv_parent = recipient_base.join("docs").join("server-requirements");

    let sender_dir = sender_req_parent.join(recipient.canonical_folder_name());
    let recipient_dir = recipient_srv_parent.join(sender.canonical_folder_name());

    // Replay recipient (codex) repo renames on the sender's microservice-requirements/ side.
    match db.list_renames_for_repo(recipient.id) {
        Ok(renames) => {
            for r in renames {
                match sync::replay_rename_in_dir(
                    &sender_req_parent,
                    &r.old_canonical,
                    &r.new_canonical,
                ) {
                    Ok(sync::RenameOutcome::Renamed) => c.migrated += 1,
                    Ok(sync::RenameOutcome::NoOp) => {}
                    Ok(sync::RenameOutcome::Collision) => c.errors.push(format!(
                        "Rename collision on consumer side: both {}/ and {}/ exist under microservice-requirements — manual intervention needed",
                        r.old_canonical, r.new_canonical
                    )),
                    Err(e) => c.errors.push(format!(
                        "Rename replay {} → {} on consumer: {}",
                        r.old_canonical, r.new_canonical, e
                    )),
                }
            }
        }
        Err(e) => c.errors.push(format!(
            "List renames for codex repo {}: {}",
            recipient.id, e
        )),
    }

    // Replay sender (consumer) repo renames on the recipient's server-requirements/ side.
    match db.list_renames_for_repo(sender.id) {
        Ok(renames) => {
            for r in renames {
                match sync::replay_rename_in_dir(
                    &recipient_srv_parent,
                    &r.old_canonical,
                    &r.new_canonical,
                ) {
                    Ok(sync::RenameOutcome::Renamed) => c.migrated += 1,
                    Ok(sync::RenameOutcome::NoOp) => {}
                    Ok(sync::RenameOutcome::Collision) => c.errors.push(format!(
                        "Rename collision on codex side: both {}/ and {}/ exist under server-requirements — manual intervention needed",
                        r.old_canonical, r.new_canonical
                    )),
                    Err(e) => c.errors.push(format!(
                        "Rename replay {} → {} on codex: {}",
                        r.old_canonical, r.new_canonical, e
                    )),
                }
            }
        }
        Err(e) => c.errors.push(format!(
            "List renames for consumer repo {}: {}",
            sender.id, e
        )),
    }

    // F-000039: auto-close pairs the consumer (sender) acknowledged via .impl.md.
    match sync::auto_close_impl_pairs(&sender_dir, &recipient_dir) {
        Ok(closed) => c.auto_closed.extend(closed),
        Err(e) => c
            .errors
            .push(format!("Auto-close impl (consumer->codex): {}", e)),
    }

    // Copy REQ-*.md from consumer (source of truth) to codex.
    for req_file in sync::scan_requirements(&sender_dir) {
        let src = sender_dir.join(&req_file);
        let dst = recipient_dir.join(&req_file);
        match sync::copy_file_if_changed(&src, &dst) {
            Ok(true) => c.copied += 1,
            Ok(false) => {}
            Err(e) => c.errors.push(format!("Copy {} -> codex: {}", req_file, e)),
        }
    }

    // Copy *.response.md from codex (source of truth) back to consumer.
    for resp_file in sync::scan_responses(&recipient_dir) {
        let src = recipient_dir.join(&resp_file);
        let dst = sender_dir.join(&resp_file);
        match sync::copy_file_if_changed(&src, &dst) {
            Ok(true) => c.responses += 1,
            Ok(false) => {}
            Err(e) => c
                .errors
                .push(format!("Copy {} -> consumer: {}", resp_file, e)),
        }
    }
}

/// T-000175: codex REQ channels, per-lane. Unlike the broadcast (svod content),
/// REQ/response between a consumer and a codex now follows the *same lane edges*
/// as the broadcast — `server↔server` and `admin_client↔admin_client` — instead
/// of being anchored solely on each side's server repo. An admin-only codex (no
/// server repo) therefore gets a working REQ channel on its admin lane.
///
/// Both orientations keep the consumer as the sender and the codex as the
/// recipient of a `sync_req_pair` edge.
/// T-000192: InOutBox channels between equals. For every repo of this project,
/// sync the channel with each of its peers — same-project same-role pairs plus
/// explicitly linked cross-project ones. Folds file writes into `copied`,
/// prunes and marker teardowns into `migrated` (both are structural changes to
/// the counterpart's folder), and closed exchanges into `auto_closed` alongside
/// the REQ pairs the same field already carries.
fn run_peer_channels(db: &AppDb, project_id: i64, c: &mut SyncCounters) {
    let o = sync::run_peer_channels(db, project_id);
    c.copied += o.copied;
    c.migrated += o.pruned;
    c.auto_closed.extend(o.closed);
    c.errors.extend(o.errors);
}

fn run_codex_req_channels(db: &AppDb, project_id: i64, project_type: &str, c: &mut SyncCounters) {
    const LANES: [&str; 2] = ["server", "admin_client"];

    // A repo passes the guard when it has a local_path and its root exists.
    // Returns None (after pushing an error) otherwise.
    let guard = |repo: &Repository, c: &mut SyncCounters| -> bool {
        let Some(ref lp) = repo.local_path else {
            c.errors.push(format!(
                "Codex REQ: {} repo {} has no local_path",
                repo.role.as_deref().unwrap_or("?"),
                repo.display_name()
            ));
            return false;
        };
        let base = Path::new(lp);
        if sync::ensure_root_exists(base).is_err() {
            c.errors.push(format!(
                "Codex REQ: {} repo {} root missing",
                repo.role.as_deref().unwrap_or("?"),
                repo.display_name()
            ));
            return false;
        }
        true
    };

    // Consumer-driven: this project consumes every connected codex — one edge
    // per lane where both sides have a matching-role repo.
    for codex_id in db.list_connected_codices(project_id).unwrap_or_default() {
        for lane in LANES {
            let Ok(consumer_repo) = db.repo_of_project_with_role(project_id, lane) else {
                continue;
            };
            let Ok(codex_repo) = db.repo_of_project_with_role(codex_id, lane) else {
                continue;
            };
            if !guard(&consumer_repo, c) || !guard(&codex_repo, c) {
                continue;
            }
            sync_req_pair(db, &consumer_repo, &codex_repo, c);
        }
    }

    // Codex-driven: if this project IS a codex, drive the same per-lane edges
    // toward each connected consumer.
    if project_type == "codex" {
        for consumer in db
            .list_parents_of_microservice(project_id)
            .unwrap_or_default()
        {
            for lane in LANES {
                let Ok(codex_repo) = db.repo_of_project_with_role(project_id, lane) else {
                    continue;
                };
                let Ok(consumer_repo) = db.repo_of_project_with_role(consumer.id, lane) else {
                    continue;
                };
                if !guard(&consumer_repo, c) || !guard(&codex_repo, c) {
                    continue;
                }
                sync_req_pair(db, &consumer_repo, &codex_repo, c);
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
            sync_server_to_microservice(db, srv, srv_base, &microservice_ids, false, &mut c);
        }
    }

    // Reverse REQ direction from a microservice's own side: it pushes responses
    // to / pulls REQs from its parent consumers. Codex REQ channels are handled
    // separately (per-lane) by run_codex_req_channels below.
    if project_type == "microservice" {
        if let Some(ms_server) = server {
            sync_microservice_to_parents(db, project_id, ms_server, false, &mut c);
        }
    }

    // T-000165: codex broadcast — pull connected codices' svods into this
    // project, and (if this project is a codex) push its svod to consumers.
    run_codex_broadcast(db, project_id, &project_type, &mut c);

    // T-000175: codex REQ channels — per-lane (server↔server, admin↔admin)
    // consumer→codex REQ pairs, so an admin-only codex isn't server-blind.
    run_codex_req_channels(db, project_id, &project_type, &mut c);

    // T-000192: peer channels (InOutBox) — client<->client, server<->server and
    // explicitly linked cross-project pairs. Role-agnostic and independent of
    // project topology, so it runs after the hierarchical passes.
    run_peer_channels(db, project_id, &mut c);

    // T-000137: auto-commit synced cross-repo files in opted-in repos (portfolio-wide;
    // pathspec-scoped, so untouched repos and the dev's own WIP are no-ops).
    if let Some(git) = crate::git_ops::check_git_available() {
        match db.list_autocommit_repos() {
            Ok(repos) => {
                for (repo, branch) in repos {
                    let Some(ref lp) = repo.local_path else {
                        continue;
                    };
                    let path = std::path::Path::new(lp);
                    if !crate::git_ops::is_git_repo(path) {
                        continue;
                    }
                    match crate::sync::autocommit_repo(&git, path, &branch) {
                        Ok(crate::git_ops::CommitOutcome::Committed { .. }) => {
                            c.committed.push(repo.display_name());
                        }
                        Ok(crate::git_ops::CommitOutcome::WrongBranch { current, expected }) => {
                            c.errors.push(format!(
                                "Auto-commit skipped for {}: on branch {:?}, expected {}",
                                repo.display_name(),
                                current,
                                expected
                            ));
                        }
                        Ok(crate::git_ops::CommitOutcome::NothingToCommit) => {}
                        Err(e) => {
                            c.errors
                                .push(format!("Auto-commit {}: {}", repo.display_name(), e))
                        }
                    }
                }
            }
            Err(e) => c
                .errors
                .push(format!("Auto-commit: failed to list repos: {}", e)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // A local repo's canonical_folder_name falls back to its description
    // (github_name is NULL for local repos), so the display_name passed here is
    // the folder name used in the REQ paths.
    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    /// T-000172: a REQ raised on a consumer for a connected codex is delivered to
    /// the codex's server-lane repo, and the codex's response comes back — the
    /// same parent↔microservice REQ round-trip, over a codex edge, with the
    /// api-contract propagation skipped.
    #[test]
    fn test_codex_req_roundtrip_and_api_skip() {
        let db = make_db();
        let consumer_dir = TempDir::new().unwrap();
        let codex_dir = TempDir::new().unwrap();
        let consumer_base = consumer_dir.path();
        let codex_base = codex_dir.path();

        // Consumer (standard) with a server repo; codex with a server-lane repo.
        let consumer = db.create_project("Consumer", None, "standard").unwrap();
        db.insert_local_repository(
            consumer_base.to_str().unwrap(),
            "consumer-backend",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();
        let codex = db.create_project("Practices", None, "codex").unwrap();
        db.insert_local_repository(
            codex_base.to_str().unwrap(),
            "practices-server",
            Some(codex.id),
            Some("server"),
        )
        .unwrap();
        db.connect_codex(consumer.id, codex.id).unwrap();

        // Consumer raises a REQ toward the codex (canonical folder = codex server
        // repo's name), and the codex happens to also have an api.md that must NOT
        // propagate (api-contract is skipped for codices).
        let consumer_req_dir = consumer_base
            .join("docs")
            .join("microservice-requirements")
            .join("practices-server");
        fs::create_dir_all(&consumer_req_dir).unwrap();
        fs::write(
            consumer_req_dir.join("REQ-001_add-retry-rule.md"),
            "Please add a retry idempotency rule.",
        )
        .unwrap();
        fs::create_dir_all(codex_base.join("docs")).unwrap();
        fs::write(
            codex_base.join("docs").join("api.md"),
            "not a real contract",
        )
        .unwrap();

        run_project_sync(&db, consumer.id).unwrap();

        // REQ landed on the codex side under server-requirements/<consumer-canonical>/.
        let codex_req = codex_base
            .join("docs")
            .join("server-requirements")
            .join("consumer-backend")
            .join("REQ-001_add-retry-rule.md");
        assert!(
            codex_req.exists(),
            "consumer's REQ must be delivered to the codex's server-requirements"
        );

        // api-contract was NOT propagated — no microservice-api inbox for the codex.
        assert!(
            !consumer_base
                .join("docs")
                .join("microservice-api")
                .join("Practices")
                .join("api.md")
                .exists(),
            "a codex must not push an api.md to the consumer"
        );

        // The codex answers with a response; next sync brings it back to the consumer.
        fs::write(
            codex_base
                .join("docs")
                .join("server-requirements")
                .join("consumer-backend")
                .join("REQ-001_add-retry-rule.response.md"),
            "Added rules/retry.md — idempotency key now required.",
        )
        .unwrap();

        run_project_sync(&db, consumer.id).unwrap();

        let consumer_response = consumer_req_dir.join("REQ-001_add-retry-rule.response.md");
        assert!(
            consumer_response.exists(),
            "the codex's response must be copied back to the consumer"
        );
    }

    /// T-000175: a REQ over the admin_client lane round-trips when the codex has
    /// ONLY an admin repo (no server repo). Proves the REQ channel follows lane
    /// edges, not the server anchor.
    #[test]
    fn test_codex_req_admin_lane_roundtrip() {
        let db = make_db();
        let consumer_dir = TempDir::new().unwrap();
        let codex_dir = TempDir::new().unwrap();
        let consumer_base = consumer_dir.path();
        let codex_base = codex_dir.path();

        let consumer = db.create_project("Consumer", None, "standard").unwrap();
        db.insert_local_repository(
            consumer_base.to_str().unwrap(),
            "consumer-admin",
            Some(consumer.id),
            Some("admin_client"),
        )
        .unwrap();
        let codex = db.create_project("Practices", None, "codex").unwrap();
        db.insert_local_repository(
            codex_base.to_str().unwrap(),
            "practices-admin",
            Some(codex.id),
            Some("admin_client"),
        )
        .unwrap();
        db.connect_codex(consumer.id, codex.id).unwrap();

        // Consumer raises a REQ toward the codex on the admin lane.
        let consumer_req_dir = consumer_base
            .join("docs")
            .join("microservice-requirements")
            .join("practices-admin");
        fs::create_dir_all(&consumer_req_dir).unwrap();
        fs::write(
            consumer_req_dir.join("REQ-001_admin-rule.md"),
            "Please add an admin-panel accessibility rule.",
        )
        .unwrap();

        run_project_sync(&db, consumer.id).unwrap();

        // REQ landed on the codex's admin repo under server-requirements/<consumer-admin>/.
        let codex_req = codex_base
            .join("docs")
            .join("server-requirements")
            .join("consumer-admin")
            .join("REQ-001_admin-rule.md");
        assert!(
            codex_req.exists(),
            "consumer's admin-lane REQ must be delivered to the codex's admin repo"
        );

        // Codex answers; next sync brings it back.
        fs::write(
            codex_base
                .join("docs")
                .join("server-requirements")
                .join("consumer-admin")
                .join("REQ-001_admin-rule.response.md"),
            "Added rules/admin-a11y.md.",
        )
        .unwrap();

        run_project_sync(&db, consumer.id).unwrap();

        let consumer_response = consumer_req_dir.join("REQ-001_admin-rule.response.md");
        assert!(
            consumer_response.exists(),
            "the codex's admin-lane response must be copied back to the consumer"
        );
    }

    /// T-000175: an admin-only codex must not cause a server-requirements folder
    /// in the consumer's server repo — the server lane has no codex counterpart,
    /// so only the admin edge delivers.
    #[test]
    fn test_codex_req_admin_only_no_server_folder() {
        let db = make_db();
        let consumer_srv_dir = TempDir::new().unwrap();
        let consumer_adm_dir = TempDir::new().unwrap();
        let codex_dir = TempDir::new().unwrap();
        let consumer_srv_base = consumer_srv_dir.path();
        let consumer_adm_base = consumer_adm_dir.path();
        let codex_base = codex_dir.path();

        let consumer = db.create_project("Consumer", None, "standard").unwrap();
        db.insert_local_repository(
            consumer_srv_base.to_str().unwrap(),
            "consumer-backend",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();
        db.insert_local_repository(
            consumer_adm_base.to_str().unwrap(),
            "consumer-admin",
            Some(consumer.id),
            Some("admin_client"),
        )
        .unwrap();
        let codex = db.create_project("Practices", None, "codex").unwrap();
        db.insert_local_repository(
            codex_base.to_str().unwrap(),
            "practices-admin",
            Some(codex.id),
            Some("admin_client"),
        )
        .unwrap();
        db.connect_codex(consumer.id, codex.id).unwrap();

        // REQ raised on the admin lane only.
        let consumer_req_dir = consumer_adm_base
            .join("docs")
            .join("microservice-requirements")
            .join("practices-admin");
        fs::create_dir_all(&consumer_req_dir).unwrap();
        fs::write(
            consumer_req_dir.join("REQ-010_admin-rule.md"),
            "Admin-only rule.",
        )
        .unwrap();

        run_project_sync(&db, consumer.id).unwrap();

        // Admin edge delivered.
        assert!(
            codex_base
                .join("docs")
                .join("server-requirements")
                .join("consumer-admin")
                .join("REQ-010_admin-rule.md")
                .exists(),
            "admin-lane REQ must be delivered to the admin-only codex"
        );

        // No server-requirements folder was created in the consumer's server repo
        // for this codex (the server lane has no codex counterpart).
        assert!(
            !consumer_srv_base
                .join("docs")
                .join("server-requirements")
                .exists(),
            "an admin-only codex must not create a server-requirements folder in the consumer server repo"
        );
    }

    /// T-000175: server and admin lanes are independent REQ pairs — a REQ placed
    /// on one lane must not leak into the other lane's folders.
    #[test]
    fn test_codex_req_two_lanes_are_independent() {
        let db = make_db();
        let consumer_srv_dir = TempDir::new().unwrap();
        let consumer_adm_dir = TempDir::new().unwrap();
        let codex_srv_dir = TempDir::new().unwrap();
        let codex_adm_dir = TempDir::new().unwrap();
        let consumer_srv_base = consumer_srv_dir.path();
        let consumer_adm_base = consumer_adm_dir.path();
        let codex_srv_base = codex_srv_dir.path();
        let codex_adm_base = codex_adm_dir.path();

        let consumer = db.create_project("Consumer", None, "standard").unwrap();
        db.insert_local_repository(
            consumer_srv_base.to_str().unwrap(),
            "consumer-backend",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();
        db.insert_local_repository(
            consumer_adm_base.to_str().unwrap(),
            "consumer-admin",
            Some(consumer.id),
            Some("admin_client"),
        )
        .unwrap();
        let codex = db.create_project("Practices", None, "codex").unwrap();
        db.insert_local_repository(
            codex_srv_base.to_str().unwrap(),
            "practices-server",
            Some(codex.id),
            Some("server"),
        )
        .unwrap();
        db.insert_local_repository(
            codex_adm_base.to_str().unwrap(),
            "practices-admin",
            Some(codex.id),
            Some("admin_client"),
        )
        .unwrap();
        db.connect_codex(consumer.id, codex.id).unwrap();

        // REQ on the server edge.
        let consumer_srv_req_dir = consumer_srv_base
            .join("docs")
            .join("microservice-requirements")
            .join("practices-server");
        fs::create_dir_all(&consumer_srv_req_dir).unwrap();
        fs::write(
            consumer_srv_req_dir.join("REQ-100_server-rule.md"),
            "Server-lane rule.",
        )
        .unwrap();

        // REQ on the admin edge.
        let consumer_adm_req_dir = consumer_adm_base
            .join("docs")
            .join("microservice-requirements")
            .join("practices-admin");
        fs::create_dir_all(&consumer_adm_req_dir).unwrap();
        fs::write(
            consumer_adm_req_dir.join("REQ-200_admin-rule.md"),
            "Admin-lane rule.",
        )
        .unwrap();

        run_project_sync(&db, consumer.id).unwrap();

        // Each REQ landed only in its own lane's folder.
        assert!(
            codex_srv_base
                .join("docs")
                .join("server-requirements")
                .join("consumer-backend")
                .join("REQ-100_server-rule.md")
                .exists(),
            "server-lane REQ must land in the codex server repo"
        );
        assert!(
            codex_adm_base
                .join("docs")
                .join("server-requirements")
                .join("consumer-admin")
                .join("REQ-200_admin-rule.md")
                .exists(),
            "admin-lane REQ must land in the codex admin repo"
        );

        // No cross-contamination between lanes.
        assert!(
            !codex_adm_base
                .join("docs")
                .join("server-requirements")
                .join("consumer-backend")
                .join("REQ-100_server-rule.md")
                .exists(),
            "server-lane REQ must NOT appear in the admin repo"
        );
        assert!(
            !codex_srv_base
                .join("docs")
                .join("server-requirements")
                .join("consumer-admin")
                .join("REQ-200_admin-rule.md")
                .exists(),
            "admin-lane REQ must NOT appear in the server repo"
        );
    }

    #[test]
    fn test_peer_channel_derived_pair_end_to_end() {
        // T-000192: two client repos in one project are peers by role alone.
        // A full project sync must create the channel on both sides, carry an
        // outbox file into the peer's inbox, and honour a `.done.md` teardown.
        let db = make_db();
        let a_dir = TempDir::new().unwrap();
        let b_dir = TempDir::new().unwrap();
        let other_dir = TempDir::new().unwrap();

        let p = db.create_project("Peers", None, "standard").unwrap();
        db.insert_local_repository(
            a_dir.path().to_str().unwrap(),
            "alpha",
            Some(p.id),
            Some("client"),
        )
        .unwrap();
        db.insert_local_repository(
            b_dir.path().to_str().unwrap(),
            "beta",
            Some(p.id),
            Some("client"),
        )
        .unwrap();
        // A repo with role `other` must not be pulled into the channel.
        db.insert_local_repository(
            other_dir.path().to_str().unwrap(),
            "gamma",
            Some(p.id),
            Some("other"),
        )
        .unwrap();

        run_project_sync(&db, p.id).unwrap();

        let a_out = a_dir.path().join("docs/InOutBox/outbox/beta");
        let b_in = b_dir.path().join("docs/InOutBox/inbox/alpha");
        assert!(a_out.is_dir(), "channel folders appear for a derived pair");
        assert!(b_dir.path().join("docs/InOutBox/pact/alpha").is_dir());
        assert!(
            !other_dir.path().join("docs/InOutBox").exists(),
            "role `other` means role-not-set and must never pair"
        );

        fs::write(a_out.join("NOTE-001_hi.md"), "observation").unwrap();
        run_project_sync(&db, p.id).unwrap();
        assert_eq!(
            fs::read_to_string(b_in.join("NOTE-001_hi.md")).unwrap(),
            "observation"
        );

        fs::write(a_out.join("NOTE-001_hi.done.md"), "closed").unwrap();
        let r = run_project_sync(&db, p.id).unwrap();
        assert!(r.auto_closed.contains(&"NOTE-001_hi.md".to_string()));
        assert!(!b_in.join("NOTE-001_hi.md").exists());
        assert!(!a_out.join("NOTE-001_hi.md").exists());

        // Idempotent: another pass neither resurrects nor errors. Only the
        // channel's own errors matter here — a bare test DB has no bundled
        // templates and no server repo, which the other passes report.
        let r = run_project_sync(&db, p.id).unwrap();
        assert!(
            !r.errors.iter().any(|e| e.starts_with("Peer channel")),
            "{:?}",
            r.errors
        );
        assert!(!b_in.join("NOTE-001_hi.md").exists());
    }

    #[test]
    fn test_peer_channel_explicit_link_across_projects() {
        // T-000192: two servers in different products pair only via an explicit
        // link — and the link is what the sync pass walks.
        let db = make_db();
        let a_dir = TempDir::new().unwrap();
        let b_dir = TempDir::new().unwrap();

        let p1 = db.create_project("Lensa", None, "standard").unwrap();
        let p2 = db.create_project("DLemma", None, "standard").unwrap();
        let a = db
            .insert_local_repository(
                a_dir.path().to_str().unwrap(),
                "lensa-srv",
                Some(p1.id),
                Some("server"),
            )
            .unwrap();
        let b = db
            .insert_local_repository(
                b_dir.path().to_str().unwrap(),
                "dlemma-srv",
                Some(p2.id),
                Some("server"),
            )
            .unwrap();

        run_project_sync(&db, p1.id).unwrap();
        assert!(
            !a_dir.path().join("docs/InOutBox").exists(),
            "same role in different projects is not a pair without an explicit link"
        );

        db.add_peer_link(a.id, b.id).unwrap();
        run_project_sync(&db, p1.id).unwrap();
        let a_pact = a_dir.path().join("docs/InOutBox/pact/dlemma-srv");
        let b_pact = b_dir.path().join("docs/InOutBox/pact/lensa-srv");
        assert!(a_pact.is_dir() && b_pact.is_dir());

        // Both sides author into the same pact folder; the union must keep both.
        fs::write(a_pact.join("PACT-001_shared.md"), "proposal").unwrap();
        fs::write(b_pact.join("PACT-001_shared.ack.md"), "agreed").unwrap();
        run_project_sync(&db, p1.id).unwrap();
        for dir in [&a_pact, &b_pact] {
            assert!(dir.join("PACT-001_shared.md").exists());
            assert!(dir.join("PACT-001_shared.ack.md").exists());
        }

        // Unlinking stops future syncing without deleting what was exchanged.
        db.remove_peer_link(a.id, b.id).unwrap();
        fs::write(a_pact.join("PACT-002_next.md"), "second").unwrap();
        run_project_sync(&db, p1.id).unwrap();
        assert!(
            !b_pact.join("PACT-002_next.md").exists(),
            "an unlinked pair must not sync"
        );
        assert!(
            b_pact.join("PACT-001_shared.md").exists(),
            "unlinking must not delete history"
        );
    }
}
