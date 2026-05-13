use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@@(\w+)@@").expect("valid placeholder regex"))
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
        let build_args = render_build_args(&["API_BASE_URL".to_string(), "APP_API_KEY".to_string()]);
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
        assert!(rendered.contains("environment: prod"), "each job declares environment");
        assert!(rendered.contains("com.docker.compose.project=swan_support_prod"));
        assert!(rendered.contains("--network swan_support_prod_proxy-network"));
        assert!(rendered.contains("API_BASE_URL=${{ secrets.API_BASE_URL }}"));
        assert!(rendered.contains("APP_API_KEY=${{ secrets.APP_API_KEY }}"));
        assert!(!rendered.contains("CONTAINER_NAME_PROD"), "no hardcoded _PROD suffix");
        assert!(!rendered.contains("${{ secrets.CONTAINER_NAME }}"), "CONTAINER_NAME is now a placeholder, not a secret");
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
        assert!(rendered.contains("environment: prod"), "job must declare GitHub environment");
        assert!(rendered.contains("com.docker.compose.service=swan-backend"));
        assert!(rendered.contains("com.docker.compose.project=swan_prod"));
        assert!(rendered.contains("--network swan_prod_proxy-network"));
        assert!(rendered.contains("DOMAIN=backend.swanqu.tech"));
        assert!(rendered.contains("forward_port:8080"));
        assert!(rendered.contains(r#"ENV_FILE="/home/sda1991/swan_backend.env""#));
        assert!(!rendered.contains("CONTAINER_NAME_PROD"), "legacy hardcoded suffix must be gone");
        assert!(!rendered.contains("${{ secrets.CONTAINER_NAME }}"), "CONTAINER_NAME is now a placeholder, not a secret");
        assert!(rendered.contains("--name swan-backend-prod"));
        assert!(rendered.contains("FORWARD_HOST=swan-backend-prod"));
    }

    #[test]
    fn test_go_deploy_yml_with_runtime_env_args() {
        let tmpl = include_str!("../templates/go/deploy.yml.tmpl");
        let runtime = render_runtime_env_args(&["DATABASE_URL".to_string(), "JWT_SECRET".to_string()]);
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
        assert!(rendered.contains("            A=${{ secrets.A }}"), "first secret 12-space indent");
        assert!(rendered.contains("            B=${{ secrets.B }}"), "second secret 12-space indent");
        assert!(rendered.contains("            C=${{ secrets.C }}"), "third secret 12-space indent");
        // Negative: 10-space prefix would mean wrong indent (sibling of build-args)
        assert!(!rendered.contains("\n          B=${{ secrets.B }}"), "second secret must NOT be at 10 spaces");
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
    fn test_render_dart_defines_emits_one_flag_per_secret() {
        let secrets = vec!["API_BASE_URL".to_string(), "APP_API_KEY".to_string()];
        let out = render_dart_defines(&secrets);
        assert!(out.contains("--dart-define=API_BASE_URL=${API_BASE_URL}"));
        assert!(out.contains("--dart-define=APP_API_KEY=${APP_API_KEY}"));
    }

    #[test]
    fn test_flutter_dockerfile_renders_dynamic_args_and_defines() {
        let tmpl = include_str!("../templates/flutter_web/dockerfile.tmpl");
        let args = render_dockerfile_args(&["API_BASE_URL".to_string(), "APP_API_KEY".to_string(), "STRIPE_KEY".to_string()]);
        let defines = render_dart_defines(&["API_BASE_URL".to_string(), "APP_API_KEY".to_string(), "STRIPE_KEY".to_string()]);
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
        let v = vars(&[
            ("DOCKERFILE_ARGS", ""),
            ("DART_DEFINES", ""),
        ]);
        let rendered = render_template(tmpl, &v).expect("must render with no build secrets");
        // Must still be a valid Dockerfile structure
        assert!(rendered.to_lowercase().contains("from "), "has base image directive");
        assert!(rendered.to_lowercase().contains("copy"), "has COPY directives");
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
        assert!(!rendered.contains("# COPY --from=builder /src/migrations"),
                "migrations COPY must NOT be commented out by default");
        // Regression: guard against the WORKDIR/-o collision that produced
        // `exec: "./app": stat ./app: no such file or directory` when BINARY_NAME defaulted to "app".
        assert!(!rendered.contains("WORKDIR /app\n\n# git"),
                "builder WORKDIR must NOT be /app (collides with default BINARY_NAME=app)");
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
        assert!(!rendered.contains("golang:alpine-alpine"), "no double-suffix");
        assert!(!rendered.contains("golang:-alpine"), "no leading-dash mangling");
    }

    #[test]
    fn test_go_dockerfile_renders_dockerfile_args_block() {
        // B-000010d: DOCKERFILE_ARGS block declares ARG NAME for each build-role
        // secret in the UNION across envs. Empty when no build-role secrets.
        let tmpl = include_str!("../templates/go/dockerfile.tmpl");
        let args_block = render_dockerfile_args(&["API_KEY".to_string(), "BUILD_TOKEN".to_string()]);
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
}
