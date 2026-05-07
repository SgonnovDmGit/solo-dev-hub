# Flow: Templates System

**Введено в:** v0.6.0 (обновлено в v0.7.0 — auto-migrate)
**Связанные файлы:** [src-tauri/src/template_seeder.rs](../../src-tauri/src/template_seeder.rs), [src-tauri/src/db.rs](../../src-tauri/src/db.rs) (методы `list_template_*`, `upsert_template_file`), [src/lib/components/TemplatesScreen.svelte](../../src/lib/components/TemplatesScreen.svelte)

## Суть

Шаблоны файлов (CI/CD workflows, dockerfile, meta и т.п.) лежат **в SQLite**, но source of truth — **bundle** в репозитории приложения (`src-tauri/templates/<lang>/`). Пользователь может редактировать их в отдельном экране (TemplatesScreen), при этом при апдейте приложения:
- Не-редактированные (`is_custom=0`) шаблоны **автоматически синхронизируются** с новой bundle-версией
- Редактированные пользователем (`is_custom=1`) **сохраняются** как есть

## Зачем такая архитектура

Нужно было:
- Шаблоны **доступны из UI** (редактирование, reset)
- Не терять пользовательские правки при апдейте
- Бесплатно получать улучшения bundled-шаблонов
- Добавлять новые языки через простой git-коммит

## Слои

```
1. Bundle (compiled-in, read-only)
   src-tauri/templates/
   ├─ flutter_web/
   │  ├─ deploy.yml.tmpl
   │  ├─ dockerfile.tmpl
   │  └─ meta.json
   └─ <future languages>/

2. DB (runtime, editable)
   templates table:
   ├─ id
   ├─ language_key  ('flutter_web', 'go_backend', ...)
   ├─ file_name     ('deploy.yml.tmpl', 'meta.json', ...)
   ├─ content       (текст файла)
   ├─ is_custom     (0 = bundle seed, 1 = user-edited)
   └─ updated_at
   UNIQUE(language_key, file_name)

3. UI (TemplatesScreen)
   - Список языков слева
   - Список файлов языка в центре
   - Текстовый редактор справа (textarea + JSON validation для meta.json)
   - Save → устанавливает is_custom=1
   - Reset to default → копирует из bundle, ставит is_custom=0
```

## Auto-migrate логика (seeder)

`seed_bundled_templates(&db)` вызывается при старте приложения в `run()`.

Для каждого `(lang, file)` в `TEMPLATES_DIR`:

| DB state | Bundle ≠ DB content | Действие |
|----------|---------------------|----------|
| нет записи | — | INSERT с `is_custom=0` |
| `is_custom=1` | любое | не трогаем |
| `is_custom=0` | одинаково | skip |
| `is_custom=0` | различается | UPDATE to bundle (`is_custom=0`) |

**Инвариант**: "все `is_custom=0` записи байт-равны bundle". Это даёт бесплатную миграцию при обновлении приложения.

**Пример**: в 0.6.0 бандл содержал `meta.json` без `file_targets`. В 0.7.0 добавили. Пользователь, который не редактировал meta.json (`is_custom=0`), при следующем запуске 0.7.0 получит обновлённый файл с `file_targets` автоматически.

## Формат meta.json

```json
{
  "display_name": "flutter_web",           // показывается в UI (обычно == language_key)
  "description": "Flutter Web deploy …",
  "placeholders": {
    "WORKFLOW_NAME": "Имя workflow",
    "IMAGE_TAG": "Docker tag",
    ...
  },
  "required_secrets": [
    { "name": "API_BASE_URL", "multiline": false, "description": "..." },
    { "name": "SSH_KEY", "multiline": true, "description": "..." },
    ...
  ],
  "file_targets": {
    "deploy.yml.tmpl": ".github/workflows/deploy.yml",
    "dockerfile.tmpl": "dockerfile"
  },
  "version": 2
}
```

- **`placeholders`** — метаданные для UI (какие переменные ждёт шаблон). Не обязательно для рендера (рендер просто возьмёт ключи из манифеста, недостающие → Err).
- **`required_secrets`** — список для "Check secrets" в Deploy. `multiline=true` — подсказка UI (SecretsPanel).
- **`file_targets`** — где в репо писать каждый шаблонный файл при Generate files. `render_deploy_files` пропускает файлы без mapping (например, meta.json — не деплоит).
- **`version`** — на будущее, для ручных миграций (пока не используется).

## Добавление нового языка

1. Создать папку `src-tauri/templates/<new_lang>/`
2. Положить туда `*.tmpl` файлы с плейсхолдерами `@@VAR@@`
3. Сделать `meta.json` со списком required_secrets, placeholders, file_targets
4. Пересобрать приложение — `include_dir!` встроит файлы
5. При следующем запуске seeder обнаружит новый language и вставит в БД
6. Пользователь выбирает его в RepoDetail → dropdown deploy_target

Ничего не нужно менять в коде — только добавить bundled-файлы.

## Редактирование в UI (TemplatesScreen)

- Отдельный экран (не внутри Settings), запускается через "Открыть редактор шаблонов" в Settings
- Sidebar слева со списком языков + поиск (масштабируется для множества языков)
- Центр: список файлов выбранного языка
- Справа: textarea с содержимым файла, JSON validation для meta.json
- Save → `is_custom=1`
- Reset to default → перезаписывает из bundle, `is_custom=0`

## Рендер шаблонов

`template_render::render_template(tmpl, vars)` — чистая функция:
- Regex `@@(\w+)@@`
- Missing key → `Err("Missing manifest key: X")`
- Extra keys в vars — игнорируются
- Идемпотентно

Использование в Deploy — см. [deploy-flow.md](./deploy-flow.md).

## Тесты

- `test_seed_inserts_flutter_web` — первичный seed
- `test_seed_preserves_custom_files` — is_custom=1 не трогается
- `test_seed_updates_non_custom_on_bundle_change` — is_custom=0 + diff → update
- `test_bundled_file_content_flutter_web` — bundle читается, meta.json содержит file_targets
- `test_regression_swan_support_test` (в template_render) — rendered bundle + swan манифест байт-в-байт равен боевому deploy.yml

## Что НЕ входит

- Добавление нового языка через UI. Только через bundle + git commit + rebuild.
- Syntax highlighting в редакторе.
- Валидация что все `@@VAR@@` в template'е описаны в `placeholders`. (Валидация только at-render-time: missing key → Err.)
- Rollback к предыдущей версии custom-шаблона. Только текущий state.
