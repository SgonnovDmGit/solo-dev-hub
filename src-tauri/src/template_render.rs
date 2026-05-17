use crate::models::MetaPlaceholder;
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@@(\w+)@@").expect("valid placeholder regex"))
}

/// T-000103 Task 3 (v0.31.0): schema-aware placeholder merger.
///
/// Iterates over `meta_placeholders` (the parsed `placeholders` section of a
/// template's `meta.json`) and sources each placeholder's value by its
/// `scope` field:
///   - `scope == "repo"`         → look up in `repo_config`
///   - `scope == "environment"`  → look up in `env_extras` (this is the default
///                                  when scope is absent)
///   - other values              → ignored (strict parser already rejected them
///                                  upstream — this branch is defense-in-depth)
///
/// If the chosen source has a non-empty value for the placeholder, that
/// value is inserted. Otherwise the placeholder is left out of the resulting
/// map — callers either supply a default from elsewhere (e.g. raw `default`
/// field on the JSON) before invoking this fn, OR `render_template` will
/// report a missing-key error.
///
/// **Orphan keys are ignored.** Keys that appear in `repo_config` or
/// `env_extras` but are NOT declared in `meta_placeholders` never reach the
/// output map — the template substitution pass therefore won't see them.
/// This is the design point: only the contract declared by the template's
/// meta.json reaches the renderer; runtime state (extras / repo_config) is
/// filtered by the schema, not the other way around.
///
/// The fn is pure (no I/O), which makes it directly unit-testable.
pub fn build_placeholder_vars(
    meta_placeholders: &[(String, MetaPlaceholder)],
    repo_config: &HashMap<String, String>,
    env_extras: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for (name, spec) in meta_placeholders {
        let source: Option<&String> = match spec.scope.as_deref().unwrap_or("environment") {
            "repo" => repo_config.get(name),
            "environment" => env_extras.get(name),
            _ => None,
        };
        if let Some(v) = source {
            // Empty-string values are treated as "not set" — fall back to whatever
            // the caller already populated (defaults from meta.json `default`).
            if !v.is_empty() {
                out.insert(name.clone(), v.clone());
            }
        }
    }
    out
}

/// Render a template by replacing `@@KEY@@` placeholders from `vars`.
/// Missing key → Err (never silent empty substitution). Extra keys in `vars` are ignored.
pub fn render_template(tmpl: &str, vars: &HashMap<String, String>) -> Result<String, String> {
    let mut missing: Option<String> = None;
    let result = placeholder_re()
        .replace_all(tmpl, |caps: &regex::Captures| {
            let key = &caps[1];
            match vars.get(key) {
                Some(v) => v.clone(),
                None => {
                    if missing.is_none() {
                        missing = Some(key.to_string());
                    }
                    String::new()
                }
            }
        })
        .into_owned();
    if let Some(key) = missing {
        return Err(format!("Missing manifest key: {}", key));
    }
    Ok(result)
}

/// v0.18.0: render build-args for `docker/build-push-action`'s `build-args:` YAML block.
/// Each secret becomes `NAME=${{ secrets.NAME }}` on its own line, indented to match
/// the pipe-delimited block layout in `deploy.yml.tmpl`:
///
/// ```yaml
///           build-args: |
///             @@BUILD_ARGS@@
/// ```
///
/// Indent = 12 spaces — matches the column where `@@BUILD_ARGS@@` sits in the
/// template (10 for `build-args:` parent + 2 for YAML block-scalar content).
/// First secret already has 12 spaces from the template; subsequent secrets get
/// the same indent via leading-newline-join. Earlier (pre-fix) join used 10 spaces
/// which broke YAML — second-and-later secrets landed at the parent map level,
/// becoming siblings of `build-args` instead of continuation. Empty slice →
/// empty string (template `build-args:` block handles empty gracefully).
pub fn render_build_args(secret_names: &[String]) -> String {
    secret_names
        .iter()
        .map(|n| format!("{name}=${{{{ secrets.{name} }}}}", name = n))
        .collect::<Vec<_>>()
        .join("\n            ")
}

