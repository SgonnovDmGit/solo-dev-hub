# Flow: Bug Tracking

**Обновлено в:** v0.17.0 (event log + Dashboard backend)  
**Введено в:** v0.16.0 (SQLite as SoT, 2-way MD sync)  
**Связанные файлы:** [src-tauri/src/db.rs](../../src-tauri/src/db.rs) (`bugs` table, `bug_events` table, dashboard queries), [src-tauri/src/export.rs](../../src-tauri/src/export.rs) (MD parser/generator), [src-tauri/src/sync.rs](../../src-tauri/src/sync.rs) (`reconcile_bugs_for_repo`, `migrate_bugs_for_repo`, `regenerate_bugs_md`), [src/lib/components/BugNotes.svelte](../../src/lib/components/BugNotes.svelte), [src/lib/components/BugItem.svelte](../../src/lib/components/BugItem.svelte)

## Суть

Начиная с v0.16.0, баги хранятся в **SQLite** (`bugs` table) — это **источник истины для приложения**.

`docs/bug-reports.md` в репозитории — **LLM-facing view**: двустороннесинхронизированный Markdown-файл, который LLM-агент может читать и редактировать (только поля `status` и `comment`). При следующем открытии репозитория в приложении изменения из MD ингестируются обратно в DB через `reconcile_bugs_for_repo`.

С v0.17.0 добавлена таблица `bug_events` — лог каждого перехода статуса с RFC3339-меткой времени. Инвариант: `COUNT(entered_testing events) == bugs.fix_attempts`. Используется Dashboard для метрик «попыток на закрытый баг» и category efficiency.

## Архитектура хранения

| Слой | Технология | Роль |
|------|-----------|------|
| `bugs` (SQLite, migration v18) | Rust / rusqlite | Source of truth — 12 полей: id, repository_id, numeric_id, display_id, created_at, description, severity, category, status, fix_attempts, comment, confirmed_at |
| `bug_events` (SQLite, migration v19) | Rust / rusqlite | Immutable event log — bug_id, event_type, occurred_at (RFC3339) |
| `docs/bug-reports.md` | Markdown файл в репо | LLM-facing view, 2-way sync |

> Note: `bug_stats` VIEW существовал с migration v18 и был удалён в v23 (T-000058, v0.24.0). Per-repo Stats теперь считается через direct queries в `stats_summary_for_repo` (см. flow `dashboard.md`).

## Формат MD-файла (LLM-facing view)

```markdown
- B-000001 | 2026-04-04 | description | severity | category | status | fix_attempts | comment
```

**8 полей** pipe-separated. Экранирование: `\|` для литерального `|` внутри текстовых полей, `\n` для переноса строки внутри одной записи.

**LLM может редактировать** только `status` и `comment`.  
**LLM не трогает** `description`, `severity`, `category`, `fix_attempts`, `date`, `id`.

При попытке LLM изменить protected-поля — `reconcile_bugs_for_repo` silent-restore их из DB (protected fields ignored on ingest).

## Статусная модель и event types

```
created       (app создаёт баг через UI → событие "created")
   ↓
in-progress   (LLM/dev берёт в работу → событие "taken")
   ↓
testing       (fix применён → событие "entered_testing", fix_attempts += 1)
   ↓
   ├─ confirmed  (пользователь нажимает ✓ в UI → событие "confirmed", confirmed_at = now)
   └─ rejected   (пользователь нажимает ✗ в UI → событие "rejected")
         ↓
      in-progress  (LLM/dev берёт снова → событие "reopened")
```

Допустимые переходы из MD (LLM):
| Из | В | Событие |
|----|---|---------|
| `created` | `in-progress` | `taken` |
| `created` | `testing` | `entered_testing` |
| `in-progress` | `testing` | `entered_testing` |
| `rejected` | `in-progress` | `reopened` |
| `rejected` | `testing` | `entered_testing` |

Переходы из UI (пользователь):
| Из | В | Событие |
|----|---|---------|
| `testing` | `confirmed` | `confirmed` |
| `testing` | `rejected` | `rejected` |

`valid_transition()` в `sync.rs` охраняет недопустимые переходы при reconcile.

## Cycle bug в приложении

### Создание
1. Открыть RepoDetail → вкладка Bugs
2. Нажать "+ Add bug"
3. Заполнить форму (description, severity, category)
4. Приложение: записывает строку в `bugs` (DB) → создаёт событие `created` в `bug_events` → регенерирует `docs/bug-reports.md`

### LLM-правка через MD
1. LLM читает `docs/bug-reports.md`, меняет `status` и/или `comment`
2. LLM коммитит изменения в репозиторий
3. Пользователь открывает RepoDetail в приложении → `reconcile_bugs_for_repo` вызывается при загрузке
4. Reconcile: читает MD, сравнивает с DB, ингестирует допустимые переходы + записывает события в `bug_events`, защищает protected-поля

### Подтверждение (confirmed)
1. Пользователь нажимает ✓ на баге со статусом `testing`
2. Приложение: ставит `status = 'confirmed'`, `confirmed_at = now()` в DB → записывает событие `confirmed` в `bug_events` → регенерирует MD
3. Подтверждённый баг остаётся в DB (история), выпадает из MD-view

### Отклонение (rejected)
1. Пользователь нажимает ✗ на баге со статусом `testing`
2. Приложение: ставит `status = 'rejected'` в DB → записывает событие `rejected` в `bug_events` → регенерирует MD
3. LLM может снова взять баг: `rejected` → `in-progress` → `testing` (следующая попытка, fix_attempts снова +1)

## Severity и категории

Severity: `critical` / `major` / `medium` / `minor`

Категории (9, закреплены в CHECK constraint БД):
`ui_ux` / `ux_flow` / `logic` / `auth` / `database` / `performance` / `security` / `integration` / `other`

## Связь с Dashboard

`bug_events` таблица питает Dashboard-метрики:
- **Попыток на закрытый**: `AVG(fix_attempts) WHERE bug closed in period` — через events grouped by bug_id
- **Category efficiency bars**: `entered_testing` + `confirmed` events per category per period
- **Per-day bugs chart**: `created_at` (opened) + `confirmed_at` (closed) из `bugs` таблицы

`bug_stats` VIEW был удалён в v23 (T-000058, v0.24.0) — per-repo Stats UI (StatsSummary, v0.22.0) использует direct queries на `bugs`+`bug_events` через `stats_summary_for_repo`.

## Что НЕ входит

- Баг-трекер сторонних сервисов (GitHub Issues, Jira)
- Rollback подтверждения (если confirmed по ошибке — пересоздать баг руками)
- Автоматические регрессионные тесты — пишутся разработчиком вручную
