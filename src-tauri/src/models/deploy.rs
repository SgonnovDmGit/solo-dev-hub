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

/// v0.18.0: meta.json v4 hint shape — role + scope per required_secret.
/// Passed from lib.rs (which parses meta.json) into db.rs ensure_deploy_secrets_populated.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaSecretHint {
    pub name: String,
    pub role: String,     // "build" | "deploy" | "runtime"
    pub scope: String,    // "repo" | "environment"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployEvent {
    pub id: i64,
    pub deploy_env_id: Option<i64>,
    pub repository_id: i64,
    pub action: String,
    pub ts: String,
    pub details: Option<String>,
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
