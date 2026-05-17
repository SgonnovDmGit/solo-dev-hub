//! Strict-mode parser for `meta.json` of deploy templates.
//!
//! T-000103 Task 2 (v0.31.0 / schema v5): scope vocabulary split.
//!
//! Two scope vocabularies coexist on the same JSON file:
//!   - `placeholders.<KEY>.scope`     ∈ {`"repo"`, `"environment"`}        default `"environment"`
//!   - `required_secrets[*].scope`    ∈ {`"deploy_repo"`, `"environment"`}  (no default — explicit)
//!
//! The `"deploy_repo"` rename (from the pre-v0.31.0 `"repo"`) disambiguates
//! the secret-side concept ("this is a GH Actions Repository Secret, not an
//! Environment Secret") from the placeholder-side concept ("this placeholder
//! renders into a single repo-wide file like Dockerfile, not a per-env file
//! like deploy-prod.yml"). Pre-v1.0.0 + no shipped users → no back-compat
//! shim. Custom templates carrying the obsolete value fail to load with a
//! human-readable error pointing at the exact field.

use crate::models::{MetaPlaceholder, MetaSecretHint};
use serde_json::Value;

/// Validate a placeholder scope value. Accepted: `"repo"`, `"environment"`,
/// or `None` (treated as `"environment"` at consumer side).
///
/// Returns the canonicalised scope string on success — caller may inspect
/// or pass it along. Unknown values produce an error that names the offending
/// template + placeholder.
pub fn validate_placeholder_scope(
    template_name: &str,
    placeholder_name: &str,
    raw: Option<&str>,
) -> Result<&'static str, String> {
    match raw {
        None => Ok("environment"),
        Some("repo") => Ok("repo"),
        Some("environment") => Ok("environment"),
        Some(other) => Err(format!(
            "Template '{}' has invalid placeholder scope '{}' for '{}'. \
             Accepted values: 'repo' | 'environment'.",
            template_name, other, placeholder_name
        )),
    }
}

/// Validate a required_secret scope value. Accepted: `"deploy_repo"`,
/// `"environment"`. The pre-v0.31.0 value `"repo"` is REJECTED loudly —
/// it was renamed to `"deploy_repo"` (decision β / D2 in T-000103 spec).
pub fn validate_secret_scope(
    template_name: &str,
    secret_name: &str,
    raw: Option<&str>,
) -> Result<&'static str, String> {
    match raw {
        Some("deploy_repo") => Ok("deploy_repo"),
        Some("environment") => Ok("environment"),
        Some("repo") => Err(format!(
            "Template '{}' uses obsolete secret scope value 'repo' for secret '{}'. \
             This was renamed to 'deploy_repo' in v0.31.0. Please update your meta.json.",
            template_name, secret_name
        )),
        Some(other) => Err(format!(
            "Template '{}' has invalid secret scope '{}' for '{}'. \
             Accepted values: 'deploy_repo' | 'environment'.",
            template_name, other, secret_name
        )),
        None => Err(format!(
            "Template '{}' is missing the 'scope' field on secret '{}'. \
             Required values: 'deploy_repo' | 'environment'.",
            template_name, secret_name
        )),
    }
}

/// Parse `required_secrets` from a parsed meta.json value, strict.
///
/// Returns the list of `MetaSecretHint`s on success. Returns an error if any
/// row carries an unknown scope value (including the obsolete `"repo"`) or is
/// missing the `name` / `scope` fields.
pub fn parse_meta_secret_hints(
    template_name: &str,
    meta: &Value,
) -> Result<Vec<MetaSecretHint>, String> {
    let Some(arr) = meta.get("required_secrets").and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let name = item.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            format!(
                "Template '{}' has a required_secrets entry missing 'name'.",
                template_name
            )
        })?;
        let role = item
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("deploy")
            .to_string();
        let raw_scope = item.get("scope").and_then(|v| v.as_str());
        let scope = validate_secret_scope(template_name, name, raw_scope)?;
        out.push(MetaSecretHint {
            name: name.to_string(),
            role,
            scope: scope.to_string(),
        });
    }
    Ok(out)
}