/// v0.18.0: render `docker run --env KEY="${{ secrets.KEY }}"` flags for runtime secrets.
/// Used in Go deploy.yml.tmpl's ssh-script `docker run` block.
///
/// IMPORTANT edge case (empty input):
/// Returns "" (empty string) when no runtime secrets — template usage is
/// `              @@RUNTIME_ENV_ARGS@@--network ...` (no trailing ` \\` after placeholder).
/// Empty expansion produces `              --network ...` which is valid bash.
///
/// When NON-empty: returns `--env A=... \\\n              --env B=... \\\n              `
/// (trailing ` \\\n              ` so the next template line — `--network ...` — concatenates
/// into a new multi-line continuation line). This keeps the entire `docker run` command
/// syntactically valid under both zero- and multi-secret scenarios.
pub fn render_runtime_env_args(secret_names: &[String]) -> String {
    if secret_names.is_empty() {
        return String::new();
    }
    let inner = secret_names
        .iter()
        .map(|n| format!("--env {name}=\"${{{{ secrets.{name} }}}}\"", name = n))
        .collect::<Vec<_>>()
        .join(" \\\n              ");
    format!("{} \\\n              ", inner)
}

/// v0.18.0: render `ARG NAME` lines for Dockerfile build-time secret declarations.
/// Used as `@@DOCKERFILE_ARGS@@` placeholder in flutter_web/dockerfile.tmpl.
/// Each secret = own `ARG NAME` line (no default value). Dockerfile uses these
/// as build-time vars passed via `docker build --build-arg NAME=value`.
pub fn render_dockerfile_args(secret_names: &[String]) -> String {
    secret_names
        .iter()
        .map(|n| format!("ARG {}", n))
        .collect::<Vec<_>>()
        .join("\n")
}

/// v0.18.0: render `--dart-define=NAME=${NAME}` flags for Flutter compile-time constants.
/// Used in flutter_web/dockerfile.tmpl's `RUN flutter build web --release ...` line.
/// Each secret is a separate `--dart-define=` flag. Order preserved from input slice.
/// Output is space-separated (single-line RUN command context).
pub fn render_dart_defines(secret_names: &[String]) -> String {
    secret_names
        .iter()
        .map(|n| format!("--dart-define={name}=${{{name}}}", name = n))
        .collect::<Vec<_>>()
        .join(" ")
}

