# Flow: Deploy (T-028)

**Введено в:** v0.7.0  
**Связанные файлы:** [src-tauri/src/template_render.rs](../../src-tauri/src/template_render.rs), [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs), [src/lib/components/DeployScreen.svelte](../../src/lib/components/DeployScreen.svelte), [src/lib/components/DiffDialog.svelte](../../src/lib/components/DiffDialog.svelte)  
**Исходная спека:** [docs/doc5_deploy_template_spec.md](../doc5_deploy_template_spec.md)

## Суть

Из параметризованного шаблона в SQLite (таблица `templates`) + пользовательского манифеста → генерируем **реальные файлы** в папке репо (обычно `.github/workflows/deploy.yml` + `dockerfile`), с превью diff'а и per-file подтверждением.

## Слои

```
1. Templates (SQLite)  ← bundle seed + editable в TemplatesScreen
   ├─ flutter_web/
   │  ├─ deploy.yml.tmpl     (@@PLACEHOLDERS@@)
   │  ├─ dockerfile.tmpl
   │  └─ meta.json           (display_name, placeholders, required_secrets, file_targets)

2. Manifest (SQLite, per-repo)
   deploy_manifests
   ├─ repository_id
   ├─ workflow_name, image_tag, compose_service, domain, deploy_branch

3. Render (pure Rust function)
   render_template(tmpl, vars) → String
   Missing key → Err "Missing manifest key: X"

4. Write (Rust command)
   write_deploy_files(local_path, [{path, content}, ...]) → WriteResult
```

## Трубопровод

### Шаг 1. Выбор deploy_target

На RepoDetail meta-row — dropdown с языками из `list_template_languages()`.
Пример: `deploy_target='flutter_web'`.
Если не задан — кнопка "🚀 Deploy" не показывается.

### Шаг 2. Заполнение манифеста (DeployScreen)

Форма 5 полей:
- `WORKFLOW_NAME` — `"SwanQu Support — Deploy"`
- `IMAGE_TAG` — `"prod"` (или `test`/`latest`/...)
- `COMPOSE_SERVICE` — `"swan-support-prod-frontend"`
- `DOMAIN` — `"support.swanqu.tech"`
- `DEPLOY_BRANCH` — dropdown с реальными ветками GitHub (через `octokit.repos.listBranches`)

Auto-save в SQLite при blur/input (debounced 400ms).

### Шаг 3. Check secrets

Кнопка "Check secrets" вызывает `listRepoSecrets(token, owner, repo)` → сравнивает с `required_secrets` из `meta.json`:

```
✓ API_BASE_URL          — есть на GitHub
✗ SSH_KEY (missing)     — нужно завести (multiline, добавь через SecretsPanel)
✓ SSH_HOST              — есть
...
```

Рекомендация: перед Generate files все required_secrets должны быть ✓.

### Шаг 4. Generate files

`render_deploy_files(repo_id)` на Rust:

1. Читает repo → `deploy_target`. Если None → Err.
2. Читает manifest. Если None → Err.
3. Читает `meta.json` из templates БД → парсит `file_targets`:
   ```json
   "file_targets": {
     "deploy.yml.tmpl": ".github/workflows/deploy.yml",
     "dockerfile.tmpl": "dockerfile"
   }
   ```
4. Для каждого `*.tmpl` файла, для которого есть mapping в `file_targets`:
   - `render_template(content, vars)` — подставляет `@@KEY@@`
   - Результат в виде `RenderedFile { path, content }`
5. Возвращает `Vec<RenderedFile>`.

### Шаг 5. Сравнение с существующими файлами

`read_repo_files(local_path, rel_paths)` → `Vec<Option<String>>`:
- `None` → файл отсутствует (будет NEW)
- `Some(c)` + `c == new` → UNCHANGED
- `Some(c)` + `c != new` → CHANGED

### Шаг 6. DiffDialog — предпросмотр с per-file чекбоксами

- **NEW**: `<pre>`-превью нового контента, чекбокс "Create" (on)
- **CHANGED**: side-by-side через `diff.diffLines`, чекбокс "Overwrite" (on)
- **UNCHANGED**: свёрнут, чекбокс disabled

Кнопка "Write selected".

### Шаг 7. Write

`write_deploy_files(local_path, files)` в Rust:
1. `sync::ensure_root_exists(local_path)` — guard из B-001 (не создаёт репо-папку, если её нет)
2. Для каждого `RenderedFile`:
   - `create_dir_all(parent)` — т.к. `.github/workflows/` может не существовать
   - `fs::write(target, content)`
3. Возвращает `WriteResult { written: [...], errors: [...] }` — partial failures возможны (один файл записан, другой упал на I/O)

## Что происходит в GitHub Actions после push

Эта часть вне нашего приложения, но важна для понимания куда ведёт весь флоу.

Сгенерированный `deploy.yml` триггерится на push в `DEPLOY_BRANCH`. Три job'а последовательно:

```
build-and-push:
  - Собирает Docker-образ (Flutter-web multi-stage: cirruslabs/flutter + nginx:alpine)
  - Генерирует .env из GitHub Secrets (API_BASE_URL, APP_API_KEY)
  - Пушит в ghcr.io/<repo>:IMAGE_TAG и ghcr.io/<repo>:<SHA>

deploy:
  - SSH на VPS (SSH_HOST/USER/KEY/PORT)
  - docker pull, docker rm, docker run в сеть goapp01_prod_proxy-network
  - Labels: com.docker.compose.project=goapp01_prod, com.docker.compose.service=COMPOSE_SERVICE

nginx:
  - По API Nginx Proxy Manager (localhost:81 на VPS, NPM_EMAIL/PASSWORD)
  - Идемпотентный upsert proxy-host для DOMAIN:
    - Если host уже существует, forward_host == CONTAINER_NAME_PROD, cert привязан, ssl_forced=true → exit 0
    - Иначе ищет/создаёт LE-сертификат, upsert proxy-host с certificate_id + ssl_forced=true + http2_support=true
```

## Плейсхолдеры

Синтаксис `@@VAR@@` (выбран чтобы не конфликтовать с GHA `${{ }}` и bash `${VAR}`).

### Текущий набор (flutter_web)
- `WORKFLOW_NAME` — `name:` в GHA
- `IMAGE_TAG` — docker tag (используется 3 раза в template)
- `COMPOSE_SERVICE` — label
- `DOMAIN` — NPM proxy-host
- `DEPLOY_BRANCH` — `branches: [ X ]` в `on: push`

## Regression защита

В `template_render.rs` есть тест `test_regression_swan_support_test`:
- Берёт bundled `flutter_web/deploy.yml.tmpl`
- Рендерит с манифестом боевого swan_support_test
- Сравнивает **байт-в-байт** с фикстурой `tests/fixtures/swan_support_test_deploy.yml` (снапшот боевого файла)

При любом изменении template'а этот тест ломается → напоминание сверить боевой файл и при необходимости обновить фикстуру.

## Что НЕ в этом флоу

- **Автоматический push файлов**. Мы только пишем в `local_path`. Commit/push — руками пользователя.
- **Отслеживание статуса GHA в приложении**. Результат смотрим в GitHub UI.
- **E2E автотест** с реальным push. Только regression-test против fixture.
