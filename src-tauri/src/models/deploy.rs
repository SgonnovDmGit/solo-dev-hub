use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// v0.18.0: one deploy environment per row. 1:N with repositories.
/// `name` is a user-chosen slug (prod/test/staging/custom). `extras` JSON
/// holds non-core placeholders (APP_PORT, NETWORK_NAME, COMPOSE_PROJECT, …).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployEnvironment {
    pub id: i64,
    pub repository_id: i64,
    pub name: String,
    pub workflow_name: String,
    pub image_tag: String,
    pub compose_service: String,
    pub domain: String,
    pub deploy_branch: String,
    pub sort_order: i64,
    #[serde(default)]
    pub extras: HashMap<String, String>,
    pub updated_at: String,
}

/// v0.18.0: per-deploy per-secret flags. Values are NOT stored here —
/// they live in GitHub Secrets API (repo-scoped or env-scoped).
/// `role` is `Option<String>` because it's meaningful only when `included=true`;
/// in DB it's NULL when included=false (CHECK constraint still allows this).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploySecret {
    pub id: i64,
    pub deploy_env_id: i64,
    pub secret_name: String,
    pub role: Option<String>,
    pub included: bool,
    pub override_enabled: bool,
    pub sort_order: i64,
}

/// Args for creating a deploy environment via Tauri command.
/// `extras` optional; defaults to empty map.
#[derive(Debug, Deserialize, Clone)]
pub struct CreateDeployEnvironmentArgs {
    pub repository_id: i64,
    pub name: String,
    pub workflow_name: String,
    pub image_tag: String,
    pub compose_service: String,
    pub domain: String,
    pub deploy_branch: String,
    #[serde(default)]
    pub extras: HashMap<String, String>,
}

/// Args for updating a deploy environment. `name` is read-only post-create,
/// so NOT present in this struct. Only placeholders + extras are mutable.
#[derive(Debug, Deserialize, Clone)]
pub struct UpdateDeployEnvironmentArgs {
    pub id: i64,
    pub workflow_name: String,
    pub image_tag: String,
    pub compose_service: String,
    pub domain: String,
    pub deploy_branch: String,
    #[serde(default)]
    pub extras: HashMap<String, String>,
}

/// v0.18.0+ meta.json hint shape — role + scope per required_secret.
/// Passed from lib.rs (which parses meta.json) into db.rs ensure_deploy_secrets_populated.
///
/// v0.31.0 (T-000103 Task 2 — schema v5): secret scope vocabulary was renamed
/// `"repo" → "deploy_repo"` to disambiguate from placeholder scope `"repo"`
/// (which marks a placeholder as repo-wide rather than per-env). The two fields
/// now use distinct value spaces:
///   - `MetaPlaceholder.scope` ∈ {`"repo"`, `"environment"`}  (default `"environment"`)
///   - `MetaSecretHint.scope`  ∈ {`"deploy_repo"`, `"environment"`}
/// Parsers must `Err` on any other value for either field — see
/// `parse_meta_secret_hint` / `parse_meta_placeholders` in `template_meta`.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaSecretHint {
    pub name: String,
    pub role: String,  // "build" | "deploy" | "runtime"
    pub scope: String, // "deploy_repo" | "environment"
}

/// v0.31.0 (T-000103 Task 2 — schema v5): meta.json `placeholders.<KEY>` shape.
/// Only fields the Rust side currently consumes are modeled — `label`,
/// `description`, `default`, `type`, `auto_detect` live in the JSON but the
/// frontend reads them straight from the raw JSON value.
///
/// `scope` marks a placeholder as either repo-wide (`"repo"`) — i.e. rendered
/// into a single repo-wide file like `Dockerfile` and stored on
/// `repositories.deploy_repo_config` — or per-env (`"environment"`, default)
/// — stored in `deploy_environments.extras`.
///
/// Unknown scope values must be rejected at template-load time
/// (strict mode, no silent fallback). See `parse_meta_placeholder`.
///
/// Consumed by Task 3's schema-aware render merger in `template_render`
/// (`build_placeholder_vars`) and by the frontend via Task 4/5's UI work.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaPlaceholder {
    /// "repo" | "environment". Absent → treat as "environment".
    #[serde(default)]
    pub scope: Option<String>,
}

