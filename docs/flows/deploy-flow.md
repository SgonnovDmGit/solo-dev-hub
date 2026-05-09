# Flow: Deploy

**Введён в:** v0.7.0 (T-028); крупно переработан в v0.18.0 (multi-env), v0.25.0 (CONTAINER_NAME placeholder + GH-env auto-ensure + secret-orphan prune)
**Связанные файлы:**
[src-tauri/src/template_render.rs](../../src-tauri/src/template_render.rs),
[src-tauri/src/lib.rs](../../src-tauri/src/lib.rs) (`render_files_for_deploy_env`, `write_deploy_files`),
[src-tauri/src/db.rs](../../src-tauri/src/db.rs) (`ensure_deploy_secrets_populated` + orphan-prune),
[src/lib/components/DeployScreen.svelte](../../src/lib/components/DeployScreen.svelte) (master list),
[src/lib/components/DeployDetail.svelte](../../src/lib/components/DeployDetail.svelte) (per-env edit + Generate),
[src/lib/components/DeploySecretsTable.svelte](../../src/lib/components/DeploySecretsTable.svelte) (per-secret role+scope),
[src/lib/components/DiffDialog.svelte](../../src/lib/components/DiffDialog.svelte)
**Спека шаблона:** [docs/deploy_template_spec.md](../deploy_template_spec.md)

## Суть

Параметризованный шаблон в SQLite (`templates`) + per-deploy-environment запись (`deploy_environments`) → рендерим **реальные файлы** в папке репо (`.github/workflows/deploy-<env>.yml` + `Dockerfile`), показываем diff, пишем по подтверждению.

Архитектура **multi-environment**: один репо может иметь N deploy-инстансов с независимыми именами (`prod` / `test` / `cloud_test` / etc). Каждый получает свой workflow-файл `deploy-<name>.yml` и свой объект GitHub Environment.

## Слои

```
1. Templates (SQLite, _global + per-language buckets)
   ├─ flutter_web/
   │  ├─ deploy.yml.tmpl     (@@PLACEHOLDERS@@)
   │  ├─ dockerfile.tmpl
   │  └─ meta.json           (display_name, placeholders, required_secrets, file_targets)
   └─ go/  (аналогично, + Go-specific placeholders: GO_VERSION, BINARY_NAME, ENTRY_POINT, APP_PORT, ENV_FILE_PATH)

2. Deploy environments (SQLite, 1:N per repo)
   deploy_environments
   ├─ id, repository_id
   ├─ name (UNIQUE per repo, immutable, slot identity)
   ├─ workflow_name, image_tag, compose_service, domain, deploy_branch (5 core columns)
   ├─ extras JSON (все остальные placeholder-значения, в т.ч. CONTAINER_NAME, NETWORK_NAME, COMPOSE_PROJECT, и language-specific)
   ├─ sort_order, updated_at

3. Per-deploy per-secret flags (SQLite)
   deploy_secrets
   ├─ deploy_env_id, secret_name
   ├─ included (bool — участвует ли в этом деплое)
   ├─ role (build / deploy / runtime — куда подставлять в YAML)
   ├─ override_enabled (bool — отдельное env-scoped значение в GitHub Environment, или fallback на repo-level secret)

4. Render (pure Rust function)
   render_template(tmpl, vars) → String
   Missing key → Err "Missing manifest key: X"
   Helper-генераторы для multi-line блоков:
   ├─ render_build_args(secrets) — `BUILD_ARGS` block (12-space indent под `build-args: |`)
   ├─ render_runtime_env_args(secrets) — `--env KEY="${{ secrets.KEY }}"` flags
   ├─ render_dockerfile_args(secrets) — `ARG NAME` lines
   └─ render_dart_defines(secrets) — `--dart-define=KEY=...` flags (flutter_web)

5. Write (Rust command)
   write_deploy_files(deploy_env_id, repo_id, local_path, files: Vec<RenderedFile>) → WriteResult { written, errors }
```