/// v0.31.0 (T-000107): render `ENV NAME=$NAME` lines for the Vite ARG→ENV
/// transition in `vite_static/dockerfile.tmpl`. Used as `@@DOCKERFILE_ENVS@@`.
///
/// Background: Vite reads `import.meta.env.VITE_*` from the build process's
/// environment at `npm run build`. Docker `ARG` lives only inside the
/// Dockerfile statement scope (RUN/FROM/COPY) and is NOT inherited by
/// spawned processes — so the npm child wouldn't see VITE_*. Each secret
/// therefore needs an explicit `ENV NAME=$NAME` after the matching `ARG` to
/// export it into the build-stage environment. Flutter sidesteps this by
/// reading ARGs via `--dart-define=` directly on the compiler CLI; npm has
/// no such hook.
///
/// Each secret = own `ENV NAME=$NAME` line. Empty input → empty string.
pub fn render_dockerfile_envs(secret_names: &[String]) -> String {
    secret_names
        .iter()
        .map(|n| format!("ENV {name}=${name}", name = n))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_basic_substitution() {
        let v = vars(&[("A", "one"), ("B", "two")]);
        assert_eq!(
            render_template("@@A@@ and @@B@@", &v).unwrap(),
            "one and two"
        );
    }

    #[test]
    fn test_missing_key_returns_err() {
        let v = vars(&[("A", "one")]);
        let err = render_template("@@A@@ @@DOMAIN@@", &v).unwrap_err();
        assert!(err.contains("Missing manifest key: DOMAIN"), "got: {}", err);
    }

    #[test]
    fn test_extra_keys_ignored() {
        let v = vars(&[("A", "one"), ("UNUSED", "xyz")]);
        assert_eq!(render_template("@@A@@", &v).unwrap(), "one");
    }

    #[test]
    fn test_repeated_placeholder() {
        let v = vars(&[("X", "value")]);
        assert_eq!(render_template("@@X@@-@@X@@", &v).unwrap(), "value-value");
    }

    #[test]
    fn test_idempotent() {
        let v = vars(&[("A", "one")]);
        let r1 = render_template("@@A@@", &v).unwrap();
        let r2 = render_template("@@A@@", &v).unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn test_regression_flutter_web_deploy_yml_v04() {
        let tmpl = include_str!("../templates/flutter_web/deploy.yml.tmpl");
        let build_args =
            render_build_args(&["API_BASE_URL".to_string(), "APP_API_KEY".to_string()]);
        let v = vars(&[
            ("WORKFLOW_NAME", "SwanQu Support — Deploy"),
            ("IMAGE_TAG", "prod"),
            ("COMPOSE_SERVICE", "swan-support-prod-frontend"),
            ("DOMAIN", "support.swanqu.tech"),
            ("DEPLOY_BRANCH", "master"),
            ("ENV_NAME", "prod"),
            ("NETWORK_NAME", "swan_support_prod_proxy-network"),
            ("CONTAINER_NAME", "swan-support-prod-frontend"),
            ("COMPOSE_PROJECT", "swan_support_prod"),
            ("BUILD_ARGS", build_args.as_str()),
        ]);
        let rendered = render_template(tmpl, &v).expect("flutter_web deploy.yml must render");
        assert!(rendered.contains("name: SwanQu Support — Deploy"));
        assert!(rendered.contains("branches: [ master ]"));
        assert!(
            rendered.contains("environment: prod"),
            "each job declares environment"
        );
        assert!(rendered.contains("com.docker.compose.project=swan_support_prod"));
        assert!(rendered.contains("--network swan_support_prod_proxy-network"));
        assert!(rendered.contains("API_BASE_URL=${{ secrets.API_BASE_URL }}"));
        assert!(rendered.contains("APP_API_KEY=${{ secrets.APP_API_KEY }}"));
        assert!(
            !rendered.contains("CONTAINER_NAME_PROD"),
            "no hardcoded _PROD suffix"
        );
        assert!(
            !rendered.contains("${{ secrets.CONTAINER_NAME }}"),
            "CONTAINER_NAME is now a placeholder, not a secret"
        );
        assert!(rendered.contains("--name swan-support-prod-frontend"));
        assert!(rendered.contains("FORWARD_HOST=swan-support-prod-frontend"));
    }

    /// Smoke-regression: Go deploy.yml.tmpl renders end-to-end with SwanQu values.
    /// Checks all @@placeholders@@ are covered by vars and key phrases appear in output.
    /// Does NOT do byte-equal check (fixture is 180+ lines, not captured by hand).
    #[test]
    fn test_regression_go_swanqu_server_deploy_yml() {
        let tmpl = include_str!("../templates/go/deploy.yml.tmpl");
        let v = vars(&[
            ("WORKFLOW_NAME", "Deploy Go Backend"),
            ("IMAGE_TAG", "latest"),
            ("COMPOSE_SERVICE", "swan-backend"),
            ("DOMAIN", "backend.swanqu.tech"),
            ("DEPLOY_BRANCH", "main"),
            ("APP_PORT", "8080"),
            ("ENV_FILE_PATH", "/home/sda1991/swan_backend.env"),
            ("ENV_NAME", "prod"),
            ("NETWORK_NAME", "swan_prod_proxy-network"),
            ("CONTAINER_NAME", "swan-backend-prod"),
            ("COMPOSE_PROJECT", "swan_prod"),
            ("RUNTIME_ENV_ARGS", ""),
            ("BUILD_ARGS", ""),
        ]);
        let rendered = render_template(tmpl, &v).expect("Go deploy.yml must render cleanly");
        assert!(rendered.contains("name: Deploy Go Backend"));
        assert!(rendered.contains("branches: [ main ]"));
        assert!(
            rendered.contains("environment: prod"),
            "job must declare GitHub environment"
        );
        assert!(rendered.contains("com.docker.compose.service=swan-backend"));
        assert!(rendered.contains("com.docker.compose.project=swan_prod"));
        assert!(rendered.contains("--network swan_prod_proxy-network"));
        assert!(rendered.contains("DOMAIN=backend.swanqu.tech"));
        assert!(rendered.contains("forward_port:8080"));
        assert!(rendered.contains(r#"ENV_FILE="/home/sda1991/swan_backend.env""#));
        assert!(
            !rendered.contains("CONTAINER_NAME_PROD"),
            "legacy hardcoded suffix must be gone"
        );
        assert!(
            !rendered.contains("${{ secrets.CONTAINER_NAME }}"),
            "CONTAINER_NAME is now a placeholder, not a secret"
        );
        assert!(rendered.contains("--name swan-backend-prod"));
        assert!(rendered.contains("FORWARD_HOST=swan-backend-prod"));
    }

    #[test]
    fn test_go_deploy_yml_with_runtime_env_args() {
        let tmpl = include_str!("../templates/go/deploy.yml.tmpl");
        let runtime =
            render_runtime_env_args(&["DATABASE_URL".to_string(), "JWT_SECRET".to_string()]);
        let v = vars(&[
            ("WORKFLOW_NAME", "Deploy"),
            ("IMAGE_TAG", "prod"),
            ("COMPOSE_SERVICE", "app"),
            ("DOMAIN", "x.com"),
            ("DEPLOY_BRANCH", "master"),
            ("APP_PORT", "8080"),
            ("ENV_FILE_PATH", ""),
            ("ENV_NAME", "prod"),
            ("NETWORK_NAME", "app_prod_net"),
            ("CONTAINER_NAME", "app-prod"),
            ("COMPOSE_PROJECT", "app_prod"),
            ("RUNTIME_ENV_ARGS", runtime.as_str()),
            ("BUILD_ARGS", ""),
        ]);
        let rendered = render_template(tmpl, &v).expect("must render");
        assert!(rendered.contains("--env DATABASE_URL=\"${{ secrets.DATABASE_URL }}\""));
        assert!(rendered.contains("--env JWT_SECRET=\"${{ secrets.JWT_SECRET }}\""));
    }

    #[test]
    fn test_go_deploy_yml_env_file_empty_renders_cleanly() {
        // ENV_FILE_PATH empty — bash conditional in script handles this:
        // ENV_FILE="" → ENV_FILE_ARG="" → docker run without --env-file flag.
        let tmpl = include_str!("../templates/go/deploy.yml.tmpl");
        let v = vars(&[
            ("WORKFLOW_NAME", "Deploy"),
            ("IMAGE_TAG", "prod"),
            ("COMPOSE_SERVICE", "app"),
            ("DOMAIN", "x.example.com"),
            ("DEPLOY_BRANCH", "main"),
            ("APP_PORT", "8080"),
            ("ENV_FILE_PATH", ""),
            ("ENV_NAME", "prod"),
            ("NETWORK_NAME", "app_prod_net"),
            ("CONTAINER_NAME", "app-prod"),
            ("COMPOSE_PROJECT", "app_prod"),
            ("RUNTIME_ENV_ARGS", ""),
            ("BUILD_ARGS", ""),
        ]);
        let rendered = render_template(tmpl, &v).expect("must render with empty ENV_FILE_PATH");
        assert!(rendered.contains(r#"ENV_FILE="""#));
        assert!(!rendered.contains("--env-file @@"));
    }

    #[test]
    fn test_render_build_args_emits_one_per_secret_with_indent() {
        // Indent must be 12 spaces — matches the column where `@@BUILD_ARGS@@` sits
        // in deploy.yml.tmpl (under `build-args: |`). Pre-fix used 10 spaces which
        // produced invalid YAML (second secret became sibling of build-args).
        let secrets = vec!["API_BASE_URL".to_string(), "APP_API_KEY".to_string()];
        let out = render_build_args(&secrets);
        assert_eq!(
            out,
            "API_BASE_URL=${{ secrets.API_BASE_URL }}\n            APP_API_KEY=${{ secrets.APP_API_KEY }}",
        );
    }

    /// Regression: render real flutter_web template and verify the second-and-later
    /// build-args lines align with the first one (column 12). Catches indent drift
    /// in either the template or the joiner.
    #[test]
    fn test_build_args_indent_aligned_in_rendered_yaml() {
        let tmpl = include_str!("../templates/flutter_web/deploy.yml.tmpl");
        let build_args = render_build_args(&["A".to_string(), "B".to_string(), "C".to_string()]);
        let v = vars(&[
            ("WORKFLOW_NAME", "W"),
            ("IMAGE_TAG", "prod"),
            ("COMPOSE_SERVICE", "s"),
            ("DOMAIN", "d"),
            ("DEPLOY_BRANCH", "m"),
            ("ENV_NAME", "prod"),
            ("NETWORK_NAME", "n"),
            ("CONTAINER_NAME", "c"),
            ("COMPOSE_PROJECT", "p"),
            ("BUILD_ARGS", build_args.as_str()),
        ]);
        let rendered = render_template(tmpl, &v).unwrap();
        // Each secret line must start at exactly 12 spaces — same column as the first.
        assert!(
            rendered.contains("            A=${{ secrets.A }}"),
            "first secret 12-space indent"
        );
        assert!(
            rendered.contains("            B=${{ secrets.B }}"),
            "second secret 12-space indent"
        );
        assert!(
            rendered.contains("            C=${{ secrets.C }}"),
            "third secret 12-space indent"
        );
        // Negative: 10-space prefix would mean wrong indent (sibling of build-args)
        assert!(
            !rendered.contains("\n          B=${{ secrets.B }}"),
            "second secret must NOT be at 10 spaces"
        );
    }

    #[test]
    fn test_render_build_args_empty_returns_empty() {
        let out = render_build_args(&[]);
        assert_eq!(out, "");
    }

    #[test]
    fn test_render_runtime_env_args_emits_docker_flags_with_trailing_continuation() {
        let secrets = vec!["DATABASE_URL".to_string(), "JWT_SECRET".to_string()];
        let out = render_runtime_env_args(&secrets);
        assert!(out.contains("--env DATABASE_URL=\"${{ secrets.DATABASE_URL }}\""));
        assert!(out.contains("--env JWT_SECRET=\"${{ secrets.JWT_SECRET }}\""));
        // Non-empty output MUST end with " \\\n              " — backslash-continuation + indent
        // so the next template line ( --network ... ) joins cleanly.
        assert!(out.ends_with(" \\\n              "),
                "non-empty runtime args must end with backslash + indent so next line concatenates: got {:?}", out);
    }

    #[test]
    fn test_render_runtime_env_args_empty_returns_empty_string() {
        // Empty = no-op expansion. Combined with template pattern `@@RUNTIME_ENV_ARGS@@--network ...`,
        // empty expansion produces valid `              --network ...` line in bash script.
        assert_eq!(render_runtime_env_args(&[]), "");
    }

    #[test]
    fn test_render_dockerfile_args_emits_arg_lines() {
        let secrets = vec!["API_BASE_URL".to_string(), "APP_API_KEY".to_string()];
        let out = render_dockerfile_args(&secrets);
        assert_eq!(out, "ARG API_BASE_URL\nARG APP_API_KEY");
    }

    #[test]
    fn test_render_dockerfile_envs_emits_env_lines() {
        // T-000107: ENV NAME=$NAME per secret, newline-joined. Vite needs
        // these because `npm run build` reads VITE_* from process.env, not
        // from Docker's ARG scope.
        let secrets = vec!["VITE_API_BASE".to_string(), "VITE_APP_KEY".to_string()];
        let out = render_dockerfile_envs(&secrets);
        assert_eq!(
            out,
            "ENV VITE_API_BASE=$VITE_API_BASE\nENV VITE_APP_KEY=$VITE_APP_KEY"
        );
        // Empty input → empty string (symmetric to render_dockerfile_args).
        assert_eq!(render_dockerfile_envs(&[]), "");
    }

    #[test]
    fn test_render_dart_defines_emits_one_flag_per_secret() {
        let secrets = vec!["API_BASE_URL".to_string(), "APP_API_KEY".to_string()];
        let out = render_dart_defines(&secrets);
        assert!(out.contains("--dart-define=API_BASE_URL=${API_BASE_URL}"));
        assert!(out.contains("--dart-define=APP_API_KEY=${APP_API_KEY}"));
    }

    #[test]
    fn test_flutter_dockerfile_renders_dynamic_args_and_defines() {
        let tmpl = include_str!("../templates/flutter_web/dockerfile.tmpl");
        let args = render_dockerfile_args(&[
            "API_BASE_URL".to_string(),
            "APP_API_KEY".to_string(),
            "STRIPE_KEY".to_string(),
        ]);
        let defines = render_dart_defines(&[
            "API_BASE_URL".to_string(),
            "APP_API_KEY".to_string(),
            "STRIPE_KEY".to_string(),
        ]);
        let v = vars(&[
            ("DOCKERFILE_ARGS", args.as_str()),
            ("DART_DEFINES", defines.as_str()),
        ]);
        let rendered = render_template(tmpl, &v).expect("flutter dockerfile must render");
        assert!(rendered.contains("ARG API_BASE_URL"));
        assert!(rendered.contains("ARG APP_API_KEY"));
        assert!(rendered.contains("ARG STRIPE_KEY"));
        assert!(rendered.contains("--dart-define=API_BASE_URL=${API_BASE_URL}"));
        assert!(rendered.contains("--dart-define=APP_API_KEY=${APP_API_KEY}"));
        assert!(rendered.contains("--dart-define=STRIPE_KEY=${STRIPE_KEY}"));
    }

    #[test]
    fn test_flutter_dockerfile_renders_with_empty_args() {
        let tmpl = include_str!("../templates/flutter_web/dockerfile.tmpl");
        let v = vars(&[("DOCKERFILE_ARGS", ""), ("DART_DEFINES", "")]);
        let rendered = render_template(tmpl, &v).expect("must render with no build secrets");
        // Must still be a valid Dockerfile structure
        assert!(
            rendered.to_lowercase().contains("from "),
            "has base image directive"
        );
        assert!(
            rendered.to_lowercase().contains("copy"),
            "has COPY directives"
        );
    }

    /// T-000107: vite_static/dockerfile.tmpl renders with VITE_* build secrets,
    /// repo-scope placeholders (NODE_VERSION/BUILD_OUTPUT_DIR/PRE_BUILD_COMMAND),
    /// and the new @@DOCKERFILE_ENVS@@ ARG→ENV transition block.
    #[test]
    fn test_vite_static_dockerfile_renders_with_vite_secrets() {
        let tmpl = include_str!("../templates/vite_static/dockerfile.tmpl");
        let args =
            render_dockerfile_args(&["VITE_API_BASE".to_string(), "VITE_APP_KEY".to_string()]);
        let envs =
            render_dockerfile_envs(&["VITE_API_BASE".to_string(), "VITE_APP_KEY".to_string()]);
        let v = vars(&[
            ("NODE_VERSION", "lts-alpine"),
            ("BUILD_OUTPUT_DIR", "dist"),
            ("PRE_BUILD_COMMAND", "true"),
            ("DOCKERFILE_ARGS", args.as_str()),
            ("DOCKERFILE_ENVS", envs.as_str()),
        ]);
        let rendered = render_template(tmpl, &v).expect("vite_static dockerfile must render");

        // Build stage uses correct node image
        assert!(
            rendered.contains("FROM node:lts-alpine"),
            "node base image with NODE_VERSION substituted"
        );

        // ARG NAME and ENV NAME=$NAME both present per VITE_* secret
        assert!(rendered.contains("ARG VITE_API_BASE"));
        assert!(rendered.contains("ARG VITE_APP_KEY"));
        assert!(rendered.contains("ENV VITE_API_BASE=$VITE_API_BASE"));
        assert!(rendered.contains("ENV VITE_APP_KEY=$VITE_APP_KEY"));

        // npm flow — deterministic lockfile install, no fallback
        assert!(rendered.contains("RUN npm ci"));
        assert!(
            !rendered.contains("npm install"),
            "deterministic install — no ergonomic fallback"
        );

        // PRE_BUILD_COMMAND (default = shell no-op `true`) and main build
        assert!(rendered.contains("RUN true"));
        assert!(rendered.contains("RUN npm run build"));

        // Runtime stage: nginx serves the build output dir
        assert!(rendered.contains("FROM nginx:alpine"));
        assert!(rendered.contains("COPY --from=builder /app/dist /usr/share/nginx/html"));
        assert!(rendered.contains("EXPOSE 80"));
    }

    /// T-000107: vite_static/deploy.yml.tmpl renders cleanly with VITE_* build
    /// secrets in the build-args block. Mirrors test_regression_flutter_web_deploy_yml_v04
    /// — vite shares the deploy stage byte-for-byte with flutter_web.
    #[test]
    fn test_vite_static_deploy_yml_renders_clean() {
        let tmpl = include_str!("../templates/vite_static/deploy.yml.tmpl");
        let build_args =
            render_build_args(&["VITE_API_BASE".to_string(), "VITE_APP_KEY".to_string()]);
        let v = vars(&[
            ("WORKFLOW_NAME", "Deploy Vite Static"),
            ("IMAGE_TAG", "prod"),
            ("COMPOSE_SERVICE", "lcm-landing-prod"),
            ("DOMAIN", "lcm.example.com"),
            ("DEPLOY_BRANCH", "master"),
            ("ENV_NAME", "prod"),
            ("NETWORK_NAME", "lcm_prod_proxy-network"),
            ("CONTAINER_NAME", "lcm-landing-prod"),
            ("COMPOSE_PROJECT", "lcm_prod"),
            ("BUILD_ARGS", build_args.as_str()),
        ]);
        let rendered = render_template(tmpl, &v).expect("vite_static deploy.yml must render");

        assert!(rendered.contains("name: Deploy Vite Static"));
        assert!(rendered.contains("branches: [ master ]"));
        assert!(
            rendered.contains("environment: prod"),
            "each job declares environment"
        );
        assert!(rendered.contains("com.docker.compose.project=lcm_prod"));
        assert!(rendered.contains("--network lcm_prod_proxy-network"));
        assert!(rendered.contains("VITE_API_BASE=${{ secrets.VITE_API_BASE }}"));
        assert!(rendered.contains("VITE_APP_KEY=${{ secrets.VITE_APP_KEY }}"));
        assert!(rendered.contains("--name lcm-landing-prod"));
        assert!(rendered.contains("FORWARD_HOST=lcm-landing-prod"));
    }

    /// Smoke-regression: Go dockerfile.tmpl renders with SwanQu values.
    #[test]
    fn test_regression_go_swanqu_server_dockerfile() {
        // v0.29.2: GO_VERSION now holds the FULL Docker Hub tag (incl. variant
        // suffix), not just the version number. Template no longer hardcodes
        // `-alpine`. Default is `alpine` (latest stable Go on alpine); users
        // can pin to `1.26-alpine`, `1.26.0-alpine`, `bookworm`, etc.
        let tmpl = include_str!("../templates/go/dockerfile.tmpl");
        let v = vars(&[
            ("GO_VERSION", "1.26-alpine"),
            ("BINARY_NAME", "swan-server"),
            ("ENTRY_POINT", "./cmd/api/"),
            ("APP_PORT", "8080"),
            ("DOCKERFILE_ARGS", ""),
        ]);
        let rendered = render_template(tmpl, &v).expect("Go dockerfile must render cleanly");
        assert!(rendered.contains("FROM golang:1.26-alpine AS builder"));
        assert!(rendered.contains("go build -o /out/swan-server ./cmd/api/"));
        assert!(rendered.contains("COPY --from=builder /out/swan-server ./"));
        assert!(rendered.contains("EXPOSE 8080"));
        assert!(rendered.contains(r#"CMD ["./swan-server"]"#));
        // B-000010c: migrations COPY is now uncommented by default (Go web
        // servers typically embed migrations). User can comment it manually
        // if the project doesn't ship a /src/migrations folder.
        assert!(rendered.contains("COPY --from=builder /src/migrations ./migrations"));
        assert!(
            !rendered.contains("# COPY --from=builder /src/migrations"),
            "migrations COPY must NOT be commented out by default"
        );
        // Regression: guard against the WORKDIR/-o collision that produced
        // `exec: "./app": stat ./app: no such file or directory` when BINARY_NAME defaulted to "app".
        assert!(
            !rendered.contains("WORKDIR /app\n\n# git"),
            "builder WORKDIR must NOT be /app (collides with default BINARY_NAME=app)"
        );
    }

    #[test]
    fn test_go_dockerfile_renders_bare_alpine_default() {
        // v0.29.2: default GO_VERSION is `alpine` — Docker Hub auto-tracks
        // latest stable Go on alpine. Template must produce `golang:alpine`
        // without any mangled prefix/suffix.
        let tmpl = include_str!("../templates/go/dockerfile.tmpl");
        let v = vars(&[
            ("GO_VERSION", "alpine"),
            ("BINARY_NAME", "app"),
            ("ENTRY_POINT", "./cmd/api/"),
            ("APP_PORT", "8080"),
            ("DOCKERFILE_ARGS", ""),
        ]);
        let rendered = render_template(tmpl, &v).expect("must render with bare 'alpine' tag");
        assert!(rendered.contains("FROM golang:alpine AS builder"));
        assert!(
            !rendered.contains("golang:alpine-alpine"),
            "no double-suffix"
        );
        assert!(
            !rendered.contains("golang:-alpine"),
            "no leading-dash mangling"
        );
    }

    #[test]
    fn test_go_dockerfile_renders_dockerfile_args_block() {
        // B-000010d: DOCKERFILE_ARGS block declares ARG NAME for each build-role
        // secret in the UNION across envs. Empty when no build-role secrets.
        let tmpl = include_str!("../templates/go/dockerfile.tmpl");
        let args_block =
            render_dockerfile_args(&["API_KEY".to_string(), "BUILD_TOKEN".to_string()]);
        let v = vars(&[
            ("GO_VERSION", "1.26"),
            ("BINARY_NAME", "app"),
            ("ENTRY_POINT", "./cmd/api/"),
            ("APP_PORT", "8080"),
            ("DOCKERFILE_ARGS", args_block.as_str()),
        ]);
        let rendered = render_template(tmpl, &v).expect("Go dockerfile must render with ARGs");
        assert!(rendered.contains("ARG API_KEY"));
        assert!(rendered.contains("ARG BUILD_TOKEN"));
    }

    // ── T-000103 Task 3: scope-aware placeholder merger ──────────────────────

    fn ph(scope: Option<&str>) -> MetaPlaceholder {
        MetaPlaceholder {
            scope: scope.map(|s| s.to_string()),
        }
    }

    fn smap(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_render_uses_repo_scope_for_repo_placeholders() {
        // Same key name in BOTH repo_config AND env_extras with DIFFERENT
        // values. Merger must pick by scope:
        //   - GO_VERSION (scope=repo)        → repo_config value
        //   - DOMAIN     (scope=environment) → env_extras  value
        // NOT a chain-merge or last-wins.
        let meta = vec![
            ("GO_VERSION".to_string(), ph(Some("repo"))),
            ("DOMAIN".to_string(), ph(Some("environment"))),
        ];
        let repo_config = smap(&[
            ("GO_VERSION", "1.26-alpine"),
            // Intentionally also has DOMAIN — must be IGNORED because DOMAIN is
            // env-scope, not repo-scope.
            ("DOMAIN", "wrong-source.example.com"),
        ]);
        let env_extras = smap(&[
            // Intentionally also has GO_VERSION — must be IGNORED because
            // GO_VERSION is repo-scope, not env-scope.
            ("GO_VERSION", "wrong-source-1.20"),
            ("DOMAIN", "prod.example.com"),
        ]);

        let vars = build_placeholder_vars(&meta, &repo_config, &env_extras);

        assert_eq!(
            vars.get("GO_VERSION").map(|s| s.as_str()),
            Some("1.26-alpine"),
            "repo-scope placeholder must come from repo_config",
        );
        assert_eq!(
            vars.get("DOMAIN").map(|s| s.as_str()),
            Some("prod.example.com"),
            "environment-scope placeholder must come from env_extras",
        );
    }

    #[test]
    fn test_render_uses_environment_scope_when_scope_absent() {
        // Placeholders without an explicit scope field default to
        // "environment" — source from env_extras.
        let meta = vec![("WORKFLOW_NAME".to_string(), ph(None))];
        let repo_config = smap(&[("WORKFLOW_NAME", "from_repo")]);
        let env_extras = smap(&[("WORKFLOW_NAME", "from_env")]);

        let vars = build_placeholder_vars(&meta, &repo_config, &env_extras);

        assert_eq!(
            vars.get("WORKFLOW_NAME").map(|s| s.as_str()),
            Some("from_env"),
            "absent scope defaults to 'environment' — source from env_extras",
        );
    }

    #[test]
    fn test_render_orphan_keys_ignored() {
        // Keys present in repo_config / env_extras but NOT declared in
        // meta.placeholders must NEVER reach the output map. Combined with
        // the strict-key behavior of `render_template` (errors on missing key
        // in the template), this means orphans are completely inert: the
        // template never sees them as substitution candidates.
        let meta = vec![("DECLARED_KEY".to_string(), ph(Some("environment")))];
        let repo_config = smap(&[
            ("DECLARED_KEY", "should_not_be_picked_for_env_scope"),
            ("ORPHAN_REPO_KEY", "ghost1"),
        ]);
        let env_extras = smap(&[
            ("DECLARED_KEY", "the_real_value"),
            ("ORPHAN_ENV_KEY", "ghost2"),
        ]);

        let vars = build_placeholder_vars(&meta, &repo_config, &env_extras);

        // Only the declared key reached the output.
        assert_eq!(
            vars.len(),
            1,
            "only declared keys may be emitted, got {:?}",
            vars
        );
        assert_eq!(
            vars.get("DECLARED_KEY").map(|s| s.as_str()),
            Some("the_real_value"),
        );
        assert!(
            !vars.contains_key("ORPHAN_REPO_KEY"),
            "orphan in repo_config must be filtered out"
        );
        assert!(
            !vars.contains_key("ORPHAN_ENV_KEY"),
            "orphan in env_extras must be filtered out"
        );

        // Belt-and-suspenders: an actual render pass against a template that
        // only references DECLARED_KEY should succeed and NOT contain the
        // ghost values anywhere in its output.
        let tmpl = "value: @@DECLARED_KEY@@";
        let rendered = render_template(tmpl, &vars).unwrap();
        assert_eq!(rendered, "value: the_real_value");
        assert!(!rendered.contains("ghost1"));
        assert!(!rendered.contains("ghost2"));
    }

    #[test]
    fn test_render_empty_value_in_source_skips_insertion() {
        // Empty-string in the chosen source is treated as "not set" — caller's
        // pre-populated default (from meta.json `default` field) must survive.
        // This matches the existing lib.rs merger behavior: `for (k, v) in
        // env.extras { if !v.is_empty() { vars.insert(...) } }`.
        let meta = vec![("ENV_FILE_PATH".to_string(), ph(Some("environment")))];
        let repo_config: HashMap<String, String> = HashMap::new();
        let env_extras = smap(&[("ENV_FILE_PATH", "")]);

        let vars = build_placeholder_vars(&meta, &repo_config, &env_extras);

        assert!(
            !vars.contains_key("ENV_FILE_PATH"),
            "empty value must NOT shadow caller-supplied default",
        );
    }

    #[test]
    fn test_render_unknown_scope_value_yields_no_value() {
        // Defense-in-depth: strict parser upstream rejects unknown scope
        // values, but if one slips through, the merger must not panic or
        // pick from a random source.
        let meta = vec![("X".to_string(), ph(Some("weird_scope")))];
        let repo_config = smap(&[("X", "from_repo")]);
        let env_extras = smap(&[("X", "from_env")]);

        let vars = build_placeholder_vars(&meta, &repo_config, &env_extras);

        assert!(
            !vars.contains_key("X"),
            "unknown scope value must yield nothing — caller falls back to default",
        );
    }
}