// Read-side DTO for deploy_events — currently consumed only by tests that
// verify the event log; gated so it doesn't read as dead code in release builds.
#[cfg(test)]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployEvent {
    pub id: i64,
    pub deploy_env_id: Option<i64>,
    pub repository_id: i64,
    pub action: String,
    pub ts: String,
    pub details: Option<String>,
}

/// v1.2.0 (deploy report): one row of the portfolio-wide deploy inventory.
/// Joins a deploy environment with its repo (display = last segment of
/// github_name) + project (NULL for orphan repos) + count of included secrets.
/// snake_case JSON, no serde rename — matches `DeployEnvironment` and the TS
/// `src/lib/types/deploy.ts` convention (Tauri tool, not a server).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployReportRow {
    pub deploy_env_id: i64,
    pub repository_id: i64,
    pub repo_name: String,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub env_name: String,
    pub domain: String,
    pub deploy_branch: String,
    pub image_tag: String,
    pub secrets_count: i64,
    pub updated_at: String,
    // v1.6.0 (T-000134): DB/SSH connection inventory, assembled from local data
    // (persisted encrypted values, plaintext placeholders, github-only secret
    // names). Sensitive values are withheld. Both default to `[]`, never null.
    #[serde(default)]
    pub db_fields: Vec<DeployInventoryField>,
    #[serde(default)]
    pub ssh_fields: Vec<DeployInventoryField>,
}

/// v1.6.0 (T-000134): one DB- or SSH-related inventory field surfaced in the
/// deploy report. `value` is `None` when the field is sensitive (withheld) or
/// exists only as a GitHub secret name with no local value. `origin` records
/// where the name was sourced from. snake_case JSON, no serde rename.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployInventoryField {
    pub name: String,
    /// `None` = redacted (sensitive) OR github-only (no local value).
    pub value: Option<String>,
    /// `"persisted"` | `"placeholder"` | `"github_only"`.
    pub origin: String,
    /// `true` = value withheld because the name is sensitive.
    pub sensitive: bool,
}

/// v1.6.0 (F-000043): one decrypted deploy secret name+value. Persisted
/// encrypted-at-rest in `deploy_secret_values` (mirrors `SecretBundleItemValue`).
/// snake_case JSON, no serde rename — matches the other deploy structs
/// (Tauri tool contract, not a server).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploySecretValue {
    pub secret_name: String,
    pub value: String,
}

/// v1.8.0 (T-000135): one normalized secret-push audit event. Unified read over
/// two already-logged sources: repo-level pushes (`sync_events`, sync_type='secret',
/// verb inside details.action) and env-level pushes (`deploy_events`, verb inside
/// the action column via `env_secret_set`/`env_secret_delete`). `action` is
/// normalized to "set" | "delete" and `source` to "repo" | "env". snake_case JSON,
/// no serde rename — matches the other deploy structs (Tauri tool contract).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecretPushEvent {
    pub source: String,
    pub repository_id: i64,
    pub repo_name: String,
    pub deploy_env_id: Option<i64>,
    pub env_name: Option<String>,
    pub secret_name: String,
    pub action: String,
    pub ts: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deploy_environment_serde_roundtrip() {
        let env = DeployEnvironment {
            id: 42,
            repository_id: 1,
            name: "prod".to_string(),
            workflow_name: "Deploy".to_string(),
            image_tag: "latest".to_string(),
            compose_service: "backend".to_string(),
            domain: "x.com".to_string(),
            deploy_branch: "master".to_string(),
            sort_order: 0,
            extras: Default::default(),
            updated_at: "2026-04-25T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&env).unwrap();
        let back: DeployEnvironment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 42);
        assert_eq!(back.name, "prod");
    }

    #[test]
    fn test_deploy_secret_serde_roundtrip() {
        let s = DeploySecret {
            id: 1,
            deploy_env_id: 42,
            secret_name: "SSH_HOST".to_string(),
            role: Some("deploy".to_string()),
            included: true,
            override_enabled: true,
            sort_order: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: DeploySecret = serde_json::from_str(&json).unwrap();
        assert_eq!(back.secret_name, "SSH_HOST");
        assert!(back.included);
    }

    #[test]
    fn test_deploy_secret_role_none_when_not_included() {
        // Role is Option<String> — NULL in DB when included=false.
        let s = DeploySecret {
            id: 1,
            deploy_env_id: 1,
            secret_name: "X".to_string(),
            role: None,
            included: false,
            override_enabled: false,
            sort_order: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"role\":null"));
    }
}
