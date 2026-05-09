# Flow: Auto-generated repo files (project.md + CLAUDE.md section + .gitignore)

**Введено в:** v0.10.0 (F-010, F-014, auto-gitignore)
**Связанные файлы:** [src-tauri/src/sync.rs](../../src-tauri/src/sync.rs) (`generate_project_md`, `update_claude_md_section`, `sync_gitignore_section`, `copy_doc_skeleton_if_missing`), [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs) (`sync_project` pre-phase, `sync_global_claude_md`, `init_docs_for_repo`)

## Суть

При каждом `sync_project` приложение пишет пять видов файлов в каждый репозиторий проекта:

| Файл | Где | Семантика | Перезаписывается? |
|------|-----|-----------|:-----------------:|
| `docs/project.md` | в `docs/` каждого репо | Карта проекта: тип, репо, роли, микросервисы, родители | Да, каждый sync |
| CLAUDE.md section | корень репо (`CLAUDE.md`) | AI-контекст проекта (между маркерами `<!-- manager:begin/end -->`) | Да, секция между маркерами |
| `.gitignore` | корень репо | Блок `# --- solo-dev-hub:begin … :end ---` с **только отсутствующими** у user правилами (dedup exact-match по template rule-строкам) | Да, блок пересобирается при sync. Если все template-правила у user уже есть — блок не создаётся |
| `docs/todo.md` | в `docs/` каждого репо | Skeleton задач (формат + пример) | Нет — только если файла нет (F-016, 0.11.0) |
| `docs/bug-reports.md` | в `docs/` каждого репо | Skeleton баг-трекера (флоу статусов + пустой список) | Нет — только если файла нет (F-016, 0.11.0) |

## Manual trigger: кнопка "📚 Обновить документацию репозитория" в RepoDetail

В дополнение к auto pre-phase при sync'е, user может запустить обновление документации одного репо через команду `init_docs_for_repo(repo_id)` и кнопку в RepoDetail. **Scope команды**: всё что приложение пишет в репо.

| Файл | Поведение в `init_docs_for_repo` |
|------|----------------------------------|
| `docs/todo.md` | copy-if-missing (skeleton) |
| `docs/bug-reports.md` | copy-if-missing (skeleton) |
| `.gitignore` | section sync (block пересобирается) |
| `docs/project.md` | full regen (только если репо привязан к проекту) |
| `CLAUDE.md` | section sync между маркерами (только если репо привязан к проекту) |

Для orphan-репо без `project_id` regen `project.md` / `CLAUDE.md` пропускается — project-context отрендерить не из чего. Скелеты (`todo.md` / `bug-reports.md` / `.gitignore`) пишутся независимо от project_id.

Кнопка зеркалит pre-phase из `sync_project` для конкретного репо — после выполнения файлы в репо в том же состоянии как после Sync на проекте. Имя кнопки до v0.25.0 — "Инициализировать документацию"; переименована потому что семантика "init" подразумевала one-time, тогда как кнопка идемпотентна и в т.ч. перезаписывает app-owned файлы.

## Когда срабатывает

Pre-phase в `sync_project` — выполняется **перед** REQ/api/handlers sync'ом:

1. Для каждого репо проекта (all_repos с local_path + ensure_root_exists ok)
2. Для каждого server-репо подключённых microservice-проектов (описывает ms-проект, не parent)

## project.md (F-010)

Inline-шаблон в `generate_project_md`. Содержит:
- Заголовок (имя проекта) + описание + тип (Стандартный / Микросервис)
- Таблица репозиториев: имя, роль, путь, GitHub-флаг
- Список подключённых микросервисов + их server-репо
- Список родительских проектов (для microservice-типа)
- Футер "не редактировать вручную"

**App-owned**: полностью перезаписывается при каждом sync. Если user правит файл — правки теряются.

## CLAUDE.md section (F-014)

Шаблон с плейсхолдерами (`claude.md.section.tmpl` из `_global` templates в SQLite). User может редактировать шаблон в AppDefaultsScreen.

Маркеры: `<!-- manager:begin -->` … `<!-- manager:end -->`

### Поведение при sync

| Состояние файла | Действие |
|-----------------|----------|
| Нет CLAUDE.md | Создаём файл с секцией |
| Есть, нет маркеров | Дописываем секцию в конец |
| Есть, оба маркера | Заменяем содержимое между ними |
| Orphan-маркер (один без пары) | **Err** — пользователь чинит вручную |

Пользовательский контент ВНЕ маркеров никогда не трогается.

### Global sync (кнопка в AppDefaultsScreen)

`sync_global_claude_md` → `~/.claude/CLAUDE.md` — тот же `update_claude_md_section` с `project_id=None`. Project-placeholders заполняются "—" / "_неприменимо_".

### Почему CLAUDE.md в .gitignore

CLAUDE.md — per-user per-machine AI-инструкции. В git не комитится (по global rules). Каждый разработчик на своей машине получает свой через sync Manager'а.

## .gitignore (auto-gitignore)

Шаблон: `.gitignore.tmpl` из `_global` templates. User может редактировать в AppDefaultsScreen.

### Семантика copy-if-missing

- Если `.gitignore` **существует** (любого размера, включая 0 байт) → skip
- Если `.gitignore` **не существует** → копируем шаблон
- Если шаблон пустой (только whitespace) → no-op

После первого копирования файл принадлежит пользователю. Приложение больше не трогает.

## Шаблоны в templates table

Оба шаблона (`_global/.gitignore.tmpl` и `_global/claude.md.section.tmpl`) хранятся в таблице `templates` с `language_key="_global"`. Наследуют всю инфраструктуру 0.6.0:
- Bundled-seed при первом запуске через `include_dir!`
- Auto-refresh не-кастомных при обновлении приложения
- `is_custom=1` сохраняется при upgrade
- Reset-to-bundle через существующий `reset_template_file`

UI-редактирование — `AppDefaultsScreen` (Settings → "Шаблоны приложения"). TemplatesScreen фильтрует `_global` из списка языков.

## Тесты

15 тестов в [sync.rs](../../src-tauri/src/sync.rs):
- `test_generate_project_md_*` (3 теста: standard, microservice, fallback placeholders)
- `test_update_claude_section_*` (8 тестов: create, append, replace, preserve, multiple markers, orphan begin/end, global rendering)
- `test_copy_gitignore_*` (4 теста: missing, existing, empty existing, empty template)
