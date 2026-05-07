# Flow: Dashboard (v0.17.0+)

**Введено в:** v0.17.0 (portfolio redesign, event-log backend)  
**Связанные файлы:** [src-tauri/src/db.rs](../../src-tauri/src/db.rs) (dashboard queries), [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs) (`read_dashboard`, `parse_done_entries_in_period_cmd`), [src/lib/stores/dashboard.ts](../../src/lib/stores/dashboard.ts), [src/lib/components/Dashboard.svelte](../../src/lib/components/Dashboard.svelte) + sub-components: `DashboardFilters`, `DashboardKpi`, `DashboardDailyChart`, `DashboardCategoryBars`, `DashboardTopHot`

## Суть

Dashboard — портфельный overview за выбранный период и по выбранным проектам. Single read — single render: один Tauri command возвращает полный `DashboardData` snapshot.

## Архитектура

1. **Frontend** (Svelte 5 runes + stores): пользователь меняет period (Неделя/Месяц/Квартал/Кастом) или project-фильтр → store `loadDashboard()` → Tauri command `read_dashboard(DashboardFilter)`
2. **Backend** (Rust): `read_dashboard` вызывает 8+ query-методов на `AppDb` (count_active_bugs, count_closed_bugs_in_period, avg_attempts_per_closed_in_period, top_hot_projects, bugs_per_day, tasks_per_day, category_efficiency, …) + парсит `docs/done.md` для tasks-chart. Возвращает полный `DashboardData` snapshot.
3. **Source of truth:** `bugs` + `bug_events` (миграции v18 + v19) для bug-метрик; `docs/done.md` parsing для tasks. `bug_stats` VIEW удалён в v23 (T-000058) — per-repo Stats теперь через `get_repo_stats_summary` direct queries (v0.22.0 T-000054).

## Period semantics

| Пресет | Начало | Конец |
|--------|--------|-------|
| Неделя | Понедельник текущей календарной недели | сегодня |
| Месяц | 1-е число текущего месяца | сегодня |
| Квартал | 1-е число первого месяца квартала (Jan/Apr/Jul/Oct) | сегодня |
| Кастом | user-picked start | user-picked end |

### Compare-to-previous (partial same-length)

Для пресетов: `d = period_end − period_start` (в днях). Prev_start = начало прошлого пресет-периода (предыдущий Пн / 1-е прошлого месяца / 1-е прошлого квартала; Q1 → Q4 прошлого года). Prev_end = prev_start + d, clamped к концу прошлого периода если выходит за границы (например, март сравнивается с первыми N днями февраля, не с «мартом прошлого месяца»).

Для **Custom** — compare скрыт (нет семантики «предыдущего кастомного периода»).  
При `d < 1` (утро понедельника с пресетом Неделя) — compare тоже скрыт.

## KPI tiles (5 штук)

| # | Метрика | Формула | Compare |
|---|---------|---------|---------|
| 1 | Активных багов | `COUNT WHERE status != 'confirmed'` | Нет (lifetime) |
| 2 | Закрыто за период | `COUNT WHERE confirmed_at ∈ period` | Да, delta |
| 3 | Задач выполнено | parse `## YYYY-MM-DD` sections в `docs/done.md` filtered repos | Да, delta |
| 4 | % решения | `closed / (closed + opened) × 100`. При 0/0 → "—" | Да, delta |
| 5 | Попыток на закрытый | `AVG(fix_attempts) WHERE bug closed in period` (via bug_events). При 0 → "—" | Да, invert-delta (меньше = лучше) |

Под KPI-1 — under-number "из них N critical".  
Под KPI-4 — `{closed} закрыто · {opened} открыто`.

## Top-3 hot projects

Показывается только при `project_filter.length > 1` или «все проекты». Tuple-sort `(active_critical, active_major, active_total)` по убыванию. `INNER JOIN + HAVING active > 0` исключает проекты с нулём активных багов.

## Per-day charts

**Bugs chart:** для каждого дня в периоде [start, end] два столбика:
- Красный: `COUNT(bugs WHERE date(created_at) = day)` — открытые
- Зелёный: `COUNT(bugs WHERE date(confirmed_at) = day AND status = 'confirmed')` — закрытые

Будущие дни (от today+1 до end) — пунктирные placeholders (нет данных, но визуализируются как ожидаемый диапазон).

**Tasks chart:** для каждого дня — `COUNT(done-entries WHERE date = day)` фиолетовый. Данные из парсинга `docs/done.md`. Асимметрия: дата открытия задачи недоступна (todo.md не хранит её) → tasks opened-chart закроется в v0.21.0 formats review.

## Category bars

9 категорий из CHECK constraint БД: `ui_ux`, `ux_flow`, `logic`, `auth`, `database`, `performance`, `security`, `integration`, `other`.

Для каждой категории:
- `touched_in_period`: `created_at ∈ period OR confirmed_at ∈ period`
- `closed_in_period`: `confirmed_at ∈ period`
- `attempts_in_period`: count `entered_testing` events в период
- `resolution_rate = closed / touched`

Sort по `resolution_rate` DESC. Цвет бара: ≥75% зелёный, 35–74% оранжевый, <35% красный. Если `touched = 0` — строка скрыта.

## Project filter

Multi-select dropdown. `project_ids: null` или empty array = ALL repos. Deselect-all ведёт себя как «выбрать все» — нет пустого состояния, предсказуемое поведение.

## Performance

При ~10 проектах × ~1000 багов ожидается < 200 ms full dashboard load. Индексы в migration v19: `idx_bugs_confirmed_at` (partial, WHERE confirmed_at IS NOT NULL), `idx_bug_events_bug_id`, `idx_bug_events_ts`, `idx_bug_events_type_ts`.

## Связанные миграции

| Миграция | Версия | Содержимое |
|----------|--------|-----------|
| v18 | v0.16.0 | `bugs` table (12 полей) + `bug_stats` VIEW (VIEW dropped in v23) |
| v19 | v0.17.0 | `bug_events` table + indexes + backfill synthetic events для pre-v19 bugs |
| v23 | v0.24.0 | `DROP VIEW bug_stats` (dead code после v0.22.0 T-000054 — Stats UI переехал на direct queries) |