## Трубопровод

### Шаг 1. Выбор `deploy_target` на репо

В RepoDetail meta-row — dropdown с языками из `list_template_languages()`.
Пример: `deploy_target='flutter_web'`. Без `deploy_target` экран Deploy недоступен.

### Шаг 2. Создание deploy-инстанса (DeployScreen master)

Кнопка `+ Новый` или `📋 Клонировать` → форма с именем (UNIQUE per repo, не редактируется потом). На submit:

1. `create_deploy_environment` (или `clone_deploy_environment`) — пишет row в `deploy_environments`.
2. `createEnvironment(owner, repo, name)` — PUT в GitHub API. Создаёт объект Environment в `Settings → Environments`. Идемпотентно (no-op если есть).

После успеха master-list обновляется + автоматически открывается DeployDetail.

### Шаг 3. Заполнение placeholders (DeployDetail)

При **mount** компонента `load()` делает (порядок важен):

1. `getDeployEnvironment(deploy_env_id)` — пуляет row из DB.
2. `getTemplateFile(deploy_target, 'meta.json')` — парсит `placeholders` для построения формы.
3. **`createEnvironment` (idempotent ensure)** — covers 3 кейса где GH-side env мог отсутствовать:
   - legacy envs от migration v20 (`deploy_manifests` → `deploy_environments` автогенерил `name='prod'` без вызова `createEnvironment`)
   - cloned env где createEnvironment упал (race / network)
   - env без override-секретов (раньше неявная цепочка через `createOrUpdateEnvironmentSecret` была единственным trigger'ом)
   API-ошибки surface через warning-toast (`deploy.envCreateFailed` i18n) — даёт диагностику для PAT permission issues (fine-grained PAT без "Environments: write" разрешения).
4. `listBranches` — для DEPLOY_BRANCH datalist.

Форма автосохраняется (`scheduleSave` debounce 400ms) в:
- 5 core колонок DB (workflow_name, image_tag, compose_service, domain, deploy_branch)
- `extras` JSON для всего остального (CONTAINER_NAME, NETWORK_NAME, COMPOSE_PROJECT, language-specific)

`REQUIRED_KEYS = [...CORE_KEYS, 'CONTAINER_NAME']` — Generate-кнопка disabled пока хотя бы одно из required-полей пустое.

### Шаг 4. Per-secret настройка (DeploySecretsTable inside DeployDetail)

Параллельно с placeholder-формой DeploySecretsTable.load() делает:

1. `listRepoSecrets(owner, repo)` — what's in GitHub repo-level Secrets.
2. **`ensureDeploySecretsPopulated(deploy_env_id, repo_secret_names)`** на Rust-стороне:
   - INSERT новые row для secret-имён которых ещё нет в `deploy_secrets` (union: `repo_secret_names` ∪ `meta.required_secrets`).
   - **DELETE orphan rows** — secret_name которых нет ни в repo_secret_names, ни в meta_hints. Покрывает случаи (a) template обновился и убрал secret из required_secrets (как CONTAINER_NAME в v0.25.0), (b) user удалил repo-level секрет в GitHub (B-000003 fix).
   - Caller обязан звать только с successfully-fetched `repo_secret_names` (empty-due-to-failure ложно бы прунил легитимные rows).
3. `listEnvironmentSecrets(envName)` — что лежит в Environment scope конкретно этого env.
4. `listDeploySecrets(deploy_env_id)` — DB state после seed/prune.

Per row UI: `[name] [role-chip: build|deploy|runtime] [☐ override] [value-input] [☑ include]`.

- **role** определяет в какой блок YAML попадёт секрет: `build` → `BUILD_ARGS:`, `deploy` → workflow-level `${{ secrets.X }}`, `runtime` → `docker run --env`.
- **override + value** — пишет env-scoped секрет в GitHub через `createOrUpdateEnvironmentSecret`. Off → `deleteEnvironmentSecret`. GitHub резолвит env→repo fallback автоматически — наш YAML использует один и тот же синтаксис `${{ secrets.X }}` независимо от scope.
- **include off** — секрет вообще не появляется в рендере для этого env.

### Шаг 5. Generate workflow files (DeployDetail)

`render_deploy_files_for_env(deploy_env_id)` на Rust:

1. Читает env, repo, `meta.json` для `file_targets` map.
2. Собирает `vars` HashMap:
   - Defaults из meta.placeholders.
   - 5 core columns из `deploy_environments` row.
   - `extras` JSON overrides defaults.
   - Synthesised vars: `ENV_NAME = env.name`, `BUILD_ARGS`, `RUNTIME_ENV_ARGS`, `DOCKERFILE_ARGS`, `DART_DEFINES` (per role secret-list rendering).
3. Для каждого `*.tmpl` файла из template-bundle, для которого есть mapping в `file_targets`:
   - `target_path = file_targets[file_name].replace("{name}", env.name)` — `deploy.yml.tmpl` → `.github/workflows/deploy-<env>.yml` per-env, `dockerfile.tmpl` → `Dockerfile` shared.
   - `render_template(content, vars)` — `@@KEY@@` substitute. Missing key → Err.
4. Возвращает `Vec<RenderedFile { path, content }>`.

### Шаг 6. Diff preview (DiffDialog)

`readRepoFiles(local_path, rel_paths)` → `Vec<Option<String>>` для существующего content'а.

DiffDialog рендерит per-file:
- **NEW** (existing=null): `<pre>`-превью нового контента, чекбокс `Create` (on).
- **CHANGED** (existing differs): side-by-side через `diff.diffLines`, чекбокс `Overwrite` (on).
- **UNCHANGED**: содержимое не показано, чекбокс disabled.

Кнопка `Write selected` → `confirm()` фильтрует `f.shouldWrite && status(f) !== 'unchanged'` и передаёт в `onConfirm(toWrite: RenderedFile[])`.

### Шаг 7. Write to disk

`write_deploy_files(deploy_env_id, repo_id, local_path, files)` на Rust:
1. `sync::ensure_root_exists(local_path)` — guard, не создаём repo-папку если её нет.
2. Для каждого `RenderedFile`: `create_dir_all(parent)` + `fs::write(target, content)`.
3. Insert `deploy_event` (`type='render'`, `file_count=N`) в timeline.
4. Возвращает `WriteResult { written: [paths], errors: [{ path, error }] }` — partial failures допустимы.

**Контракт TS↔Rust:** `RenderedFile` имеет поля `{ path, content }` (B-000005-fix v0.25.0 — раньше TS-side ошибочно ренеймил `path → rel_path`, serde падал с `missing field path`). `WriteResult` — `{ written: string[], errors: WriteError[] }`. Оба типа shared между сторонами через `src/lib/types.ts` ↔ `src-tauri/src/models.rs`.

## Что происходит в GitHub Actions после push

Workflow триггерится на push в `DEPLOY_BRANCH`. Три job'а последовательно:

```
build-and-push:
  - environment: <ENV_NAME>      ← каждый job декларирует env scope
  - Собирает Docker-образ, build-args=BUILD_ARGS-секреты
  - Пушит в ghcr.io/<repo>:IMAGE_TAG и ghcr.io/<repo>:<SHA>

deploy:
  - SSH на VPS (SSH_HOST/USER/KEY/PORT — env-scoped или repo-fallback)
  - docker pull, docker rm, docker run в сеть NETWORK_NAME (per-env)
  - --name CONTAINER_NAME (placeholder, per-env)
  - Labels: com.docker.compose.project=COMPOSE_PROJECT, com.docker.compose.service=COMPOSE_SERVICE
  - Runtime env через --env KEY="${{ secrets.KEY }}" для role=runtime секретов

nginx:
  - По API Nginx Proxy Manager (localhost:81 на VPS, NPM_EMAIL/PASSWORD)
  - Идемпотентный upsert proxy-host для DOMAIN:
    - Если host уже существует, forward_host == CONTAINER_NAME, cert привязан, ssl_forced=true → exit 0
    - Иначе ищет/создаёт LE-сертификат, upsert proxy-host с certificate_id + ssl_forced=true + http2_support=true
```

## Плейсхолдеры

Синтаксис `@@VAR@@` (выбран чтобы не конфликтовать с GHA `${{ }}` и bash `${VAR}`).

### Universal (используются всеми language-templates)
- `WORKFLOW_NAME` — `name:` workflow в GHA
- `IMAGE_TAG` — Docker tag в `ghcr.io/<repo>:<TAG>`
- `DOMAIN` — публичный домен; NPM proxy-host
- `DEPLOY_BRANCH` — `branches: [ X ]` в `on: push`
- `NETWORK_NAME` — Docker network контейнера
- `CONTAINER_NAME` — `--name` контейнера + NPM `forward_host` (v0.25.0+: placeholder, не secret. Кнопка ↩ копирует значение в `COMPOSE_SERVICE` поскольку 99% кейсов совпадают)
- `COMPOSE_PROJECT` — label `com.docker.compose.project`
- `COMPOSE_SERVICE` — label `com.docker.compose.service`
- `ENV_NAME` (synthesised, не в meta.json) — имя deploy-инстанса; используется в `environment: <name>` на каждом job + в filename `deploy-<name>.yml`

### Language-specific (Go)
- `GO_VERSION`, `BINARY_NAME`, `ENTRY_POINT`, `APP_PORT`, `ENV_FILE_PATH`

### Synthesised (rendered helpers, не в форме)
- `BUILD_ARGS` — multi-line блок `KEY=${{ secrets.KEY }}` (12-space indent, см. B-000005-fix)
- `RUNTIME_ENV_ARGS` — `--env` flags для docker run
- `DOCKERFILE_ARGS` — `ARG NAME` lines в Dockerfile
- `DART_DEFINES` — `--dart-define=KEY=...` flags (flutter_web)

## Regression-защита

В `template_render.rs` тесты:
- `test_regression_flutter_web_deploy_yml_v04` — рендерит реальный flutter_web template, проверяет ключевые блоки (CONTAINER_NAME → `--name X` + `FORWARD_HOST=X`, no `${{ secrets.CONTAINER_NAME }}` legacy, etc).
- `test_regression_go_swanqu_server_deploy_yml` — то же для go-template.
- `test_build_args_indent_aligned_in_rendered_yaml` — рендерит template с N>1 build-секретов, проверяет что все на column 12 (catches indent drift в render_build_args или в template).
- `test_ensure_deploy_secrets_populated_prunes_orphans` — проверяет что секрет которого нет ни в repo_secrets, ни в meta_hints, удаляется из deploy_secrets.

## Что НЕ в этом флоу

- **Автоматический git push.** Мы только пишем в `local_path`. Commit/push — руками пользователя.
- **Отслеживание GHA run-status в приложении.** Результат смотрим в GitHub UI.
- **E2E с реальным VPS push.** Только regression-tests против fixture'ов.
- **Управление env-scoped секретами для secret keys которых нет в meta.json `required_secrets`.** Пользователь сам в GitHub UI или через SecretsPanel + override.

## Известные UX-моменты

- **DeployDetail не показывает откуда репо.** Заголовок "Редактирование деплоя: \<env-name\>" не даёт контекста какого репо. См. соотв. T-NNN в todo.md.
- **Deploy — отдельный экран, не таб RepoDetail.** Архитектурно это master-detail внутри одного репо; мог бы жить как RepoDetail-таб. См. соотв. T-NNN в todo.md.