/// Parse `placeholders` from a parsed meta.json value, strict.
///
/// Returns a list of `(name, MetaPlaceholder)` tuples. Returns an error if any
/// placeholder carries an unknown scope value. Placeholders without a `scope`
/// field are valid — they default to `"environment"` at consumer side
/// (modelled as `MetaPlaceholder.scope = None`).
pub fn parse_meta_placeholders(
    template_name: &str,
    meta: &Value,
) -> Result<Vec<(String, MetaPlaceholder)>, String> {
    let Some(obj) = meta.get("placeholders").and_then(|v| v.as_object()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(obj.len());
    for (name, spec) in obj {
        let raw_scope = spec.get("scope").and_then(|v| v.as_str());
        // Validate even when None → ensures unknown values error.
        let _ = validate_placeholder_scope(template_name, name, raw_scope)?;
        out.push((
            name.clone(),
            MetaPlaceholder {
                scope: raw_scope.map(|s| s.to_string()),
            },
        ));
    }
    Ok(out)
}

/// Validate an entire meta.json text (parsed from disk or DB). Errors fail
/// loud, naming the template + field. Returns `Ok(())` on success.
/// Caller passes `template_name` (e.g. `"go"`) for error messages.
///
/// Currently the validation surface = required_secrets scopes + placeholders
/// scopes. Other fields (file_targets, version, label/description i18n) are
/// not validated here — they're consumed by the frontend or render layer with
/// looser checks.
pub fn validate_meta_json(template_name: &str, meta_text: &str) -> Result<(), String> {
    let meta: Value = serde_json::from_str(meta_text)
        .map_err(|e| format!("Template '{}': invalid meta.json — {}", template_name, e))?;
    let _ = parse_meta_secret_hints(template_name, &meta)?;
    let _ = parse_meta_placeholders(template_name, &meta)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── secret scope ─────────────────────────────────────────────────────────

    #[test]
    fn test_validate_secret_scope_accepts_deploy_repo() {
        assert_eq!(
            validate_secret_scope("go", "NPM_EMAIL", Some("deploy_repo")).unwrap(),
            "deploy_repo"
        );
    }

    #[test]
    fn test_validate_secret_scope_accepts_environment() {
        assert_eq!(
            validate_secret_scope("go", "SSH_HOST", Some("environment")).unwrap(),
            "environment"
        );
    }

    #[test]
    fn test_validate_secret_scope_rejects_obsolete_repo() {
        let err = validate_secret_scope("go", "NPM_EMAIL", Some("repo")).unwrap_err();
        assert!(err.contains("obsolete"), "error mentions obsolete: {}", err);
        assert!(err.contains("'repo'"), "error names the bad value");
        assert!(err.contains("'deploy_repo'"), "error names the new value");
        assert!(err.contains("NPM_EMAIL"), "error names the secret");
        assert!(err.contains("'go'"), "error names the template");
        assert!(
            err.contains("v0.31.0"),
            "error names the version that introduced rename"
        );
    }

    #[test]
    fn test_validate_secret_scope_rejects_garbage() {
        let err = validate_secret_scope("go", "X", Some("global")).unwrap_err();
        assert!(err.contains("invalid"), "error mentions invalid: {}", err);
        assert!(err.contains("'global'"));
    }

    #[test]
    fn test_validate_secret_scope_rejects_missing() {
        let err = validate_secret_scope("go", "X", None).unwrap_err();
        assert!(err.contains("missing"), "error mentions missing: {}", err);
    }

    // ── placeholder scope ────────────────────────────────────────────────────

    #[test]
    fn test_validate_placeholder_scope_accepts_repo() {
        assert_eq!(
            validate_placeholder_scope("go", "GO_VERSION", Some("repo")).unwrap(),
            "repo"
        );
    }

    #[test]
    fn test_validate_placeholder_scope_accepts_environment() {
        assert_eq!(
            validate_placeholder_scope("go", "DOMAIN", Some("environment")).unwrap(),
            "environment"
        );
    }

    #[test]
    fn test_validate_placeholder_scope_absent_defaults_to_environment() {
        // Absence is legal — caller treats None as "environment".
        assert_eq!(
            validate_placeholder_scope("go", "DOMAIN", None).unwrap(),
            "environment"
        );
    }

    #[test]
    fn test_validate_placeholder_scope_rejects_unknown() {
        let err = validate_placeholder_scope("go", "X", Some("deploy_repo")).unwrap_err();
        assert!(err.contains("invalid"), "error mentions invalid: {}", err);
        assert!(err.contains("'deploy_repo'"));
        assert!(err.contains("'repo' | 'environment'"));
    }

    // ── parse_meta_secret_hints (full meta.json scenarios) ───────────────────

    #[test]
    fn test_parse_secret_hints_happy_path_v5() {
        let meta: Value = serde_json::json!({
            "required_secrets": [
                {"name": "SSH_HOST", "role": "deploy", "scope": "environment"},
                {"name": "NPM_EMAIL", "role": "deploy", "scope": "deploy_repo"}
            ]
        });
        let hints = parse_meta_secret_hints("go", &meta).unwrap();
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].name, "SSH_HOST");
        assert_eq!(hints[0].scope, "environment");
        assert_eq!(hints[1].name, "NPM_EMAIL");
        assert_eq!(hints[1].scope, "deploy_repo");
    }

    #[test]
    fn test_parse_secret_hints_strict_rejects_old_repo_value() {
        // Custom template (is_custom=1) carrying obsolete value → load fails.
        let meta: Value = serde_json::json!({
            "required_secrets": [
                {"name": "SSH_HOST", "role": "deploy", "scope": "environment"},
                {"name": "NPM_EMAIL", "role": "deploy", "scope": "repo"}
            ]
        });
        let err = parse_meta_secret_hints("custom_template", &meta).unwrap_err();
        assert!(err.contains("obsolete"));
        assert!(err.contains("NPM_EMAIL"));
        assert!(err.contains("custom_template"));
    }

    #[test]
    fn test_parse_secret_hints_empty_when_section_absent() {
        let meta: Value = serde_json::json!({"placeholders": {}});
        let hints = parse_meta_secret_hints("any", &meta).unwrap();
        assert_eq!(hints.len(), 0);
    }

    // ── parse_meta_placeholders ──────────────────────────────────────────────

    #[test]
    fn test_parse_placeholders_with_repo_scope() {
        let meta: Value = serde_json::json!({
            "placeholders": {
                "GO_VERSION": {"default": "alpine", "type": "string", "scope": "repo"},
                "DOMAIN": {"default": "", "type": "string"}
            }
        });
        let phs = parse_meta_placeholders("go", &meta).unwrap();
        assert_eq!(phs.len(), 2);
        let by_name: std::collections::HashMap<_, _> = phs.into_iter().collect();
        assert_eq!(by_name["GO_VERSION"].scope.as_deref(), Some("repo"));
        assert!(
            by_name["DOMAIN"].scope.is_none(),
            "absent scope stays None — consumer defaults to environment"
        );
    }

    #[test]
    fn test_parse_placeholders_rejects_unknown_scope() {
        let meta: Value = serde_json::json!({
            "placeholders": {
                "X": {"default": "", "type": "string", "scope": "weird_value"}
            }
        });
        let err = parse_meta_placeholders("bad", &meta).unwrap_err();
        assert!(err.contains("invalid"), "error mentions invalid: {}", err);
        assert!(err.contains("'weird_value'"));
        assert!(err.contains("'bad'"));
    }

    // ── validate_meta_json (top-level entrypoint) ────────────────────────────

    #[test]
    fn test_validate_meta_json_bundled_go_loads_cleanly() {
        // Bundled go meta.json — after Task 2 edits — must validate.
        let txt = include_str!("../templates/go/meta.json");
        validate_meta_json("go", txt).expect("bundled go meta.json must validate");
    }

    #[test]
    fn test_validate_meta_json_bundled_flutter_web_loads_cleanly() {
        let txt = include_str!("../templates/flutter_web/meta.json");
        validate_meta_json("flutter_web", txt)
            .expect("bundled flutter_web meta.json must validate");
    }

    #[test]
    fn test_validate_meta_json_fails_on_legacy_secret_repo() {
        // Simulates a custom template (is_custom=1) that was hand-edited and
        // still carries the pre-v0.31.0 `"scope": "repo"` on a secret.
        let legacy = r#"{
            "display_name": "legacy",
            "placeholders": {},
            "required_secrets": [
                {"name": "NPM_EMAIL", "role": "deploy", "scope": "repo"}
            ],
            "file_targets": {},
            "version": 4
        }"#;
        let err = validate_meta_json("legacy", legacy).unwrap_err();
        assert!(err.contains("obsolete"), "error mentions obsolete: {}", err);
        assert!(err.contains("NPM_EMAIL"));
        assert!(err.contains("v0.31.0"));
    }

    #[test]
    fn test_validate_meta_json_fails_on_invalid_json() {
        let err = validate_meta_json("broken", "{not json").unwrap_err();
        assert!(err.contains("invalid meta.json"));
        assert!(err.contains("'broken'"));
    }
}
