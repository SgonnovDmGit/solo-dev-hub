use crate::db::AppDb;
use crate::models::*;
use crate::{template_meta, template_render};
use std::collections::HashMap;
use tauri::State;

// ── Deploy (0.7.0) ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn set_deploy_target(
    db: State<AppDb>,
    id: i64,
    target: Option<String>,
) -> Result<Repository, String> {
    db.set_deploy_target(id, target.as_deref())
        .map_err(|e| e.to_string())
}

// T-000103 Task 1: repo-wide deploy config (placeholder values shared across
// envs — e.g. GO_VERSION baked into the single Dockerfile).
#[tauri::command]
pub fn get_repo_deploy_config(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<HashMap<String, String>, String> {
    db.get_repo_deploy_config(repo_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_repo_deploy_config(
    db: State<AppDb>,
    repo_id: i64,
    config: HashMap<String, String>,
) -> Result<(), String> {
    db.set_repo_deploy_config(repo_id, &config)
        .map_err(|e| e.to_string())
}

// ── Secret bundles (v1.3.0) ───────────────────────────────────────────────────
#[tauri::command]
pub fn list_secret_bundles(db: State<AppDb>) -> Result<Vec<SecretBundle>, String> {
    db.list_secret_bundles().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_secret_bundle(
    db: State<AppDb>,
    name: String,
    description: String,
) -> Result<i64, String> {
    db.create_secret_bundle(&name, &description)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_secret_bundle(
    db: State<AppDb>,
    id: i64,
    name: String,
    description: String,
) -> Result<(), String> {
    db.rename_secret_bundle(id, &name, &description)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_secret_bundle(db: State<AppDb>, id: i64) -> Result<(), String> {
    db.delete_secret_bundle(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn upsert_bundle_item(
    db: State<AppDb>,
    bundle_id: i64,
    secret_name: String,
    value: String,
) -> Result<(), String> {
    db.upsert_bundle_item(bundle_id, &secret_name, &value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_bundle_item(db: State<AppDb>, item_id: i64) -> Result<(), String> {
    db.delete_bundle_item(item_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_bundle_decrypted(
    db: State<AppDb>,
    bundle_id: i64,
) -> Result<Vec<SecretBundleItemValue>, String> {
    db.get_bundle_decrypted(bundle_id)
        .map_err(|e| e.to_string())
}

// ── Deploy render (v0.18.0, multi-env) ────────────────────────────────────────

/// v0.18.0: render workflow/Dockerfile files for a single deploy_env.
/// Returns Vec<RenderedFile> with paths substituted per `meta.json.file_targets`
/// ({name} → deploy_env.name). Shared files (Dockerfile) appear ONCE per call —
/// multi-env consumers may get the same Dockerfile twice; caller dedupes.
///
/// Placeholder composition:
///   - core 5 from deploy_env (WORKFLOW_NAME, IMAGE_TAG, COMPOSE_SERVICE, DOMAIN, DEPLOY_BRANCH)
///   - extras (APP_PORT, NETWORK_NAME, COMPOSE_PROJECT, ENV_FILE_PATH, …)
///   - ENV_NAME = deploy_env.name
///   - BUILD_ARGS, RUNTIME_ENV_ARGS — per-env, from deploy_secrets with included=1 + role
///   - DOCKERFILE_ARGS, DART_DEFINES — UNION of build-role secrets across ALL deploy_envs of this repo
pub fn render_files_for_deploy_env(
    db: &AppDb,
    deploy_env_id: i64,
) -> Result<Vec<RenderedFile>, String> {
    let env = db
        .get_deploy_environment(deploy_env_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("deploy_environment {} not found", deploy_env_id))?;
    let repo = db
        .get_repository(env.repository_id)
        .map_err(|e| e.to_string())?;
    let target = repo
        .deploy_target
        .clone()
        .ok_or_else(|| "No deploy target set for this repository".to_string())?;

    // Load meta.json for file_targets + placeholder defaults
    let meta_file = db
        .get_template_file(&target, "meta.json")
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("meta.json missing for language '{}'", target))?;
    let meta: serde_json::Value = serde_json::from_str(&meta_file.content)
        .map_err(|e| format!("Invalid meta.json: {}", e))?;
    let file_targets = meta
        .get("file_targets")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "meta.json missing 'file_targets'".to_string())?;

    // T-000103 Task 3: parse meta.placeholders strict (also gives us each
    // placeholder's `scope` for the schema-aware merger below).
    let meta_placeholders = template_meta::parse_meta_placeholders(&target, &meta)?;

    // T-000103 Task 3: fetch repo-wide deploy config (placeholder values for
    // repo-scope keys like GO_VERSION that render a single repo-wide
    // Dockerfile). Empty map on first render before user fills anything in.
    let repo_config = db
        .get_repo_deploy_config(env.repository_id)
        .map_err(|e| e.to_string())?;

    // Gather build/runtime secrets for THIS env
    let secrets = db
        .list_deploy_secrets(deploy_env_id)
        .map_err(|e| e.to_string())?;
    let build_for_this_env: Vec<String> = secrets
        .iter()
        .filter(|s| s.included && s.role.as_deref() == Some("build"))
        .map(|s| s.secret_name.clone())
        .collect();
    let runtime_for_this_env: Vec<String> = secrets
        .iter()
        .filter(|s| s.included && s.role.as_deref() == Some("runtime"))
        .map(|s| s.secret_name.clone())
        .collect();

    // UNION build-role secrets across ALL envs of this repo (for shared Dockerfile)
    let all_envs = db
        .list_deploy_environments(env.repository_id)
        .map_err(|e| e.to_string())?;
    let mut union_build: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in &all_envs {
        let esec = db
            .list_deploy_secrets(e.id)
            .map_err(|err| err.to_string())?;
        for s in esec {
            if s.included && s.role.as_deref() == Some("build") {
                union_build.insert(s.secret_name);
            }
        }
    }
    let union_build_vec: Vec<String> = union_build.into_iter().collect();

    // Build placeholder map. Order matters:
    //  1. Seed with each placeholder's `default` from meta.json.
    //  2. Overlay scope-driven values via build_placeholder_vars — sources
    //     `scope: "repo"` keys from `repo_config` and `scope: "environment"`
    //     (the default) from `env.extras`. Orphan keys in either source are
    //     filtered out by the merger.
    //  3. Override the typed columns from `deploy_environments` (core 5 +
    //     ENV_NAME) — these are not in `env.extras` but are listed in
    //     `meta.placeholders` with `scope: "environment"`; the merger emits
    //     nothing for them in step 2, so the explicit override below fills
    //     them from the typed columns.
    //  4. Insert v0.18.0-specific synthetic vars (BUILD_ARGS etc.) — these
    //     are NOT in meta.placeholders, they're rendered separately by the
    //     helper fns and substituted into placeholders that appear in the
    //     templates verbatim.
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(phs) = meta.get("placeholders").and_then(|v| v.as_object()) {
        for (k, spec) in phs {
            if let Some(default) = spec.get("default").and_then(|v| v.as_str()) {
                vars.insert(k.clone(), default.to_string());
            }
        }
    }
    // Schema-aware merge: pick each declared placeholder's value from the
    // correct source based on its scope. Orphan keys in either source are
    // ignored.
    for (k, v) in
        template_render::build_placeholder_vars(&meta_placeholders, &repo_config, &env.extras)
    {
        vars.insert(k, v);
    }
    // Core 5 from deploy_env typed columns (overrides defaults — these
    // values live on the typed columns, not in `extras`).
    vars.insert("WORKFLOW_NAME".to_string(), env.workflow_name.clone());
    vars.insert("IMAGE_TAG".to_string(), env.image_tag.clone());
    vars.insert("COMPOSE_SERVICE".to_string(), env.compose_service.clone());
    vars.insert("DOMAIN".to_string(), env.domain.clone());
    vars.insert("DEPLOY_BRANCH".to_string(), env.deploy_branch.clone());
    // v0.18.0-specific synthetic placeholders (not declared in meta.placeholders)
    vars.insert("ENV_NAME".to_string(), env.name.clone());
    vars.insert(
        "BUILD_ARGS".to_string(),
        template_render::render_build_args(&build_for_this_env),
    );
    vars.insert(
        "RUNTIME_ENV_ARGS".to_string(),
        template_render::render_runtime_env_args(&runtime_for_this_env),
    );
    vars.insert(
        "DOCKERFILE_ARGS".to_string(),
        template_render::render_dockerfile_args(&union_build_vec),
    );
    vars.insert(
        "DOCKERFILE_ENVS".to_string(),
        template_render::render_dockerfile_envs(&union_build_vec),
    );
    vars.insert(
        "DART_DEFINES".to_string(),
        template_render::render_dart_defines(&union_build_vec),
    );

    // Render each file from the template dir whose file_name is listed in file_targets
    let all_files = db.list_template_files(&target).map_err(|e| e.to_string())?;
    let mut rendered: Vec<RenderedFile> = Vec::new();
    for f in &all_files {
        let Some(target_path_tmpl) = file_targets.get(&f.file_name).and_then(|v| v.as_str()) else {
            continue;
        };
        let target_path = target_path_tmpl.replace("{name}", &env.name);
        let content = template_render::render_template(&f.content, &vars)?;
        rendered.push(RenderedFile {
            path: target_path,
            content,
        });
    }
    Ok(rendered)
}

#[tauri::command]
pub fn render_deploy_files_for_env(
    db: State<AppDb>,
    deploy_env_id: i64,
) -> Result<Vec<RenderedFile>, String> {
    render_files_for_deploy_env(&db, deploy_env_id)
}

#[tauri::command]
pub fn list_deploy_environments(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<DeployEnvironment>, String> {
    db.list_deploy_environments(repo_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_deploy_report(db: State<AppDb>) -> Result<Vec<DeployReportRow>, String> {
    db.list_deploy_report().map_err(|e| e.to_string())
}

/// v1.8.0 (T-000140): CSV-export the deploy report. The frontend flattens each
/// displayed row into `DeployReportCsvRow` and passes them here; the backend
/// only CSV-formats (RFC4180) + writes to a user-chosen path (from an OS save
/// dialog, so no path guard needed).
#[tauri::command]
pub fn export_deploy_report_csv(
    file_path: String,
    rows: Vec<crate::models::DeployReportCsvRow>,
) -> Result<(), String> {
    let csv = crate::export::deploy_report_to_csv(&rows);
    std::fs::write(&file_path, csv).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_deploy_environment(
    db: State<AppDb>,
    id: i64,
) -> Result<Option<DeployEnvironment>, String> {
    db.get_deploy_environment(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_deploy_environment(
    db: State<AppDb>,
    args: CreateDeployEnvironmentArgs,
) -> Result<DeployEnvironment, String> {
    validate_env_name(&args.name)?;
    db.insert_deploy_environment(&args)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clone_deploy_environment(
    db: State<AppDb>,
    source_id: i64,
    new_name: String,
) -> Result<DeployEnvironment, String> {
    validate_env_name(&new_name)?;
    db.clone_deploy_environment(source_id, &new_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_deploy_environment(
    db: State<AppDb>,
    args: UpdateDeployEnvironmentArgs,
) -> Result<DeployEnvironment, String> {
    db.update_deploy_environment(&args)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_deploy_environment(db: State<AppDb>, id: i64) -> Result<(), String> {
    db.delete_deploy_environment(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reorder_deploy_environments(
    db: State<AppDb>,
    repo_id: i64,
    ordered_ids: Vec<i64>,
) -> Result<(), String> {
    db.reorder_deploy_environments(repo_id, &ordered_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_deploy_secrets(
    db: State<AppDb>,
    deploy_env_id: i64,
) -> Result<Vec<DeploySecret>, String> {
    db.list_deploy_secrets(deploy_env_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn upsert_deploy_secret(
    db: State<AppDb>,
    deploy_env_id: i64,
    secret_name: String,
    role: Option<String>,
    included: bool,
    override_enabled: bool,
) -> Result<(), String> {
    db.upsert_deploy_secret(
        deploy_env_id,
        &secret_name,
        role.as_deref(),
        included,
        override_enabled,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_deploy_secret(
    db: State<AppDb>,
    deploy_env_id: i64,
    secret_name: String,
) -> Result<(), String> {
    db.delete_deploy_secret(deploy_env_id, &secret_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ensure_deploy_secrets_populated(
    db: State<AppDb>,
    deploy_env_id: i64,
    repo_secret_names: Vec<String>,
) -> Result<(), String> {
    // Parse meta.json to get hints
    let env = db
        .get_deploy_environment(deploy_env_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("deploy_env {} not found", deploy_env_id))?;
    let repo = db
        .get_repository(env.repository_id)
        .map_err(|e| e.to_string())?;
    let target = repo.deploy_target.clone().unwrap_or_default();
    let hints = if target.is_empty() {
        Vec::new()
    } else {
        parse_meta_secret_hints(&db, &target)?
    };
    db.ensure_deploy_secrets_populated(deploy_env_id, &repo_secret_names, &hints)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn register_repo_secret_in_deploys(
    db: State<AppDb>,
    repo_id: i64,
    secret_name: String,
) -> Result<(), String> {
    db.register_repo_secret_in_deploys(repo_id, &secret_name)
        .map_err(|e| e.to_string())
}

// ── Persisted deploy secret values (v1.6.0, F-000043) ─────────────────────────
#[tauri::command]
pub fn set_deploy_secret_value(
    db: State<AppDb>,
    deploy_env_id: i64,
    secret_name: String,
    value: String,
) -> Result<(), String> {
    db.set_deploy_secret_value(deploy_env_id, &secret_name, &value)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_deploy_secret_value(
    db: State<AppDb>,
    deploy_env_id: i64,
    secret_name: String,
) -> Result<(), String> {
    db.delete_deploy_secret_value(deploy_env_id, &secret_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_deploy_secret_values(
    db: State<AppDb>,
    deploy_env_id: i64,
) -> Result<Vec<DeploySecretValue>, String> {
    db.get_deploy_secret_values(deploy_env_id)
        .map_err(|e| e.to_string())
}

fn validate_env_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Environment name is required".to_string());
    }
    if name.len() > 255 {
        return Err("Environment name too long (max 255)".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Environment name must contain only letters, digits, hyphens and underscores"
                .to_string(),
        );
    }
    Ok(())
}

/// Parse `required_secrets` for a given template (`target` = `language_key`)
/// from the `templates` SQLite table. v0.31.0+ uses the strict parser in
/// `template_meta` — custom templates with the obsolete `"scope": "repo"`
/// secret value fail to load with a UI-friendly error.
fn parse_meta_secret_hints(db: &AppDb, target: &str) -> Result<Vec<MetaSecretHint>, String> {
    let meta_file = db
        .get_template_file(target, "meta.json")
        .map_err(|e| e.to_string())?;
    let Some(mf) = meta_file else {
        return Ok(Vec::new());
    };
    let meta: serde_json::Value =
        serde_json::from_str(&mf.content).map_err(|e| format!("Invalid meta.json: {}", e))?;
    template_meta::parse_meta_secret_hints(target, &meta)
}

/// Read a single file from a repo by its database id + relative path.
/// Returns `Ok(None)` if the repo has no `local_path`, the file doesn't exist,
/// or the contents aren't valid UTF-8. Returns `Err` only for DB errors.
/// Used by DeployScreen's `auto_detect` to pre-fill placeholders (e.g. GO_VERSION from go.mod).
#[tauri::command]
pub fn read_repo_file(
    db: State<AppDb>,
    repo_id: i64,
    rel_path: String,
) -> Result<Option<String>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(None);
    };
    let full = std::path::Path::new(&local_path).join(&rel_path);
    if !full.exists() {
        return Ok(None);
    }
    Ok(std::fs::read_to_string(&full).ok())
}

#[cfg(test)]
mod render_deploy_tests {
    use super::*;
    use crate::db::AppDb;
    use crate::models::CreateDeployEnvironmentArgs;
    use crate::template_seeder::seed_bundled_templates;

    fn setup() -> (AppDb, i64, i64) {
        let tmp = tempfile::TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        std::mem::forget(tmp);
        seed_bundled_templates(&db).unwrap();
        let project = db.create_project("p", None, "tool").unwrap();
        let repo = db
            .insert_local_repository("/tmp/r", "r", Some(project.id), None)
            .unwrap();
        db.set_deploy_target(repo.id, Some("go")).unwrap();
        // T-000103 Task 3: repo-scope placeholders (GO_VERSION, BINARY_NAME,
        // ENTRY_POINT, APP_PORT) now live in `repositories.deploy_repo_config`,
        // not per-env `extras`. They bake into the single repo-wide Dockerfile.
        let repo_config: std::collections::HashMap<String, String> = [
            ("GO_VERSION", "1.23"),
            ("BINARY_NAME", "app"),
            ("ENTRY_POINT", "./cmd/api/"),
            ("APP_PORT", "8080"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        db.set_repo_deploy_config(repo.id, &repo_config).unwrap();
        let env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo.id,
                name: "prod".to_string(),
                workflow_name: "Deploy".to_string(),
                image_tag: "latest".to_string(),
                compose_service: "backend".to_string(),
                domain: "x.com".to_string(),
                deploy_branch: "master".to_string(),
                extras: {
                    // Env-scope placeholders only — repo-scope ones moved to repo_config above.
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m.insert("NETWORK_NAME".to_string(), "app_prod_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_prod".to_string());
                    m
                },
            })
            .unwrap();
        (db, repo.id, env.id)
    }

    #[test]
    fn test_render_for_env_produces_deploy_yml_with_env_name() {
        let (db, _repo, env_id) = setup();
        // Seed 1 runtime secret for this env
        db.upsert_deploy_secret(env_id, "DATABASE_URL", Some("runtime"), true, true)
            .unwrap();
        let files = render_files_for_deploy_env(&db, env_id).unwrap();

        let deploy_yml = files
            .iter()
            .find(|f| f.path == ".github/workflows/deploy-prod.yml")
            .expect("deploy-prod.yml must be produced");
        assert!(deploy_yml.content.contains("environment: prod"));
        assert!(deploy_yml
            .content
            .contains("--env DATABASE_URL=\"${{ secrets.DATABASE_URL }}\""));
        assert!(deploy_yml.content.contains("--network app_prod_net"));
        assert!(deploy_yml
            .content
            .contains("com.docker.compose.project=app_prod"));
    }

    #[test]
    fn test_render_multiple_envs_produces_separate_workflow_files() {
        let (db, repo_id, prod_id) = setup();
        // T-000103 Task 3: repo-scope keys live in deploy_repo_config (seeded
        // by setup() for both envs of this repo). Only env-scope keys go in
        // each env's `extras`.
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m
                },
            })
            .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        assert!(prod_files
            .iter()
            .any(|f| f.path == ".github/workflows/deploy-prod.yml"));
        assert!(test_files
            .iter()
            .any(|f| f.path == ".github/workflows/deploy-test.yml"));
    }

    #[test]
    fn test_multi_env_go_isolation_per_env_values_baked_in() {
        // v0.29.0 multi-deploy smoke: same Go repo, two envs, each rendered
        // workflow file must contain its own env-specific values and NOT leak
        // values from the other env.
        // T-000103 Task 3: repo-scope keys live in deploy_repo_config (seeded
        // by setup()). Only env-scope keys go in `extras`.
        let (db, repo_id, prod_id) = setup();
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m
                },
            })
            .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        let prod_yml = &prod_files
            .iter()
            .find(|f| f.path.ends_with("deploy-prod.yml"))
            .unwrap()
            .content;
        let test_yml = &test_files
            .iter()
            .find(|f| f.path.ends_with("deploy-test.yml"))
            .unwrap()
            .content;

        // Prod-specific values present in prod, absent from test.
        assert!(prod_yml.contains("environment: prod"));
        assert!(prod_yml.contains("--network app_prod_net"));
        assert!(prod_yml.contains("com.docker.compose.project=app_prod"));
        assert!(prod_yml.contains("branches: [ master ]"));
        assert!(prod_yml.contains("DOMAIN=x.com"));
        assert!(
            !prod_yml.contains("app_test_net"),
            "prod must not leak test network"
        );
        assert!(
            !prod_yml.contains("test.x.com"),
            "prod must not leak test domain"
        );

        // Test-specific values present in test, absent from prod.
        assert!(test_yml.contains("environment: test"));
        assert!(test_yml.contains("--network app_test_net"));
        assert!(test_yml.contains("com.docker.compose.project=app_test"));
        assert!(test_yml.contains("branches: [ dev ]"));
        assert!(test_yml.contains("DOMAIN=test.x.com"));
        assert!(
            !test_yml.contains("app_prod_net"),
            "test must not leak prod network"
        );
        assert!(
            !test_yml.contains("DOMAIN=x.com\n"),
            "test must not leak prod domain"
        );
    }

    #[test]
    fn test_multi_env_go_runtime_secrets_per_env_isolation() {
        // Each env's runtime secrets must appear only in that env's deploy.yml.
        // T-000103 Task 3: APP_PORT lives in deploy_repo_config (seeded by setup()).
        let (db, repo_id, prod_id) = setup();
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    m
                },
            })
            .unwrap();

        // Per-env runtime secrets: prod gets DATABASE_URL_PROD, test gets DATABASE_URL_TEST.
        db.upsert_deploy_secret(prod_id, "DATABASE_URL_PROD", Some("runtime"), true, true)
            .unwrap();
        db.upsert_deploy_secret(
            test_env.id,
            "DATABASE_URL_TEST",
            Some("runtime"),
            true,
            true,
        )
        .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        let prod_yml = &prod_files
            .iter()
            .find(|f| f.path.ends_with("deploy-prod.yml"))
            .unwrap()
            .content;
        let test_yml = &test_files
            .iter()
            .find(|f| f.path.ends_with("deploy-test.yml"))
            .unwrap()
            .content;

        assert!(
            prod_yml.contains("DATABASE_URL_PROD"),
            "prod must reference its own runtime secret"
        );
        assert!(
            !prod_yml.contains("DATABASE_URL_TEST"),
            "prod must not leak test runtime secret"
        );
        assert!(
            test_yml.contains("DATABASE_URL_TEST"),
            "test must reference its own runtime secret"
        );
        assert!(
            !test_yml.contains("DATABASE_URL_PROD"),
            "test must not leak prod runtime secret"
        );
    }

    #[test]
    fn test_multi_env_go_shared_dockerfile_identical_across_envs() {
        // Go's Dockerfile uses only repo-wide placeholders (GO_VERSION,
        // BINARY_NAME, ENTRY_POINT, APP_PORT) and NOT env-specific
        // DOCKERFILE_ARGS (that's a Flutter-specific concept — Go binaries are
        // statically linked, secrets are runtime-injected via docker --env).
        // So when those repo-wide values match, the rendered Dockerfile must
        // be byte-identical regardless of which env triggers the render.
        // T-000103 Task 3: repo-wide values now live in `deploy_repo_config`
        // (seeded once by setup() — shared by ALL envs of the repo by design,
        // so identical-rendered-Dockerfile becomes a structural guarantee, not
        // a coincidence-of-matching-extras).
        let (db, repo_id, prod_id) = setup();
        let test_env = db
            .insert_deploy_environment(&CreateDeployEnvironmentArgs {
                repository_id: repo_id,
                name: "test".to_string(),
                workflow_name: "Deploy test".to_string(),
                image_tag: "test".to_string(),
                compose_service: "backend".to_string(),
                domain: "test.x.com".to_string(),
                deploy_branch: "dev".to_string(),
                extras: {
                    let mut m = std::collections::HashMap::new();
                    m.insert("ENV_FILE_PATH".to_string(), "".to_string());
                    // Env-specific values differ from prod — must NOT affect Dockerfile.
                    m.insert("NETWORK_NAME".to_string(), "app_test_net".to_string());
                    m.insert("COMPOSE_PROJECT".to_string(), "app_test".to_string());
                    m
                },
            })
            .unwrap();

        let prod_files = render_files_for_deploy_env(&db, prod_id).unwrap();
        let test_files = render_files_for_deploy_env(&db, test_env.id).unwrap();

        let prod_dockerfile = &prod_files
            .iter()
            .find(|f| f.path == "Dockerfile")
            .unwrap()
            .content;
        let test_dockerfile = &test_files
            .iter()
            .find(|f| f.path == "Dockerfile")
            .unwrap()
            .content;
        assert_eq!(
            prod_dockerfile, test_dockerfile,
            "Go Dockerfile is repo-wide; identical extras must render identical Dockerfile"
        );
    }

    #[test]
    fn test_validate_env_name_accepts_valid() {
        assert!(super::validate_env_name("prod").is_ok());
        assert!(super::validate_env_name("test-1").is_ok());
        assert!(super::validate_env_name("staging_v2").is_ok());
    }

    #[test]
    fn test_validate_env_name_rejects_invalid() {
        assert!(super::validate_env_name("").is_err());
        assert!(super::validate_env_name("has space").is_err());
        assert!(super::validate_env_name("dot.name").is_err());
        assert!(super::validate_env_name("slash/name").is_err());
        assert!(super::validate_env_name(&"x".repeat(256)).is_err());
    }
}
