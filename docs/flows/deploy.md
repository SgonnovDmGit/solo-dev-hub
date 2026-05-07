# Deploy Flow (v0.18.0)

## Architecture

- Each repository can have multiple deploy environments (rows in `deploy_environments`, 1:N). Free-named (`prod`, `test`, `staging`, `client-demo`, ...).
- Each deploy has its own `.github/workflows/deploy-{name}.yml` file + a shared `Dockerfile` (per repo).
- Workflow `on: push: branches: [{deploy_branch}]` triggers on pushes to env-specific branch.

## Secrets layering

- **Repo-level secrets** (managed in SecretsPanel): shared defaults, stored in GitHub repo-level Secrets.
- **Env-level secrets** (managed in DeployScreen per-env tab via Override toggle): env-specific values, stored in GitHub Environment Secrets.
- GitHub native resolution: `${{ secrets.NAME }}` in a workflow job with `environment: test` resolves to env-scoped value if present, else falls back to repo-level, else empty string.
- `deploy_secrets` SQLite table holds flags (`included` / `role` / `override_enabled`) — values never stored in DB.

## Roles (meta.json v4)

| role | Injected as | Used by |
|---|---|---|
| `build` | `docker build --build-arg NAME=...` + `ARG NAME` in Dockerfile + `--dart-define=NAME=${NAME}` (Flutter) | Flutter web compile-time constants (API_BASE_URL, APP_API_KEY) |
| `deploy` | `${{ secrets.NAME }}` in deploy-job context (SSH, NPM creds) | SSH_HOST, NPM_EMAIL, ... |
| `runtime` | `docker run --env NAME="${{ secrets.NAME }}"` | Go backend runtime env (DATABASE_URL, JWT_SECRET, ...) |

Single Dockerfile per repo → `@@DOCKERFILE_ARGS@@` = UNION of `role=build` secrets across all deploy environments.

## Lifecycle

1. User creates deploy `prod` via DeployScreen → app PUTs GitHub environment `prod` + ensures `deploy_secrets` seeded (union of repo secrets + meta.json hints).
2. User fills placeholders (domain, branch, etc.) → auto-save to `deploy_environments`.
3. User toggles Include/Override per secret → PUT/DELETE env-scoped secret.
4. User clicks Generate workflow files → render per-env `deploy-{name}.yml` + shared `Dockerfile` → DiffDialog → write.
5. User pushes to `deploy_branch` → GitHub Actions picks up `deploy-{name}.yml` → workflow runs with `environment: {name}` → secrets resolved per scope.

## UI surface

Two screens cooperate; their roles are strictly separated:

- **Repo Secrets tab (SecretsPanel)** — single source of truth for what secrets EXIST and their repo-level values. Add/remove secrets here; deploys see them via sync-trigger that calls `register_repo_secret_in_deploys` after a successful PUT.
- **DeployScreen** — master-detail. Master = list of deploys (table with copy-icon ⎘ in first column, delete-icon ✕ on hover-right). Detail = placeholders form + `DeploySecretsTable`. The deploy view does NOT add or remove secrets — it only toggles `included` / `override_enabled` flags per existing secret and writes env-scoped values via GitHub API.

`DeploySecretsTable` is a single-line per-secret row: `name [role-chip] [☐ Override] [______ value ______] [☑ Include]`. The role chip is a clickable colored badge (build=indigo, deploy=teal, runtime=amber) that cycles through values on click — most users won't touch it (it's pre-set by meta.json hints), but it's available for custom secrets and edge cases. The value input is always rendered but disabled until both `Include` and `Override` are on; placeholder text describes the current resolution path (`(from repo, updated YYYY-MM-DD)` / `Not set in repo` / `saved YYYY-MM-DD`).

## Clone (Copy from)

Master view's ⎘ icon on each row enters clone mode directly — form shows `Скопировать prod как: [____]` with no source dropdown (source = clicked row). `+ New deployment` shows `Новый деплой: [____]` (no Copy-from selector). Both flows ask only for the new name; branch + domain + other placeholders are filled in on the detail screen.

`clone_deploy_environment` copies placeholders + extras + all `deploy_secrets` flags. Env-scoped secret VALUES are not copied (GitHub API can't read values) — user re-enters them in the new deploy.

## Rename

`name` is read-only after create. To rename: clone with new name, enter override values, delete original.

## Cleanup on delete

- Deploy delete: GitHub environment (cascades env-scoped secrets) → DB row (cascades `deploy_secrets`).
- Repo secret delete: DB-level `deploy_secrets` rows for that name deleted across all envs.
- Old `deploy.yml` (pre-v0.18.0): detected on first Generate, toast warning shown — user removes manually with `git rm`. Full DiffDialog-integrated delete deferred to v0.18.1.
