mod crypto;
mod db;
mod export;
mod git_ops;
mod keyring_store;
mod models;
mod sync;
mod template_meta;
mod template_render;
mod template_seeder;

#[allow(unused_imports)]
use chrono;
use db::AppDb;
use std::path::PathBuf;

fn get_db_path() -> PathBuf {
    let local = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let new_dir = local.join("solo-dev-hub");
    let new_db = new_dir.join("data.db");

    // T-000063: one-time copy-once migration from the legacy app dir.
    // We copy (not move) so that if the new build crashes mid-migration the
    // legacy SQLite stays intact as a recovery breadcrumb. The user can
    // delete the legacy folder manually once they're satisfied the rebrand
    // build works. Idempotent: only fires when new doesn't exist yet.
    if !new_db.exists() {
        let legacy_db = local.join("github-repo-manager").join("data.db");
        if legacy_db.exists() {
            std::fs::create_dir_all(&new_dir).ok();
            if let Err(e) = std::fs::copy(&legacy_db, &new_db) {
                eprintln!(
                    "warn: failed to migrate legacy DB {:?} → {:?}: {}",
                    legacy_db, new_db, e
                );
            }
        }
    }

    std::fs::create_dir_all(&new_dir).ok();
    new_db
}

mod commands;

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = AppDb::new(get_db_path()).expect("Failed to initialize database");

    // T-000063: copy legacy PAT from old keyring service to new one. Idempotent;
    // only fires when new service has no entry yet.
    keyring_store::migrate_legacy_pat();

    // Seed bundled templates (e.g. flutter_web) if language is missing in DB.
    if let Err(e) = template_seeder::seed_bundled_templates(&db) {
        eprintln!("Warning: template seeding failed: {}", e);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|_app| {
            // B-000017 v5 (reverted v3+v4 set_icon override): comparing
            // against a sibling Tauri-2 + Svelte app (MySafeSpace) on the
            // same Win11 high-DPI display revealed that the default Tauri
            // window-icon path (NO programmatic set_icon) renders sharp
            // because Tauri/tao does pick the right frame from icon.ico's
            // multi-frame resource for each context (taskbar / alt-tab /
            // title). Earlier explicit set_icon attempts (v1=64×64 PNG,
            // v3=hub-spokes PNG, v4=square-frame PNG) forced a single RGBA
            // bitmap on every DPI context — Windows then downscaled with
            // poor filtering on non-integer ratios. Removing the override
            // restores multi-frame ICO behaviour for free.
            //
            // The original B-000010 comment claimed default Tauri picked
            // "the largest frame and downscaled it" — that was likely true
            // of an earlier tao version; current Tauri 2.x handles
            // multi-frame ICO correctly. icon.ico already has the right
            // frames (16/20/24/32/40/48/64/96/128/256, SDH-crop on small,
            // full logo on large) per the B-000010 rebuild — no changes
            // needed there.
            //
            // Does NOT affect the .exe file icon either way (still
            // multi-frame icon.ico via tauri-bundler's embedded resource).
            //
            // Generator + explored override variants left at
            // docs/superpowers/plans/2026-05-24-sdh-icon-v2.html for
            // history.
            Ok(())
        })
        .manage(db)
        .invoke_handler(tauri::generate_handler![
            // Projects
            commands::project::create_project,
            commands::project::list_projects,
            commands::project::update_project,
            commands::project::delete_project,
            // Repositories
            commands::repo::create_local_repository,
            commands::repo::upsert_repository,
            commands::repo::resolve_merge_with_local,
            commands::repo::force_insert_github_repo,
            commands::repo::assign_repository,
            commands::repo::reorder_project,
            commands::repo::reorder_repo,
            commands::repo::rebalance_repo_group,
            commands::repo::rebalance_projects,
            commands::repo::auto_sort_all,
            commands::repo::list_repos_by_project,
            commands::repo::list_all_repos,
            commands::repo::get_repository,
            commands::repo::get_repository_by_name,
            // PAT / Keyring
            commands::misc::store_pat,
            commands::misc::get_pat,
            commands::misc::delete_pat,
            // local_path
            commands::repo::set_repo_local_path,
            commands::repo::update_repo_description,
            // Repo deletion (B-003)
            commands::repo::delete_repository,
            // F-000041: untrack gitignored files
            commands::repo::check_git_available_for_repo,
            commands::repo::list_gitignored_tracked,
            commands::repo::untrack_files,
            // Workspace scanner
            commands::repo::scan_workspace_for_repos,
            // File-based bugs
            commands::bug::read_bugs_from_file,
            commands::bug::write_bugs_to_file,
            // Bugs (v0.16.0, SQLite SoT)
            commands::bug::ensure_bugs_migrated,
            commands::bug::reconcile_bugs_for_repo,
            commands::bug::reconcile_all_projects,
            commands::bug::read_bugs_from_db,
            commands::bug::count_confirmed_bugs,
            commands::bug::create_bug,
            commands::bug::update_bug_fields,
            commands::bug::delete_bug,
            commands::bug::resolve_bug,
            commands::bug::reject_bug,
            commands::bug::reopen_bug,
            // Microservice connections
            commands::project::connect_microservice,
            commands::project::disconnect_microservice,
            commands::project::list_project_microservices,
            commands::project::list_microservice_projects,
            commands::project::list_parents_of_microservice,
            commands::project::update_project_type,
            commands::project::server_repo_of_microservice,
            // Settings
            commands::misc::get_setting,
            commands::misc::set_setting,
            // Stats / Graph
            commands::dashboard::get_repo_stats_summary,
            commands::dashboard::get_project_stats_summary,
            commands::dashboard::get_project_graph,
            // Dashboard v0.17.0
            commands::dashboard::read_dashboard,
            // Activity feed v0.19.0
            commands::dashboard::read_recent_activity,
            // Timeline v0.20.0
            commands::timeline::read_timeline,
            // Requirements sync
            commands::sync::sync_global_claude_md,
            commands::sync::sync_project,
            commands::sync::init_docs_for_repo,
            commands::sync::list_project_requirements,
            commands::sync::confirm_requirement,
            // Rename log (F-033)
            commands::sync::list_rename_history,
            commands::sync::list_renames_for_repo,
            // Templates (0.6.0)
            commands::templates::list_template_languages,
            commands::templates::list_template_files,
            commands::templates::get_template_file,
            commands::templates::save_template_file,
            commands::templates::reset_template_file,
            // Deploy (0.7.0 / v0.18.0 multi-env)
            commands::deploy::set_deploy_target,
            commands::deploy::get_repo_deploy_config,
            commands::deploy::set_repo_deploy_config,
            commands::deploy::render_deploy_files_for_env,
            commands::deploy::list_deploy_environments,
            commands::deploy::list_deploy_report,
            commands::deploy::get_deploy_environment,
            commands::deploy::create_deploy_environment,
            commands::deploy::clone_deploy_environment,
            commands::deploy::update_deploy_environment,
            commands::deploy::delete_deploy_environment,
            commands::deploy::reorder_deploy_environments,
            commands::deploy::list_deploy_secrets,
            commands::deploy::upsert_deploy_secret,
            commands::deploy::delete_deploy_secret,
            commands::deploy::ensure_deploy_secrets_populated,
            commands::deploy::register_repo_secret_in_deploys,
            commands::deploy::set_deploy_secret_value,
            commands::deploy::delete_deploy_secret_value,
            commands::deploy::get_deploy_secret_values,
            commands::deploy::read_repo_file,
            commands::misc::read_repo_files,
            commands::misc::read_repo_todo,
            commands::misc::read_repo_done,
            commands::misc::parse_done_entries_in_period_cmd,
            commands::misc::write_deploy_files,
            commands::sync::sync_tasks_for_repo_cmd,
            commands::sync::read_tasks_from_db,
            commands::sync::read_done_from_db,
            commands::timeline::record_secret_event,
            commands::timeline::record_deploy_secret_event,
            // Secret bundles (v1.3.0)
            commands::deploy::list_secret_bundles,
            commands::deploy::create_secret_bundle,
            commands::deploy::rename_secret_bundle,
            commands::deploy::delete_secret_bundle,
            commands::deploy::upsert_bundle_item,
            commands::deploy::delete_bundle_item,
            commands::deploy::get_bundle_decrypted,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
