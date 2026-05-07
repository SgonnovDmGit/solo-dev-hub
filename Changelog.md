# Changelog

Формат: [Keep a Changelog](https://keepachangelog.com/)

## [Unreleased]

## [0.24.2] — 2026-05-07

Diagnostics + ms reverse-lookup patch.

### Added
- **B-000018 | Microservice → reverse-lookup на parent серверы** (T-000070):
  при открытии ms-проекта `list_project_requirements` теперь дополнительно
  собирает требования и api/handlers со стороны connected parent-серверов.
  Из микросервиса видны все требования к нему. Новый флаг
  `RequirementInfo.is_reverse_lookup` отличает строки со стороны ms — UI
  скрывает ✓-кнопку (confirm должен делать sender = parent server из
  своего SyncScreen) и показывает hint ↩.
- `docs/known-issues/B-000017-flicker-multi-monitor.md` — детальный разбор
  расследования B-000017: repro, root cause (WebView2 mixed-DPI multi-monitor
  bug в Chromium DPI pipeline), 3 attempted fixes с причинами почему не
  сработали, applied side-fixes, workaround, re-evaluation triggers.

### Changed
- **B-000017 | SvelteKit preload-on-hover отключён** (T-000069 side-fix):
  `data-sveltekit-preload-data="off"` + `data-sveltekit-preload-code="off"`
  в [src/app.html](src/app.html). Tauri SPA с store-based screen switching
  не использует SvelteKit nav — handlers были pure dead overhead, генерируя
  noise в Performance trace на каждый pointermove (20ms-таймер install/remove).
- `initUiScale` defensive cleanup: стрипает inline `style.zoom` на
  `documentElement` при init на случай leftover от dev-эксперимента (CSS
  zoom стэкуется поверх WebView setZoom, ломая viewport).

### Fixed
- **T-000072 | Settings PAT card "Удалить токен" overflow**: при zoom ≥125%
  кнопка вылезала за правый край карточки. `flex-wrap: wrap` на `.pat-row-2`
  + `min-width: 0` на `.pat-row-2-left` — кнопка мягко падает на следующую
  строку при дефиците места.

### Known limitations
- **B-000017 | Subpixel flicker на secondary мониторе при zoom ≥125%**
  (T-000069): Chromium/WebView2 mixed-DPI multi-monitor bug. Не решается на
  нашей стороне (CSS layer promotion не помог; CSS zoom вместо setZoom
  ломает viewport math). **Workaround:** на проблемном мониторе Settings →
  Внешний вид → Масштаб → manual <125% (100/110%) или ≥150%. Полный разбор
  и re-evaluation triggers в `docs/known-issues/B-000017-flicker-multi-monitor.md`.

## [0.24.1] — 2026-05-07

UX patch — i18n cleanup в Dashboard/Timeline и SyncScreen polish.

### Added
- 22 новых i18n ключа × ru/en для timeline kinds + Dashboard hardcoded
  строк: `timeline.kind.{bug_event,task_event,repo_rename,sync_event,deploy_event}`,
  `dashboard.{deltaToPrev,bugsAbbrev,attemptsAbbrev,outOfFmt,dow.0..6}`,
  `common.{selectAll,clearAll}`. Раньше эти места были hardcoded на ru.
- Locale param в `formatRelativeTime(iso, nowMs?, locale?)` — раньше был
  ru-only. Default читает текущий `$locale` через `get(locale)`, явный
  param используется в reactive context (Dashboard activity feed).
- +1 vitest case для en-locale formatRelativeTime (40 frontend тестов).

### Changed
- **B-000015 | Dashboard + Timeline i18n cleanup** (T-000066): 7 hardcoded
  ru-мест переведены на `$tStore`. Дополнительно (попытка 2 после dogfood):
  допилены KPI hint'ы где английские термины (`confirmed`, `fix_attempts`,
  `closed/created`, `critical`) оставались внутри ru-фраз — заменены на
  русскую терминологию ("закрытые", "до закрытия", "критичных").
- **Семантика timeline + Dashboard activity events**: `bug.confirmed/rejected`
  с 'подтверждён/отклонён' (про bug) на 'решение принято/отклонено' (про
  fix). EN side: `'fix accepted/rejected'`. Точнее отражает workflow.
- **B-000019 | SyncScreen api.md/handlers.md dedupe** (T-000068): direction
  `server_to_client` создавал отдельный `RequirementInfo` на каждого клиента
  для shared файлов (api.md, handlers.md). 5 клиентов → 5 строк "api.md".
  Frontend теперь агрегирует by `(filename, status)` внутри каждой
  source-group: одинаково-статусные клиенты схлопываются в одну строку
  с counter `×N` и list'ом targets через ", ". Mixed статусы (3 sent / 2
  new) остаются отдельными строками.
- **B-000020 | SyncScreen объединение Microservice → Server** (T-000071):
  направления `microservice_to_server_api` и `microservice_to_server_handlers`
  слиты в одну секцию "Микросервис → Сервер" (зеркальная "Сервер → Клиент"
  после B-000019). `aggregateServerToClient` переименована в
  `aggregateByFilename` — generic helper переиспользуется для обоих
  shared-file directions. Удалены старые i18n ключи
  `sync.microserviceToServer{Api,Handlers}`, добавлен унифицированный
  `sync.microserviceToServer`.
- `DashboardActivityFeed` мигрирован с inline-relative-time (дублировал
  логику из `time-format.ts` плюс `$locale` switch'и) на общий
  `formatRelativeTime` + `nowTick` + `$locale` параметр.

### Fixed
- **B-000016 | SyncScreen scroll-jump после confirm** (T-000067):
  `handleConfirm` вызывал `loadRequirements()` который пересоздавал весь
  `requirements` массив → keyed `{#each}` ремонтировал весь DOM →
  scrollTop сбрасывался в 0. Теперь после успешного `confirmRequirement`
  локально фильтруем массив (Svelte точечно убирает только confirmed-row,
  surrounding nodes стабильны). Backend `confirm_requirement` физически
  удаляет file pair, так что filter — корректное отражение нового
  состояния без round-trip.

### Tests
- 40/40 vitest passing (39 → 40 за en-locale случай)
- 290/290 cargo passing (без новых backend тестов — фиксы только frontend
  + i18n)
- svelte-check 444 files, 0 errors / 0 warnings

## [0.24.0] — 2026-05-04

### Added
- **F-000036 | Templates UX rework** — переразбивка Settings на 3-bucket модель:
  - Card "Шаблоны репозиториев" с inline-кнопками `📋 Стартовые файлы`
    (`AppDefaultsScreen` для `_global` файлов: `.gitignore`, project
    CLAUDE.md section, `docs/todo.md` / `bug-reports.md` заглушки) и
    `📤 Шаблоны деплоя` (`TemplatesScreen` per-language).
  - Новая card "Глобальные правила AI" с кнопками `📝 Открыть шаблон`
    (открывает GlobalClaudeEditor напрямую) + `⟳ Синхронизировать`
    (уровень Settings — раньше Sync был спрятан внутри editor'а), плюс
    inline-status `Последняя: <время>` через relative-time formatter.
  - Новый компонент `GlobalClaudeEditor.svelte` — dedicated editor для
    `claude.md.global.tmpl` (single-file, без list view), с Sync-кнопкой
    в header'е (disabled когда editor dirty + tooltip "Сначала
    сохраните") + last-sync timestamp.
  - Sync timestamp persist'ится в settings table key `ai_rules_last_sync_at`
    (RFC3339), обновляется только при успехе синхронизации.
- `formatRelativeTime(iso, nowMs?)` helper в `src/lib/utils/time-format.ts`
  с порогами `<1мин` → "только что", `<60мин` → "{N} мин назад", `<24ч` →
  "{N} ч назад", `≥24ч` → "{N} дн назад". Плюс `nowTick` readable store
  с 60-секундным интервалом — обеспечивает auto-refresh relative-time
  displays без user interaction. Russian-only formatting (i18n by locale —
  следующая итерация).
- 9 новых i18n ключей × ru/en для cards 4-5 + AI rules editor (`templatesRepoCard`,
  `aiRulesCard`, `bucketRepoInit`, `bucketDeploy`, `aiRulesOpenTemplate`,
  `aiRulesSync`, `aiRulesSyncTooltipDirty`, `aiRulesLastSync`, `aiRulesNeverSynced`).

### Changed
- **Routing refactor** (D12): `currentScreen` Svelte store мигрирован
  с `writable<Screen>` (string union) на `writable<ScreenState>`
  (`{ name: ScreenName; params?: Record<string, unknown> }`). Все ~21 caller
  `.set('xxx')` обновлены на `.set({ name: 'xxx' })`; все ~18 reactive read
  `$currentScreen === 'xxx'` обновлены на `$currentScreen.name === 'xxx'`.
  Atomic single-commit, `npm run check` clean. Future-proofs routing для
  routes с params; немедленно unblock'ает route `'global_claude_editor'`.
  Helper `navigateTo(screen)` принимает string-or-ScreenState (backward-compat).
- **AppDefaultsScreen** превращена в bucket-B-only: всегда исключает
  `claude.md.global.tmpl` из displayed списка `_global` файлов через
  `excludeFiles` prop на `TemplateEditor`. Footer Sync-кнопка удалена
  целиком (~38 строк): button + ConfirmDialog block + `handleSyncGlobal`
  handler + 3 imports + 3 CSS rules. Sync переехал на Settings card
  и в editor header.
- `sync_global_claude_md` Tauri command: теперь возвращает
  `SyncGlobalClaudeResult { path, synced_at }` (раньше plain string),
  пишет setting `ai_rules_last_sync_at = NOW()` после успешного
  `sync::update_claude_md_global`. snake_case на Tauri-границе
  (project convention; в TS — `result.synced_at`).
- Settings card 4 переименована "Шаблоны" → "Шаблоны репозиториев",
  layout с двух row-label rows на single row с двумя inline кнопками.

### Removed
- **T-000058 | Migration v23** — `DROP VIEW IF EXISTS bug_stats`. Legacy
  VIEW не использовался с v0.22.0 (T-000054 stats redesign перевёл per-repo
  StatsTable на новые queries `get_repo_stats_summary` / `get_project_stats_summary`;
  Dashboard перешёл на свои queries в v0.17.0). 2 минорных версии dead schema
  → finally clean. +1 cargo test `test_db_migration_v23_drops_bug_stats_view`,
  4 existing test'а обновлены под v23, 1 obsolete `test_bug_stats_view_from_bugs`
  удалён. Сопутствующая чистка stale references в `README.md`,
  `docs/flows/dashboard.md`, `docs/flows/bug-tracking.md`,
  `docs/flows/repository-deletion.md` (последняя получила ещё и обновлённую
  cascade-list — обнаружено что doc отстал на 9 cascades vs 3 mentioned).
- Orphan i18n keys `settings.templatesCard` / `templatesRepoLabel` /
  `templatesGlobalLabel` / `templatesOpenEditor` (заменены новыми
  card-specific ключами в Task 4).
- Unused `previousScreen` import в `src/routes/+page.svelte`.

### Fixed
- TemplateEditor `excludeFiles` prop теперь reactive (`$effect` правильно
  отслеживает изменения post-mount через явный `void excludeFiles`
  inside body). Раньше — read-once-at-mount.
- GlobalClaudeEditor `handleSave` теперь вызывает `loadContent()` после
  successful `saveTemplateFile` — авторитарный refresh из БД вместо
  только-optimistic local update; mirrors `TemplateEditor.handleSave`
  pattern, защита от latent staleness если is_custom flag когда-нибудь
  surface'нется в этом UI.
- Settings cards 4-5 grid layout: `.row-control` без sibling `.row-label`
  раньше landил в 130px label column из-за grid auto-placement, кнопки
  жались. Inline `grid-column: 1 / -1;` восстанавливает full-width.

### Tests
- +1 vitest test file (5 cases) для `formatRelativeTime` thresholds.
- +3 cargo тестa: `test_sync_global_claude_md_sets_last_sync_at`,
  `test_sync_global_claude_md_does_not_set_on_failure` (Task 5),
  `test_db_migration_v23_drops_bug_stats_view` (T-000058).
- Cargo suite: 290 tests passing (288 baseline + 3 new − 1 obsolete = 290).

## [0.23.0] — 2026-05-04

### Added
- **Global CLAUDE.md template** обогащён двумя новыми H1-секциями:
  **"Phase work workflow"** (триггер trivial vs non-trivial с тремя axis:
  ≥2 non-obvious decisions / cross-boundary / ≥3 sub-tasks; mechanical
  mass-changes carve-out; user-override clause; 5 шагов chat → spec →
  self-review → OK → impl; project-bindings заглушка) и
  **"Manual-smoke verification in every spec"** (required content для
  `## Verification` секции в любом spec'е). Project-addendum в нашем
  CLAUDE.md (`docs/superpowers/specs/<YYYY-MM-DD>-<phase-name>.md`,
  subagent-driven implementation).
- **B-000009 / B-000012** Pre-release dogfood patch (см. Fixed ниже).
- **UI scale auto + manual** (B-000009) — Settings → Внешний вид → "Масштаб":
  Auto (default) подбирает zoom по effective logical width текущего монитора
  (`monitor.size.width / scaleFactor`), пересчёт по `window.onMoved` с
  300ms-debounce при перетаскивании окна между мониторами; пресеты 80% / 90% /
  100% / 110% / 125% / 150% для manual override. Эвристика: ≥3500px → 1.5×,
  ≥2500px → 1.25×, ≥1900px → 1.1×, иначе 1.0×. Apply через нативный
  `getCurrentWebview().setZoom(scale)` (Tauri v2 webview API), требует
  permission `core:webview:allow-set-webview-zoom`. Persist в `settings`-таблице
  ключами `ui_scale_mode` (`auto` / `manual`) и `ui_scale_manual` (число).
  Новый модуль `src/lib/ui-scale.ts` инкапсулирует stores, эвристику, init
  с `onMoved` listener; настройки в `src/lib/stores/settings.ts` через
  `saveUiScaleMode` / `saveUiScaleManual`. +2 i18n ключа (`uiScaleLabel`,
  `uiScaleAuto`) × ru/en. Round 2: разделение `uiScaleApplied`
  (что применено) и `uiScaleAutoComputed` (что насчитал бы auto для текущего
  монитора независимо от mode) — dropdown-лейбл "Авто (NN%)" теперь читает
  второй, поэтому в Manual-режиме показывает истинное auto-значение, а не
  выбранный manual. `onMoved` пересчитывает оба значения; applyZoom в manual
  — no-op (тот же manual), autoComputed обновляется для UI-консистентности
  при переезде окна между мониторами.
- **T-000057** About window redesign: hero становится 2-кол (logo + tagline + GitHub-link),
  donate prominent сразу после hero (featured pink-gradient styling), новая секция
  "Что умеет" с 6 features в 2-кол grid (полные русифицированные формулировки),
  update переезжает вниз как minor utility, devs-row становится compact one-liner
  («Автор: Сгоннов Д.А. · ИИ-ассистент: Claude (Anthropic) · Лицензия: MIT»).
- **Адаптивный layout About** — content стречится на всю ширину окна с adaptive
  padding `clamp(32px, 4%, 80px)`; hero (logo + название) и features-grid scale'ятся
  через `clamp(180px, 12vw, 280px)` логотип, `clamp(26px, 2vw, 36px)` название;
  features-grid 2-кол (≤1100px) → 3-кол (≥1100px) → 1-кол (≤720px); donate-rows
  выровнены через CSS grid + pixel-precise text alignment между ссылкой Boosty
  и адресом TON-кошелька.
- **Vertical centering** в About через auto-margin trick на `:first-child` /
  `:last-child` (центрируется когда контент влезает, нормально скроллится без
  clipping когда не влезает).

### Changed
- About-окно: GitHub из button в text-link; logo 256px → responsive
  `clamp(180px, 12vw, 280px)`; devs-card vertical → one-liner.
- Update error state в About-карточке: убраны дублирующие кнопки «Попробовать
  ещё раз» и «Скрыть» под текстом ошибки — `↻ Проверить` в шапке карточки уже
  выполняет ту же функцию.

### Removed
- i18n keys `about.developers`, `about.developersValue`, `about.githubRepo` —
  заменены новой структурой `about.devs.*` и `about.tagline`.
- Unused `dismissUpdateStatus` import из About.svelte (после упрощения error state).

### Fixed
- **B-000012** Global CLAUDE.md template (`src-tauri/templates/_global/claude.md.global.tmpl`)
  получил H1-секцию **"Feature flow docs (`docs/flows/`)"** с ключевым правилом
  disambiguation current-vs-planned: present tense описывает текущее поведение
  в HEAD, planned помечается явно (inline `(planned)` tag, `> 🚧 Not implemented yet`
  блок, или отдельная секция `## Planned changes`); past tense запрещён в флоу-
  доках (это changelog-материал). Также update-policy (флоу обновляется в том
  же коммите что код, не follow-up'ом; (planned)-маркер снимается при имплементации;
  ссылки на файлы должны указывать на live код), границы с Changelog.md /
  docs/todo.md / REQ-pairs / design-memo папками. Бонусом (отдельно от B-000012)
  добавлены H1 **"Release closure checklist"** (9-step чеклист со шкалой условных
  шагов: REQ receipts только если applicable, schema regen только для серверных
  с БД) и H1 **"Commit messages"** (Conventional Commits format с trailers
  `Refs T-NNNNNN` / `REF: REQ-NNN` / `B-NNNNNN`). Параграф "Release workflow:" в
  Versioning section заменён на forward-link к новому чеклисту.
- **B-000014** При maximize окна с custom-titlebar (`decorations: false`) на Win11
  Windows extend'ил окно ~8px за каждый край screen (для invisible resize-border)
  → content с `height: 100vh` уходил на 8px ниже visible area, scrollbar и нижняя
  граница "уходили за экран". В `+page.svelte` к `.app` div добавлен
  `class:maximized={isMaximized}` (state уже был для maximize/restore icon swap).
  CSS-правило `.app.maximized { box-sizing: border-box; padding: 0 8px 8px 8px }`
  компенсирует overhang только в maximize-state. Round 2: дополнительно
  фикс горизонтального scroll'а — `.comment-btn` в `BugItem.svelte` не имел
  `white-space: pre-wrap; word-break: break-word` (только `.text-btn` для
  description имел). Длинные comment'ы вылезали за пределы content area,
  заставляя `<main>` показывать горизонтальный скролл. Wrapping-CSS теперь
  одинаков для description и comment.
- **B-000013** Stats-табы (RepoDetail / ProjectDetail) не считали `rejected`-баги
  активными в KPI «Активные» / «Критических активных». В `stats_summary_for_repo`
  и `stats_summary_for_project` whitelist `status IN ('created','in-progress',
  'testing')` заменён на `status != 'confirmed'` (как в `count_active_bugs`,
  который Dashboard использует — там было правильно изначально). Логика: rejected
  — НЕ закрытое состояние, юзер не принял фикс и баг ушёл обратно в работу.
  4 query изменено в `db.rs`. +1 unit test (`test_stats_summary_includes_rejected_in_active`,
  288 total). Round 2: подсказка `stats.summary.kpiActiveHint` обновлена с
  устаревшего "open + in-progress + testing" на локализованное "все кроме
  закрытых" / "all except closed" — не перечисляю статусы, гибче и не сломается
  при добавлении новых workflow-состояний.
- **B-000004** Tasks tab дублировал строки при рерайте ID в todo.md (T-034 и
  T-000034 одновременно, или плейсхолдер F-NNN рядом с реальным F-000035).
  `sync_tasks_for_repo` теперь чистит orphan todo-строки: любая DB-row с
  source='todo', чьего task_id нет в актуальном todo.md, удаляется (task_events
  каскадятся через FK). После нормализации todo.md и Refresh дубликат
  автоматически исчезает. Done-rows append-only и не трогаются. +1 unit test
  (`test_sync_tasks_cleans_up_orphan_todo_rows`).
- **B-000011** Tasks/Сделано табы залипали с данными прошлого репо при
  переключении (RepoDetail не пере-маунтит вкладку, лишь обновляет prop —
  `onMount` срабатывал один раз). `TasksTab` и `DoneTab` теперь перечитывают
  данные через `$effect(() => { void repoId; load(); })` — паттерн уже
  использовался в `RepoChangelogTab`. Также `<DataGrid>` внутри обоих обёрнут
  в `{#key repoId}` чтобы persisted sort/filters не утекали между репо при
  смене `persistKey`. Остальные табы проверены — все реактивны корректно.
- **B-000010** Иконка приложения в таскбаре больше не мутная и показывает
  SDH-кроп. Корень проблемы: Tauri/tao грузит ОДНУ icon-frame для running-window
  (по дефолту 256×256 = полный логотип), и при 200% DPI Windows downscale'ил
  256→64 → blur + не SDH дизайн. Решение в `lib.rs` — `tauri::Builder::setup`
  callback явно ставит window icon из `64x64.png` (SDH-кроп, exact 1:1 при
  200% DPI таскбаре) через `Image::from_bytes(include_bytes!(...))`. Включена
  Tauri Cargo feature `image-png` для PNG-decoding. .exe-file-icon
  (RT_GROUP_ICON для Explorer) не затронут — там по-прежнему все 10 sharp-фреймов
  через `embed_resource`. Заодно `icon.ico` пересобран из правильных источников
  по размеру: 16/20/24 — Lanczos из `32x32.png` (раньше over-downsample 4× из 64),
  32 / 64 — exact, 40/48 — Lanczos из `64x64.png`, 96/128/256 — Lanczos из
  `icon.png` (512×512 full logo). Граница 64→96 совпадает с переходом Windows
  к полноразмерному рендеру на ≥175% DPI.
- **B-000007** Default-меню WebView2 (Inspect / Reload / "Другие инструменты" /
  "Направление письма") больше не появляется по ПКМ в release-сборке —
  ни на основном UI, ни в полях ввода. В `+page.svelte` глобальный
  contextmenu-handler под `import.meta.env.PROD`-гардом подавляет нативное
  меню повсюду; для `<input>` / `<textarea>` рендерится новый кастомный
  `InputContextMenu.svelte` — fixed-position 4-item меню (Вырезать /
  Копировать / Вставить / Выделить всё) с position-clamp в viewport,
  закрытием по outside-click / Esc. Cut/Copy disabled при отсутствии
  выделения, Cut/Paste disabled на readOnly/disabled полях. Clipboard
  через `navigator.clipboard` (works в WebView2 privileged context).
  `ctxMenu` хранится через `$state.raw` (deep-proxy Svelte 5 не дружит с
  DOM-элементом внутри — ассайнмент тихо проваливался в build'е). В dev
  меню остаётся доступным для отладки. Горячие клавиши Ctrl+C/V/X/A не
  затронуты. Новые i18n-ключи `ctx.cut/copy/paste/selectAll`.
- **B-000008** В About убран standalone-пункт «ИИ-ассистент: Claude (Anthropic)»
  из devs-one-liner — звучал как product placement. AI-tooling теперь упоминается
  inline после автора: «Автор: Сгоннов Д.А., с ИИ-помощниками · Лицензия: MIT».
  i18n: удалены `about.devs.aiAssistant` + `about.devs.aiValue`, добавлен
  `about.devs.aiHint`.
- **B-000006** Иконки кастомного titlebar (свернуть / развернуть / закрыть)
  заменены с Unicode-глифов (─ □ ✕) на чёткие 12×12 SVG-stroke иконки.
  Maximize-кнопка теперь свапается на restore-down иконку (две перекрывающиеся
  рамки, как в нативном Windows) когда окно развёрнуто, tooltip между
  «Развернуть» / «В окно». State синхронизируется через `appWindow.onResized()`
  + `isMaximized()` после toggle — корректно отражает snap-resize, double-click
  на titlebar и OS-шорткаты. Новый i18n-ключ `app.restore`.
- **B-000005** Сортировка по приоритету и статусу в Tasks-табе теперь по
  workflow-весу, не по алфавиту: priority `critical → high → medium → low`,
  status `open → in-progress → review`. В DataGrid ColumnDef добавлены
  опциональные `sortWeight: Record<string,number>` (вес для workflow-порядка)
  и `labelMap: Record<string,string>` (локализованный label в ячейке + filter
  dropdown + chips, raw value для match-логики не меняется — значения вне
  формата пропускаются неизменно и группируются в конце сортировки). Лейблы
  переведены: критический/высокий/средний/низкий, открыта/в работе/на ревью.
  Заодно `TasksTab.columns` переписан с `const` на `$derived` — labels
  пересобираются при смене локали (раньше captured один раз на mount).

## [0.22.0] — 2026-04-28

### Added
- **T-000054** Stats tab redesign: новый `StatsSummary.svelte` с lifetime-only
  KPI(4: Активные / Закрыто всего / Среднее попыток / Fix rate) + (project-only)
  🔥 Top-3 hot repos within project + Category efficiency bars (sort by % closed
  DESC). Lifetime-banner показывает дату создания scope'а и days_history.
- 2 new Tauri commands: `get_repo_stats_summary`, `get_project_stats_summary`.
- 3 new DB queries: `top_hot_repos_in_project`, `stats_summary_for_repo`,
  `stats_summary_for_project`.
- 4 new DTOs: `StatsSummary`, `StatsKpi`, `CategoryBar`, `HotRepo`.
- 10 new unit tests (top hot ordering / confirmed excluded / zero-active filter,
  basic stats / empty / categories sorted / avg+median, project aggregate +
  top hot / empty / repos no bugs).
- 30 new i18n keys `stats.summary.*` × ru + en.
- **T-000056** Recent Activity Feed: compact 10-event timeline embedded in Stats
  tabs of both RepoDetail and ProjectDetail. Per-day grouping, deep-link "Все
  события →" navigates to top-level Timeline screen with pre-filled scope filter.
  Backend reuses existing `read_timeline` (no new queries / DTOs / migrations).

### Changed
- Stats таб в `RepoDetail` и `ProjectDetail` рендерит новый `<StatsSummary>` вместо
  старой `<StatsTable>` pivot-таблицы (severity × category × date).

### Removed
- `StatsTable.svelte` компонент.
- Tauri commands: `get_repo_stats`, `get_project_stats`, `get_global_stats`,
  `get_all_stats`. DB methods и `BugStatRow` DTO/interface — удалены за ними.
- 3 legacy db.rs тестов, заменённых новыми stats_summary_* тестами.
- VIEW `bug_stats` остаётся в схеме как dead code (no migration). Cleanup —
  deferred в v0.23.0 если потребуется.

## [0.21.1] — 2026-04-27

### Fixed
- **B-000002** Хронология фильтр по репо показывал `owner/repo` вместо short-name; теперь `getDisplayName(r)` (last segment github_name или description), совпадает с Sidebar и графом.
- **B-000003** Двойная стрелочка `← ←` на кнопке "Назад" в AppDefaultsScreen — в template был hardcoded `← {settings.back}`, а i18n value `'settings.back'` уже содержит `← Назад`. Убрал hardcoded prefix; стрелка только из i18n.
- **BugNotes UX:** после клика ✓ confirmed-баг исчезал из списка моментально (re-fetch с filter не-confirmed). Теперь optimistic local-mutation — строка остаётся видимой со styled-confirmed (серый фон, ✓ маркер, отключённые контролы) до manual Refresh. Пользователь видит "click registered" feedback вместо мгновенной пропажи.
- **Bug LLM-acknowledgement workflow восстановлен** — confirmed-баги остаются в `docs/bug-reports.md` после клика ✓ в app, чтобы LLM на следующей сессии увидел подтверждение и удалил строку как cleanup. Раньше (v0.16.0..v0.21.0) `regenerate_bugs_md` фильтровал confirmed на write-side, и LLM никогда не видел подтверждение. Теперь:
  - App ставит `status='confirmed'` + `confirmed_at`, MD пере-генерируется с видимой строкой
  - LLM на следующей session edit удаляет confirmed-строку (per global spec)
  - `reconcile_bugs_for_repo` детектит удаление и ставит `archived_from_md_at = NOW`
  - Subsequent regen'ы постоянно исключают строку (DB row остаётся для истории)

### Added
- **DB schema migration v22** — новая колонка `bugs.archived_from_md_at TEXT` (NULL по умолчанию). Маркер LLM-acknowledgement.
- **`db.list_bugs_for_md(repo_id)`** — возвращает active rows + не-archived confirmed rows для регенерации MD.
- **`db.mark_bug_archived_from_md(bug_id)`** — idempotent helper (не перезаписывает существующий timestamp).
- **2 новых Rust теста** (277 → 279):
  - `test_regenerate_bugs_md_excludes_archived_confirmed`
  - `test_reconcile_marks_confirmed_archived_when_llm_removes_from_md`
- Test `test_regenerate_bugs_md_includes_only_non_confirmed` переименован → `_includes_unacknowledged_confirmed` с обновлёнными assertions.

### Notes
- Существующие confirmed-баги в legacy DB считаются `archived_from_md_at = NOW` при импорте через `migrate_bugs_for_repo` — preserves UX expectation "confirmed-from-MD-import → drops from MD". Только fresh confirmations через v0.21.1+ работают по новому workflow.
- DB-side bug history (toggle "Показать закрытые" в BugNotes) не изменился — читает из DB напрямую через `count_confirmed_bugs` / `read_bugs_from_db`, видит ВСЕ confirmed independently of MD-state.

## [0.21.0] — 2026-04-27

### Added
- **F-000013 Project graph** — новый таб "Граф" в ProjectDetail с интерактивной картой проекта (cytoscape.js): сервер в центре, клиенты + микросервисы вокруг. Pan/zoom (Ctrl+scroll) + click → переход в RepoDetail / другой ProjectDetail. Theme switching через CSS-vars + cy.style().update(). Concentric layout, role-coloring (server=blue, client/landing=green, ms=purple, tool=gray), dashed edges для cross-project ms.
- **T-000055 Settings UX redesign** — Settings переразбит в 4 thematic карточки (GitHub PAT / Внешний вид / Рабочее пространство / Шаблоны), compact rows вместо отдельных карточек на настройку. ~50% вертикальной экономии. PAT-tooltip "Токен нужен для синхронизации репозиториев".
- **T-000050 Local-only repo rename detection** — новая функция `update_repo_description` + Tauri command, hook на изменение description логгирует событие в `repo_renames` (только для local-only репо, где canonical = description). UI: click-to-edit имя в RepoDetail + inline-list "↳ ранее: <old> (date)" под заголовком.
- **ProjectDetail tabs** — переразбит на 4 таба (Репозитории / Микросервисы / Граф / Статистика) по паттерну RepoDetail, header получил [✏ изм] + [⌫ del] action-кнопки в правом углу.
- Tauri command `list_renames_for_repo` (per-repo rename history fetch).

### Changed
- ProjectDetail header: delete-button переехал из низа страницы в правый верхний угол (паттерн от RepoDetail).
- StatsTable в ProjectDetail таб больше не collapsible — раз в табе, всегда развёрнуто.
- В шапке табов вместо текстового section-label остался только counter-badge `(N)` — title уже отражён в активной таб-кнопке.

### Removed
- `SettingsRenameLog.svelte` компонент — переехал в RepoDetail header как inline-list (per-repo).
- Settings "История" карточка — нечего настраивать, history виден per-repo.
- **Project-level Secrets tab** — секреты задаются per-repo в RepoDetail; дублирование на уровне проекта избыточно. SecretsPanel mode="project" путь больше не используется в ProjectDetail.

### Fixed
- ProjectGraph рендерился в контейнер 0×0 (parent `.project-detail` использует block+overflow-y, не flex). Добавлен явный `min-height: 600px` + `height: calc(100vh - 280px)` на `.graph-wrapper`.
- Метки узлов в графе показывали полный github_name (`owner/repo`) вместо short-name. Backend теперь использует `canonical_folder_name()` (last segment github_name или description) — совпадает с frontend `getDisplayName`.
- Колонки в таблице репозиториев в ProjectDetail были hardcoded English (`Repo` / `Lang`); теперь локализованы через `project.colRepo` / `project.colLang`.

### Notes
- Bundle size impact: cytoscape.js добавляет ~200KB к app-bundle. Acceptable для desktop app.
- Test count: 270 → 277 (+3 для T-050 + 4 для F-013).
- Migration не требуется — все изменения SoT/schema совместимы с существующей DB.

## [0.20.2] — 2026-04-26

### Fixed
- DataGrid: длинное описание задач/сделанного больше не вытягивается в одну сверхширокую строку — wrap до 3 строк с многоточием, переменная высота строк (`white-space: normal` + `-webkit-line-clamp: 3`).
- Поиск в гридах теперь покрывает не только description, но и task ID (любая monospace-колонка). Раньше по `T-000042` не находилось.

### Changed
- Filter-кнопка в шапке колонки гридов стала заметнее: `▾` вместо `⚙`, рамка при hover, акцентная рамка + бейдж со счётчиком при активном фильтре. Discoverability вырос — раньше иконка была 10px и сливалась с текстом.

## [0.20.1] — 2026-04-26

### Changed
- Унификация формата task ID: `T-NNN` / `F-NNN` / `D-NNN` → `T-NNNNNN` / `F-NNNNNN` / `D-NNNNNN` (6-значный zero-padded, как у багов с v0.16.0). Новые задачи LLM пишет в 6-значной форме; парсер читает оба формата (legacy 3-digit + new 6-digit) — backwards-compatible.
- `parse_done_tasks` synthetic counter `D-{:03}` → `D-{:06}` (например, для строк с пустым id-слотом).
- Global CLAUDE.md template (`claude.md.global.tmpl`) обновлён: format spec для todo.md / done.md / bug-reports.md теперь явно говорит про 6-digit формат + parser leniency. Юзеру нужно нажать "Sync global CLAUDE.md" в Settings — обновлённая spec пушится в `~/.claude/CLAUDE.md`.

### Notes
- Existing legacy IDs в todo.md / done.md существующих репо НЕ переписываются — они продолжают читаться парсером. На write (новый task через LLM) используется 6-digit. Постепенный transition без принудительной миграции файлов.

## [0.20.0] — 2026-04-26

### Added
- Универсальный `<DataGrid />` Svelte component для filter/sort/search/persist (text + select filter types)
- Раздельные вкладки **Задачи** и **Сделано** в RepoDetail (replace single Tasks tab) с new DataGrid
- Новый top-level экран **Хронология** (📅 в titlebar): multi-source timeline с date-range / event-kinds / repos / search фильтрами, per-day grouping, pagination
- todo.md format: 6-е поле `created_at` (`YYYY-MM-DD`), legacy 5-field парсится, mtime-backfill при первом sync
- DB-mirror tasks с migration v21 (4 new tables: tasks, task_events, sync_events, deploy_events; mirror v0.16.0 bugs SoT pattern)
- Event recording hooks: sync_project, write_deploy_files, secret push/delete (через TS-side `record_*_event` thin commands)
- 14 новых Rust тестов (251 → 264), новые i18n keys

### Changed
- DashboardActivityFeed подхватил sync/deploy/task event types через расширенный recent_activity (теперь делегирует на read_timeline_filtered)
- ActivityEvent.repo_id теперь Optional (portfolio-wide sync_events не имеют конкретного репо)
- write_deploy_files command принимает deploy_env_id + repo_id для event recording
- delete_repository теперь cleanup'ит `tasks_grid_state_<id>` и `done_grid_state_<id>` settings keys

### Removed
- Компонент `RepoDocsTab.svelte` — replaced by TasksTab + DoneTab (-306 строк)

### Notes
- После установки v0.20.0 рекомендуется в Settings нажать "Sync global CLAUDE.md" — пушит обновлённый формат-spec todo.md (6-е поле) в `~/.claude/CLAUDE.md`. LLM прочтёт обновление при следующей сессии.
- Timeline начинает заполняться событиями по мере работы; полный исторический backfill за период до v0.20.0 не выполняется (только rename log + bug events с момента v0.17.0).

## [0.19.0] — 2026-04-26

### Added
- Мини activity-feed в Dashboard: последние 10 событий портфеля (`bug_events` + `repo_renames` через UNION ALL), click → repo-detail
- Sidebar collapsible (VS Code-style): 52px initials-strip с цветом по типу проекта (стандарт серый, microservice синий), click на icon → expand + select project + scroll
- Sidebar drag-resize: handle на правой границе, ширина 200..500px, snap в collapsed при drag past 160px threshold, rAF-throttled live preview
- Persist sidebar layout (`sidebar_width`, `sidebar_collapsed` в settings, debounced 300ms)
- 5 новых Rust тестов для `recent_activity` (235 → 240)

### Changed
- Default screen на старте app'а: Dashboard (вместо RepoList с unassigned-репо)
- Текст back-button в RepoDetail: «Назад к Дашборду» (был «Назад к репозиториям»)

### Removed
- Компонент `RepoList.svelte` (-263 строки) и route `'repo-list'` (дублирование с Sidebar drag-drop)
- i18n блок `repoList.*` (46 ключей в ru+en)

## [0.18.0] — 2026-04-25

### Added
- **Multi-environment deploy (T-044):** один репозиторий может иметь несколько параллельных деплоев (prod/test/staging/любое имя). Новая таблица `deploy_environments`, DeployScreen переделан на master-detail (таблица деплоев + drill-down).
- **Clone deployment:** при создании нового деплоя можно выбрать "Copy from: ..." — копируются placeholders + deploy_secrets флаги (значения env-scoped секретов не копируются — GitHub API не отдаёт values).
- **meta.json v4 (T-046):** `role: build/deploy/runtime` + `scope: repo/environment` на каждом required_secret. Генератор разносит секреты по ролям: build → `docker build --build-arg`, runtime → `docker run --env`, deploy → workflow context.
- **GitHub Environments integration:** env-scoped секреты пишутся в GitHub Environments native. Workflow использует `environment: @@ENV_NAME@@` на jobs — GitHub резолвит `${{ secrets.NAME }}` с env-scoped override если есть, иначе наследует с repo level.
- Placeholders `NETWORK_NAME`, `COMPOSE_PROJECT` в go + flutter_web шаблонах — убирают hardcoded `goapp01_prod_proxy-network` / `goapp01_prod`.
- Sync-trigger в SecretsPanel: после успешного PUT нового repo-секрета приложение регистрирует его в `deploy_secrets` для всех деплоев репо.
- Cascade-cleanup при удалении репо-секрета: убираются и его `deploy_secrets` строки во всех деплоях.

### Changed
- `deploy_manifests` таблица → переименована в `deploy_environments` (migration v20). Существующие manifest'ы мигрируют как `name='prod'`.
- `CONTAINER_NAME_PROD` → `CONTAINER_NAME` в шаблонах (имя контейнера теперь per-env secret).
- flutter_web Dockerfile использует `@@DOCKERFILE_ARGS@@` + `@@DART_DEFINES@@` вместо hardcoded `ARG API_BASE_URL`/`ARG APP_API_KEY`. Build-args = UNION всех `role=build` секретов по всем deploys репо.
- Go deploy.yml эмитит `--env KEY=...` для каждого runtime-секрета; `ENV_FILE_PATH` остаётся как escape-hatch для bulk-переменных.
- flutter_web meta.json target `dockerfile.tmpl` → `Dockerfile` (было `dockerfile` lowercase, ломалось на Linux CI).
- Bundled meta.json рекомендует `role` + `scope` как hints — пользователь свободен переопределить в UI.

### Removed
- `DeployManifest` struct + `get_deploy_manifest` / `save_deploy_manifest` / `render_deploy_files` Tauri commands. Заменены на `list_deploy_environments` / `get_deploy_environment` / `create_deploy_environment` / `clone_deploy_environment` / `update_deploy_environment` / `delete_deploy_environment` / `render_deploy_files_for_env`.
- Rename deployment flow — `name` становится read-only после создания. Для переименования: clone с новым именем + удаление старого.
- Hardcoded `build-args: API_BASE_URL=...` + pre-build ".env from secrets" step из flutter_web deploy.yml.
- Add/remove secret из самого деплоя: добавление новых секретов и физическое удаление из `deploy_secrets` происходит через **репо Secrets tab** (sync-trigger пропагирует во все деплои). Деплой только тогглит флаги Include/Override.

### Fixed (post-dogfood polish)
- DeploySecretsTable race condition: child component own'ит весь pipeline (list repo secrets → ensure_populated → list deploy_secrets → list env-scoped). Прежде parent seed'ил параллельно с child list'ом → таблица была пустая на первом open.
- Toggle `Включить` / `Своё значение` больше не ресетит scroll в начало — оптимистичный local update в `dbSecrets` без `await load()`.
- `common.cancel` / `common.loading` i18n ключи добавлены — раньше UI показывал literal `common.cancel` в кнопке отмены.

### UI polish (post-dogfood)
- DeployScreen master: ⎘ копирующая иконка в **первой** колонке (всегда видна, не на hover). Клик по ⎘ ведёт прямо в clone-flow без dropdown'а — источник из выбранной строки.
- New-deployment форма упрощена: только поле имени (branch выбирается на детальном экране через GitHub branches dropdown).
- DeploySecretsTable single-line layout: имя → role-chip → Override → input (flex) → Include. Поле ввода всегда видно (disabled пока override не включён).
- Role представлен как кликабельный цветной chip: `BUILD` (indigo) → `DEPLOY` (teal) → `RUNTIME` (amber). Tooltip с объяснением. Click — cycle.
- Inline label/input layout с tooltip-описанием для placeholder'ов (вместо 3-row label/input/desc).
- GitHub branches dropdown для DEPLOY_BRANCH через `<datalist>` — autocomplete + free-text fallback.
- "Generate workflow files" прижата вправо для visual hierarchy.

## [0.17.0] — 2026-04-24

### Added
- Dashboard redesign: period filter (Неделя/Месяц/Квартал/Кастом), multi-select projects filter, 5 KPI tiles with partial-same-length period comparison, Top-3 hot projects, per-day charts (bugs opened/closed + tasks done), category efficiency bars with 9 correct categories from DB.
- Migration v19: `bug_events` log table with 3 indexes + `idx_bugs_confirmed_at`. Back-fills synthetic `entered_testing` events for pre-v19 bugs, preserving `COUNT(entered_testing) == fix_attempts` invariant.
- `bug_events` recorded on every status transition (in lib.rs commands + sync.rs reconcile).
- `BugCategory` enum as single source of truth in Rust; TS mirror in `types.ts`.
- Dashboard date math helpers (`resolvePeriod` / `resolveComparePeriod`) with 12 unit tests covering Mon-start week, Q1→Q4-prev-year rollover, end-of-month clamp, d<1 edge.

### Changed
- Dashboard.svelte split into 5 sub-components (DashboardFilters, DashboardKpi, DashboardDailyChart, DashboardCategoryBars, DashboardTopHot).
- `BugCategory` TS type cleaned up — removed stale `backend`/`network`/`unknown`, aligned with 9 DB-valid categories.
- `bug_stats` VIEW stays but is used only by per-repo StatsTable. Dashboard uses its own DB queries (no incremental drift possible).

### Removed
- Three legacy dashboard tables (By Category / By Severity / By Status) with raw `bugs/attempts` cells.

## [0.16.0] — 2026-04-24

### Added
- **Bug architecture rework (T-025 / T-026 / T-027)** — SQLite стала source-of-truth для багов; MD-файл `docs/bug-reports.md` остался LLM-facing view с 2-way sync. Новая таблица `bugs` (12 полей: `id`, `repository_id`, `numeric_id`, `display_id`, `created_at`, `description`, `severity`, `category`, `status`, `fix_attempts`, `comment`, `confirmed_at`) + 3 индекса (`idx_bugs_repo`, `idx_bugs_status`, `idx_bugs_repo_date`). Миграция v18 в DB.
- **История закрытых багов** — confirmed-строки физически остаются в DB, выпадают только из MD-view. Доступны через toggle "Показать закрытые (N)" в BugNotes (серый фон + ✓ префикс + дата закрытия). Раньше LLM вычищал confirmed-строки из MD при следующем edit — история терялась.
- **6-значный ID формат `B-NNNNNN`** — переход с 3-значного (`B-001`, cap 999) на 6-значный (`B-000001`, cap 999999). Parser остался lenient к любой длине (`\d+`) — существующие MD с `B-042` мигрируются бесшовно, numeric-id preserved (42 → display `B-000042`).
- **Lazy MD→DB migration** — при первом открытии bug-таба репо в v0.16.0 автоматически импортирует содержимое `docs/bug-reports.md` в DB (идемпотентно, повторные open — no-op). Pre-check duplicate ID, transactional INSERT, marker `repositories.bugs_migrated_at`. Toast "Импортировано N багов, из них M в архив".
- **Reconciliation MD ↔ DB** — на open bug-таба / Refresh / global Sync вызывается `reconcile_bugs_for_repo`: LLM-правки `status`/`comment` в MD ингестятся в DB, protected-fields (description/severity/category/fix_attempts/date) молча восстанавливаются через regen, orphan-строки и illegally-deleted rows silent-remove/silent-restore.
- **Новые Tauri commands** — `ensure_bugs_migrated`, `reconcile_bugs_for_repo`, `read_bugs_from_db`, `count_confirmed_bugs`, `create_bug`, `update_bug_fields`, `delete_bug`, `resolve_bug`, `reject_bug`. DTO `BugView` (9 полей с `confirmed_at`) отдельно от MD-формата `FileBugNote` (8 полей).
- **33 новых теста в Rust** (+18 в `sync::tests`, +15 в `db::tests`): migration idempotency, preserve numeric_id, duplicate-id abort, status transitions increment attempts, protected-field restore, orphan removal, deleted-row restore, invalid transition ignored, VIEW correctness на CRUD, per-repo counter independence. Всего 183 cargo-теста.

### Changed
- **`bug_stats` table → VIEW** — инкрементальная таблица с drift-prone write-handlers (~150 строк в `db.rs` + `lib.rs`) удалена. Stats теперь live-computed из `bugs` таблицы (`CREATE VIEW bug_stats AS SELECT ... GROUP BY repo, severity, category, date`). Dashboard/StatsTable SQL-запросы работают без изменений — SQLite трактует `SELECT FROM bug_stats` прозрачно. Drift by construction невозможен.
- **Stats-recalculate кнопка убрана** из Dashboard — VIEW всегда актуален, ручной пересчёт потерял смысл. Та же кнопка убрана из RepoDetail stats-таба.
- **bug_notes таблица удалена** из SQLite (legacy с v1, не использовалась с тех пор как баги переехали в MD в v4).
- **Delete bug UI** — теперь доступен только для `status='created'` (accidental creation escape hatch). Для реально отработанных багов путь один — через ✓ confirm в testing-статусе (мягкий архив с сохранением истории).
- **BugItem confirmed-styling** — строки с `status='confirmed'` визуально отличаются: полупрозрачный фон, ✓-маркер вместо ✓-кнопки, selects/edit заблокированы, справа показан `confirmed_at` дата зелёным.
- **fix_attempts -/+ ручные кнопки убраны** из BugItem — счётчик теперь полностью app-managed, инкрементится только при валидном переходе в `testing`. Ручная правка больше не имеет смысла (перезапишется regen'ом).
- **Status badge в BugItem** — цветная плашка (created=серая / in-progress=синяя / testing=оранжевая / rejected=красная / confirmed=зелёная) после даты. Раньше статус был виден только косвенно через наличие ✓/✗ кнопок; теперь читается сразу. i18n через `status.*` ключи (уже существовали).
- **onMount reconcile в BugNotes** — при каждом remount компонента (открытие bug-tab после переключения с Tasks/Changelog/Stats) вызывается `refreshBugs`. Раньше reconcile триггерился только при смене selectedRepoId — MD-правки LLM'а во время работы на другом tab'е оставались невидимыми до явного Refresh.
- **LLM shortcut transitions разрешены** в `valid_transition`: добавлены `created → testing` (быстрый fix без in-progress marker) и `rejected → testing` (retry после rejection). Обе ведут в `testing` → `fix_attempts +1`. Это делает natural LLM-сессионный flow (создал бага → взял → сразу testing) рабочим без обязательного промежуточного refresh. Invalid transitions (e.g. `created → confirmed`, `confirmed → *`) остаются запрещены.
- **i18n +13 ключей** в `bugs.*` и `bugItem.*`: showConfirmed, showConfirmedHint, confirmedAt, migrationToast, migrationError, duplicateIdError, confirmTooltip, rejectTooltip, confirmedBadge, attemptsLabel, attemptsTooltip, rejectConfirmTitle, rejectConfirmMessage, addComment, commentPlaceholder (все ru + en).

### Removed
- **8 incremental bug_stats функций** из `db.rs`: increment_bug_stat, decrement_bug_stat, add_attempts_stat, subtract_attempts_stat, transfer_bug_stat, increment_resolved_stat, reset_repo_stats, reset_all_stats. Их write-команды в `lib.rs` оставлены как Ok-stub'ы для backward-compat (фронт будет чистить в следующих патчах).
- **Старые MD-centric store-функции** из `src/lib/stores/bugs.ts`: `loadBugsFromFile`, `flushBugs`, `reloadBugs`, `setStatus`, `incrementAttempts`, `decrementAttempts`. Новый API: `loadBugsForRepo`, `refreshBugs`, `toggleShowConfirmed`, `rejectBugWithComment`.

### Fixed
- **Stats drift** — `attempts_count` в Dashboard мог привирать из-за инкрементальных UPDATE'ов, которые не переживали ручные MD-правки вне app, LLM-edits, миграции и баги хендлеров. Теперь VIEW пересчитывается live из `bugs` таблицы — правда, только правда, и ничего кроме правды.

## [0.15.4] — 2026-04-23

### Added
- **Donate-секция в About** — новая карточка "Поддержать разработку" с двумя способами: **Boosty** (`boosty.to/sgonnovdm/donate`, RUB / российские карты / СБП, кликабельная ссылка через `@tauri-apps/plugin-opener`) и **TON-кошелёк** (`UQA-0I3SN2vw8F2ZzEoOTXT36-ToF0mu4Yp4_6pVmsR_dI0S`, moncypace code-блок + copy-to-clipboard кнопка через `navigator.clipboard`, подтверждение "✓ Скопировано" на 2 сек). i18n ключи `about.support.*` (7 × ru/en). Донаты прилетели раньше v1.0.0 — T-045 остаётся только с pubic repo init + README polish для ребренда.

### Changed
- **Error state в About — убран блок "Технические подробности"** — raw-сообщение от плагина больше не показывается в UI (захламляло). Friendly-сообщение + кнопки "Повторить"/"Скрыть" остались. Для отладки технический err теперь логируется через `console.warn` (видно в DevTools). i18n ключи `about.update.error.*` переформулированы без упоминания "подробности ниже".

## [0.15.3] — 2026-04-23

### Changed
- Тестовый релиз для проверки из установленной 0.15.2 нового autoupdate UX живьём: зелёная CTA-кнопка в titlebar (слева от 📊 Dashboard), новый порядок в About (install-кнопка раньше release notes), silent-check без 24h кеша (каждый cold start — свежий запрос).

## [0.15.2] — 2026-04-23

### Added
- **Update CTA-кнопка в titlebar** — когда autoupdate находит доступное обновление, слева от 📊 Dashboard в верхнем баре появляется зелёная кнопка `⬇ Обновление X.Y.Z`, клик открывает About для install-and-relaunch. Раньше был только незаметный красный dot на ℹ-кнопке — его убрали, CTA перекрывает. Рендерится только когда `updaterStatus.kind === 'available'`.

### Changed
- **About "Доступна версия X" — порядок элементов** — install-кнопка теперь сразу после заголовка, release notes "Что нового" вниз. CTA-действие доминирует, notes — справка.
- **Silent-check без 24h кеша** — каждый холодный старт триггерит проверку обновлений (раньше кеш мог пропустить release в первые 24h после публикации). HTTP-нагрузка копеечная (1 запрос к GitHub CDN на launch), зато user видит CTA сразу.

## [0.15.1] — 2026-04-23

### Changed
- **Updater error UX** — raw-сообщения от `tauri-plugin-updater` (типа `Could not fetch a valid release JSON from the remote`) заменены на friendly тексты. Ошибки классифицируются на `notFound` / `network` / `signature` / `unknown` + техническое сообщение доступно в свёрнутом блоке `<details>`. Новые i18n-ключи `about.update.error.*` (ru/en × 5). Добавлена кнопка "Повторить" рядом с "Скрыть".

### Fixed
- Docs: Public launch (T-037 / T-045) возвращён в **v1.0.0** (как и планировался исходно — создание нового public репо с нуля, не rename private). В v2.0.0 остаётся только **T-051** Windows code signing как post-launch polish. Правка в todo.md / roadmap.md / CLAUDE.md / Changelog [0.15.0] migration notes.

## [0.15.0] — 2026-04-23

### Added
- **T-038 / F-018 Автообновление** — `tauri-plugin-updater` + GitHub Actions release-pipeline. При старте app тихо проверяет `latest.json` раз в сутки (timestamp в localStorage); в About появилась секция "Обновления" с кнопкой "Проверить обновления", прогресс-баром загрузки и кнопкой "Установить и перезапустить" в один клик. В titlebar — красный dot-badge на кнопке "О программе" когда апдейт доступен.
  - **Ed25519 signing** через Tauri updater — integrity update-канала (приватник в GitHub Actions secret, паблик в `tauri.conf.json`). Windows Authenticode (SmartScreen) намеренно за scope'ом, задача **T-051** в v2.0.0 (post public-launch polish).
  - **Release workflow** (`.github/workflows/release.yml`) — триггер push тега `v*` → `tauri-apps/tauri-action@v0` собирает на `windows-latest`, подписывает артефакты, генерирует `latest.json`, публикует GitHub Release. Release notes auto-extract из секции `## [X.Y.Z]` Changelog.md через `scripts/extract-changelog.mjs`.
  - **Release-тег вида `v*-rc*` / `v*-beta*` / `v*-alpha*`** автоматически помечается как prerelease.
  - Dependencies: +`tauri-plugin-updater 2`, +`tauri-plugin-process 2`, +`@tauri-apps/plugin-updater`, +`@tauri-apps/plugin-process`.
  - i18n: +17 ключей × ru/en (`about.update.*`).

### Changed
- **Процесс релиза** — локальная `npm run tauri build` с ручной раздачей `.exe` заменена на `git tag vX.Y.Z && git push origin vX.Y.Z` → CI сборка + публикация. Runbook в [docs/RELEASING.md](docs/RELEASING.md).

### Migration notes
- **0.14.0 → 0.15.0 переход — руками (one-time)**. 0.14.0 не содержит updater-plugin, поэтому in-app update с неё на 0.15.0 невозможен; установи 0.15.0 `.exe` вручную. Дальше (0.15.0 → 0.15.1, 0.15.0 → 0.16.0 и т.д.) — всё через autoupdate.
- **SmartScreen warning при первом запуске 0.15.0** — ожидаем (как и у прошлых локально-собранных версий). Устраняется в T-051 (Azure Trusted Signing) после v1.0.0 public launch.
- **Autoupdate endpoint 404 на private репе** — installed 0.15.0 не сможет проверить обновления пока текущий репо private. В v1.0.0 переезд на новый public репо `solo-dev-hub` снимает ограничение. До v1.0.0 — manual install `.exe` с Releases page.

## [0.14.0] — 2026-04-23

### Added
- **F-033 Cross-repo sync folder naming + rename-log** — папки sync-директорий теперь именуются по canonical repo name (last segment of `github_name`, унифицировано для client/microservice). `server-requirements/` на microservice side **nested per parent-server-repo** — снимает collision для multi-parent microservice (блокер #1 для multi-parent сценария).
  - **Rename-log в DB** (таблица `repo_renames`, миграция v16): при переименовании репы (detect'ится в `upsert_repository_with_outcome` через github_id match + различный github_name) запись добавляется в лог. На следующем sync app переименовывает counterparty-side папки на fs. Идемпотентно — нет state-поля "applied", fs сама state.
  - **Sync-preamble replay** — 3 direction'а (client→server, server→ms на обеих сторонах).
  - **One-time migration для existing installations** (Case A/B/C внутри sync, идемпотентно):
    - Case A — client folders уже корректные, no-op.
    - Case B — `microservice-requirements/<project-name>/` → `<ms-canonical>/` на server side.
    - Case C — flat `server-requirements/*.md` на ms side → nested `<parent-canonical>/*.md`. Attribution через byte-equal content-match с parent-side копиями. 4 ветки: 1 match → rename; >=2 matches с одинаковым content → копируем в каждый subfolder + удаляем flat; conflict → leave + error; orphan → leave + warn.
  - **UI rename-log viewer** — `SettingsRenameLog.svelte`, expandable `<details>` в Settings с read-only таблицей (repo id | old → new | timestamp). Re-load on expand.
  - **Tauri command** `list_rename_history` + frontend wrapper `listRepoRenames`.
  - **Tests** — 12 новых в sync.rs (replay × 4, subfolder rename × 2, Case C × 6) + 2 rename detection hook в db.rs. Итого 159/159.
- **T-049 RepoDocsTab refresh + reverse done** — 🔄 кнопка в section header для Todo и Done, `ontoggle` re-read при раскрытии секции. Done отсортирован reverse (новые сверху — соответствует file append pattern done.md). i18n `tasks.refresh`.

### Changed
- **T-047 Settings UI compaction** — Language + Theme объединены в одну `.card-row` (horizontal flex), вместо двух full-width карточек. `.preference` items с labels для чёткой hierarchy.
- **`microservice-requirements/<X>/`** subfolder на server side теперь `<ms-server-repo-canonical>` вместо `<ms-project-name>`. `microservice-api/<ms-project-name>/` (для api.md/handlers.md sync) оставлен как был — другой сценарий, рефактор отложен.
- **Removed `copy_if_missing` helper** + 3 его теста — функция больше не использовалась после перехода на `copy_file_if_changed` в 0.13.27.
- **Removed `Repository::github_name_or_empty()`** — заменено везде на `canonical_folder_name()`.

### Fixed
- **Default bug-reports path** — `bug_file_path` setting defaulted к `'docs/bug_list.md'` (устаревшее имя), не к `'docs/bug-reports.md'` из template. Новые установки без выставленного setting читали НЕПРАВИЛЬНЫЙ путь. Исправлено в рамках T-048 — путь hardcoded `'docs/bug-reports.md'`.

### Removed
- **T-048 `bug_file_path` setting** — путь зафиксирован в template contract. Удалены: SQLite key (миграция v17 очищает существующие записи), Settings card, `bugFilePath` store + `saveBugFilePath`, 3 i18n ключа × ru/en, `recalculate_all_stats` param, import'ы в RepoDetail/Dashboard.

### Migration notes
- **One-time sync migration** для существующих установок запускается автоматически при первом sync после 0.14.0 (Case B/C в F-033). Warnings/errors выводятся в `SyncResult.errors` если migration ambiguous (multi-parent ms с unsafe content divergence, orphan files).
- **Rollback не поддерживается** — новый nested layout `server-requirements/<parent>/` на microservice side не читается 0.13.x кодом. Downgrade → sync не найдёт файлы. Если rollback нужен — руками перенести файлы обратно в flat root.
- **Local-only repo renames** не обрабатываются автоматически (canonical=description, но изменение description в UI не логгируется в `repo_renames`). Follow-up — T-050 в todo.md.

## [0.13.27] — 2026-04-22

### Changed
- **Cross-repo sync: edits propagate** — `REQ-*.md` и `*.response.md` теперь синхронизируются через `copy_file_if_changed` (было `copy_if_missing`). Когда источник истины (sender для REQ, recipient для response) редактирует файл — следующий sync перезаписывает копию на другой стороне. Раньше копировалось один раз, правки не пропагейтились. Применено в 4 местах: client↔server REQ/response, server↔microservice REQ/response.

### Removed
- **Reject requirement flow** — убрана ✗ кнопка из SyncScreen и Tauri-команда `reject_requirement`. Если sender не удовлетворён response — создаётся новый `REQ-N+1_<slug>.md` с уточнённым запросом (или recipient сам редактирует свой response, edit propagate'ится). Reject реально не нёс контракта — причина отказа нигде не сохранялась, коммуникация всё равно out-of-band. Убрали сущность, которая только добавляла кнопки без data trail'а. Удалены: Rust `reject_requirement`, frontend `rejectRequirement` wrapper + `handleReject`, 2 ✗ кнопки, CSS `.reject-btn`, i18n-ключ `sync.reject` × ru/en.
- **Flow doc update** — [docs/flows/requirements-sync.md](docs/flows/requirements-sync.md) обновлён под propagate-edits + без reject.

## [0.13.26] — 2026-04-22

### Changed
- **Global CLAUDE.md template** (`claude.md.global.tmpl`): добавлена секция `# Cross-repo requirements (Message/Receipt pattern)` между `# File formats` и `# Versioning`. Паттерн уже был в персональных рулах юзера (section 6), но это на самом деле часть общего контракта, обеспечиваемого app'ом через `sync.rs` (copy REQ/response между парой outgoing/incoming папок). Теперь формально зафиксировано: какие файлы (REQ-NNN_slug.md + REQ-NNN_slug.response.md), какие папки (flat vs nested per-repo), шаги sync-флоу 1-6, кто что может редактировать/удалять. Пересобирается в per-user `~/.claude/CLAUDE.md` при следующей syncronisation любого репозитория.

## [0.13.25] — 2026-04-22

### Fixed
- **B-006 (confirmed)** round 6 — precheck из 0.13.24 на реальном деплое выдал `roles=[] visibility=` даже после того, как user выставил `visibility='all'` в NPM UI. Root cause: `/api/users/me` **не включает** `permissions` в ответ без `?expand=permissions` — без expand объект `permissions` отсутствует, и `.permissions.visibility` парсится как пустая строка, precheck всегда fail'ит для non-admin user'а.
  - **Фикс:** precheck теперь дёргает `GET /api/users/me?expand=permissions`.
  - **Диагностика:** на fail-ветке теперь дампим весь raw-ответ NPM — если expand не поддерживается или структура ответа другая, сразу видно что именно пришло, без слепых гаданий.
  - Применено в flutter_web + go шаблонах + regression fixture.
  - Confirmed via deploy на swan_info_test_app на v0.13.25 — прошёл успешно после установки `visibility='all'` на NPM-аккаунте. Цепочка root cause закрыта: (1) NPM-scope bug, (2) missing `?expand=permissions` в precheck.

## [0.13.24] — 2026-04-22

### Added
- **B-006 round 5 (root cause + preventive check)** — проблема "already in use" на nginx step'е была не в шаблоне, а в **скоупе NPM-аккаунта**. `NPM_EMAIL`/`NPM_PASSWORD` в GitHub secrets указывали на user с `permissions.visibility='user'` — такой user через API видит **только собственные** proxy-hosts. NPM POST validation при этом проверяет **всю** БД → "already in use" от записей, недоступных нашему GET-списку. Диагностические cleanup-проходы находили `(none)` и POST retry падал.
  - **Fail-fast check** добавлен в nginx-step после auth: `GET /api/users/me` → проверка прав. Проходит если либо `roles` содержит `admin`, либо `permissions.visibility == "all"`. Иначе script падает с чётким guideline в какой UI-чекбокс NPM пойти: "Users → edit → tick Administrator, OR set Visibility to All Items".
  - Раньше issue manifест'ился как бесконечный цикл "POST 400 → cleanup пусто → retry 400". Теперь видно сразу: "insufficient scope" с инструкцией что нажать.
  - Применено в flutter_web + go шаблонах + regression fixture (148/148).
  - **NB:** precheck пока блокирует корректных non-admin user'ов с visibility='all' из-за отсутствия `?expand=permissions` — пофикшено в 0.13.25.

## [0.13.23] — 2026-04-22

### Fixed
- **Flutter build fail: "No file or variants found for asset: .env"** — в 0.13.19 убрал step `Create .env from secrets` из deploy.yml, заменив на `--dart-define`. Но проекты, использующие пакет **flutter_dotenv** (читает `.env` из assets в runtime), ломаются — `pubspec.yaml` у них объявляет `.env` как asset, flutter build требует файл, а его нет. Восстановил step, сохранил и `--dart-define` как belt-and-suspenders.
  - **Два pattern'а теперь coexist'ят:**
    1. `--dart-define` через ARG в Dockerfile — compile-time constants, baked в bundle (подход swanqu_server)
    2. `.env` file через `COPY . .` → flutter_dotenv reads в runtime — файл создаётся в GitHub Actions workspace перед `docker/build-push-action`
  - Проект сам выбирает, какой использовать, через pubspec.yaml assets + Dart-код. Template предоставляет оба пути, безвредный overlap если проект использует только один.
  - Коммент в Dockerfile объясняет оба варианта, чтобы user видя файл понимал когда какой нужен.
  - Применено в flutter_web шаблоне + regression fixture (148/148).

## [0.13.22] — 2026-04-21

### Fixed
- **B-006 round 3:** aggressive cleanup fallback в 0.13.21 всё равно не нашёл удерживающий домен host — потому что сканировал только **proxy-hosts**. NPM имеет 4 типа хостов (proxy, redirection, dead, stream); первые 3 могут удерживать домен. Если у user'а есть redirection-host или dead-host для этого домена (создан через NPM UI в прошлом, возможно случайно), POST proxy-host получает "already in use" а cleanup не находит source.
  - **Фикс:** fallback теперь сканирует **все 3 типа** (`proxy-hosts` / `redirection-hosts` / `dead-hosts`). Streams не сканируем — они биндятся к портам, не к доменам.
  - **Диагностика:** cleanup-pass теперь **дампит в лог** ВСЕ domain_names всех hosts каждого типа (`id=X: domain.com, other.com`). Если даже после cleanup POST retry fail'ит — в логе чётко видно где домен удерживается.
  - Применено в flutter_web + go шаблонах + regression fixture (148/148 tests pass).

## [0.13.21] — 2026-04-21

### Fixed
- **B-006 round 2:** deploy на swan_info_test_app всё равно упал с `info.*** is already in use` — step 2 lookup по `domain_names` не нашёл existing proxy-host (вероятно case mismatch), DELETE не сработал, POST на step 5 наткнулся на конфликт. Два фикса defense-in-depth:
  - **Step 2 case-insensitive match**: jq filter теперь `.domain_names | map(ascii_downcase) | index($d_lc)` вместо case-sensitive `index($d)`.
  - **Step 5 POST fallback**: если POST возвращает `400 "already in use"`, script агрессивно re-scan'ит все proxy-hosts (тот же case-insensitive filter), DELETE'ит все конфликтующие по domain, retry'ит POST один раз. Защищает от любых edge-cases (pagination, stale API response, нестандартный format).
  - Применено в flutter_web + go шаблонах + regression fixture.

### Changed
- **Комментарии в шаблонах deploy.yml.tmpl + dockerfile.tmpl переведены на English** (flutter_web + go). Раньше были смешанные русские + английские комменты. Templates генерируют файлы в user-проекты, English однороднее и доступнее. i18n UI-строк в meta.json `description` остаются локализованными `{ru, en}` — это user-facing UI, не комменты в коде.

## [0.13.20] — 2026-04-21

### Fixed
- **B-006: nginx proxy-host update не триггерит regen config**. На первом деплое из app (когда proxy-host уже существовал от ручных NPM UI экспериментов или предыдущих попыток), script создавал новый LE-cert, делал PUT proxy-host с новым cert_id, но NGINX продолжал serve старый cert. Root cause гипотеза: NPM PUT c частичным payload (без `meta`, `locations`, `caddy_config`) не триггерит надёжно nginx config regeneration. Воспроизведено на swan_info_test_app (0.13.19 fix не помогал на этом классе сценариев).
  - **Фикс:** в "needs update" ветке step 2 теперь **DELETE existing proxy-host + POST fresh** вместо PUT-update. Чистый state, guaranteed nginx config regen. Краткий downtime (~5-10s) при recreation, но это first-deploy или broken-state — пользователь и так не имеет работающего https в этот момент.
  - **Early-exit усилен:** проверяется не только cert_id != 0, но и `expires_on != null` у current cert. Orphan certs (которые остались от неудачных попыток) больше не заставляют script early-exit'ить с сломанным конфигом.
  - **HTTP code check:** DELETE и POST теперь проверяют response code и abort'ят с error при failure (раньше силlently продолжали).
  - Применено в flutter_web + go шаблонах + regression fixture swan_support_test_deploy.yml (148 Rust-тестов pass).

## [0.13.19] — 2026-04-21

### Fixed
- **Flutter web template: build-args vs runtime-env collision**. Flutter web — статический build (HTML/JS/CSS подаются nginx'ом), env-vars НЕ подхватываются в runtime. Они должны быть **baked in** на стадии `flutter build` через `--dart-define`. В текущих шаблонах:
  - [`deploy.yml.tmpl`](src-tauri/templates/flutter_web/deploy.yml.tmpl) создавал `.env` файл из секретов (API_BASE_URL / APP_API_KEY), но Dockerfile его нигде не использовал → secrets никуда не доходили.
  - [`dockerfile.tmpl`](src-tauri/templates/flutter_web/dockerfile.tmpl) не имел `ARG` блоков и `--dart-define` — build был без API-конфига, prod-приложение не могло достучаться до backend.
  
  Исправлено:
  - Dockerfile: добавлены `ARG API_BASE_URL` + `ARG APP_API_KEY`, `flutter build web --dart-define=API_BASE_URL=$API_BASE_URL --dart-define=APP_API_KEY=$APP_API_KEY`. Base image `flutter:latest` → `flutter:stable` (предсказуемее).
  - deploy.yml: удалён step `Create .env from secrets` (бесполезный), в `docker/build-push-action` добавлен `build-args:` блок с `API_BASE_URL` и `APP_API_KEY` из secrets.
  - Regression fixture [swan_support_test_deploy.yml](src-tauri/tests/fixtures/swan_support_test_deploy.yml) обновлён под новый формат. test_regression_swan_support_test byte-equal проходит.

### Known limitation
- Имена build-args захардкожены в шаблон flutter_web (`API_BASE_URL` / `APP_API_KEY`). Если Flutter-проект требует другие build-time переменные (`FIREBASE_KEY`, `SENTRY_DSN` и т.п.), user должен руками допилить generated Dockerfile + deploy.yml. Правильное решение — meta.json v4 со `role: build | deploy` на каждый секрет (F-032, v0.15.0).

## [0.13.18] — 2026-04-21

### Changed (template review round 5 — final calibration)

- **O1 Todo id-counter per-prefix** — уточнено: "LLM assigns the next free id — **T- and F- have independent counters**; scan the file for the max used number *within the same prefix*, use +1." Убрана ambiguity "общий счётчик vs per-prefix".
- **O2 D-NNN scope** — "D-NNN exists only inside the app's UI — when referring to a done entry elsewhere (commit message, conversation, other files), use the description text, not D-NNN." Чётче для LLM, который сам вне app UI.
- **O3 Version update files** — Release workflow явно упоминает что "exact file list is in the per-project CLAUDE.md" с примером для Tauri-проекта (package.json + Cargo.toml + tauri.conf.json). Глобальный шаблон не перечисляет, пере-project разный.
- **O4 Historical entries imperative** — "Historical entries must not be modified or removed — done.md is an append-only log." Императив вместо пассивного "are not rewritten".

### Removed
- **`docs/api.md` секция вырезана из global template** — domain-specific (только server-проекты), app не parse'ит. Содержимое сохранено как opt-in addon в [docs/formats/addons-user-only/api.md](docs/formats/addons-user-only/api.md). Симметрично с benchmarks (0.13.15).

Template теперь 145 строк (было 185) — чистый универсальный контракт: todo/done/bugs + SemVer + Changelog. Доменно-специфичные конвенции (api.md, benchmarks.md) живут как user-only addons.

## [0.13.17] — 2026-04-21

### Changed (template review round 4)

- **Q1 bug-reports LLM policy reworked** — вынесена отдельная "### LLM policy for bug-reports.md" секция, которая однозначно разрешает три операции (edit status/comment на существующих строках + delete confirmed строк целиком), и прямо указывает что delete-rule **приоритетнее** правила "edit only two fields". Устранено противоречие между "LLM should delete confirmed rows" и "LLM edits only status and comment".
- **Q2 confirmed cleanup timing уточнён** — "whenever the LLM is about to edit `bug-reports.md` in a session, it first removes any rows currently marked `confirmed` — cleanup bundled into the same edit, not a separate commit." Раньше было "next time LLM opens... as part of the same edit" — туманно.
- **Q3 MAJOR/MINOR bump policy** — "LLM must ask the dev before bumping MAJOR or MINOR. Never auto-decide a MAJOR/MINOR jump. PATCH bumps for bug fixes can be applied routinely." Прошлое "agreed between user and LLM" было пассивной формой без чёткого триггера.
- **Q4 git tag/push — dev'ом** — "Release workflow: LLM updates version in code and records the section in `Changelog.md`. The **dev** then runs `git tag vX.Y.Z` and `git push --tags` — LLM does not execute git tag/push autonomously." Явно кто что делает.
- **Q7 [Unreleased] дата** — добавлено правило: "`## [Unreleased]` section **never has a date** — the date is added only when the version is tagged/released (at which point `[Unreleased]` becomes `[X.Y.Z] — YYYY-MM-DD`)."

### Removed
- **Q6 `docs/benchmarks.md` секция вырезана из global template** — Go-specific, не enforce'ится app'ом, для большинства tool/client проектов нерелевантна. Содержимое сохранено как opt-in addon в [docs/formats/addons-user-only/benchmarks.md](docs/formats/addons-user-only/benchmarks.md) — user сам вставит в `~/.claude/CLAUDE.md` если нужно.

## [0.13.16] — 2026-04-21

### Fixed (template review, contradiction)
- **N2 redone** — в 0.13.15 я написал "LLM must write only values from this set" для `category`, что прямо противоречило основной policy "LLM is allowed to edit only `status` and `comment`". Удалил. Теперь category-поле просто помечено `User-owned`, как и severity/description. Упоминание нормализации оставлено как описание внутренней safety-net (НЕ как разрешение LLM использовать). Заодно убрал дубликат "User-owned — LLM does not edit" из description (redundant с policy-строкой ниже) — все три user-owned поля теперь в единой лаконичной форме.

## [0.13.15] — 2026-04-21

### Changed (template review round 3 — finish)
- **N4 Changelog: `Deprecated` + `Security` категории добавлены**. Template теперь полный Keep-a-Changelog набор (6 типов: Added/Changed/Deprecated/Removed/Fixed/Security). В преамбуле — описание каждой категории с примером использования. Добавлена note: "Empty categories may be omitted from a released version."
- **N2 Bug category clarification**: добавлено явное правило для LLM — "LLM must write only values from this set — do not invent new categories. The app normalises unknown values to `other` on load as a safety-net, not as a permission (ugly values still show up in git diffs and distort stats until the app next saves the file)."

## [0.13.14] — 2026-04-21

### Changed (template review round 3)

- **Todo `id` mandatory** — в описании поля явно: "Always required — never leave empty. LLM assigns next free id by scanning the file for max used number +1" (N13).
- **Todo `effort` fallback** — "Required. Use `0` if unknown." (N6).
- **Done empty-id slot пример с explicit wording** — "`- | Description | v1.0` — markdown dash, space, empty slot, first pipe" (N7).
- **Done ordering правило** — добавлено "new entries appended at the bottom of the current `## <date>` section (chronological within the day)" (N14).
- **Bug `created → in-progress` явно LLM-triggered** (N3) — "LLM transitions created → in-progress when it takes the bug into work."
- **Bug `testing` транзиция явно бампает fix_attempts** (дубль для clarity в status-блоке, было только в fix_attempts описании).
- **Bug `confirmed` cleanup timing уточнён** (N9) — "the next time the LLM opens `bug-reports.md` for work, it should delete rows with status `confirmed` as part of the same edit."
- **API admin-matrix footnote** (N1) — добавлено пояснение: "Admin has `auth-write: ✗` by design — all admin writes go through dedicated `admin-write` endpoints."
- **Убрано лишнее "the rule never triggers..."** в escape-правилах (N5) — жёстче, без эмпатии.
- **SemVer `dev/LLM` унифицировано** (N8) — "user (acting as the dev) and the LLM".
- **Benchmarks — убраны конкретные SwanQu-значения** (N12) — "Targets are project-specific — fill with your actual SLOs."
- **Benchmarks 10–15% → точное правило** (N10) — "> 10% flags for review; > 15% blocks release."
- **Changelog дубликат ссылки убран** (N11) — Keep-a-Changelog упомянут в тексте один раз, в примере-template'е строка про формат удалена.

### Changed (code)

- **`parse_done_tasks`: 2-поля отвергаются** — warning вместо молчаливого auto-id (R5 из прошлого раунда, но был закоммичен без явного теста; теперь есть `test_parse_done_two_fields_rejected` + `test_parse_done_three_fields_empty_id_accepted`).

## [0.13.13] — 2026-04-21

### Changed (template review round 2)

Вторая итерация ревью `claude.md.global.tmpl` + синхронизация с кодом:

- **Bug workflow — две точки истины синхронизированы** (R1). Старая формулировка "rejected → attempts +1" в [`bug-reports.md.tmpl`](src-tauri/templates/_global/bug-reports.md.tmpl) и header от [`generate_bug_reports`](src-tauri/src/export.rs) — перезаписаны на фактическое поведение: "incremented each time status enters `testing`". Теперь глобальный шаблон + seed-скелет + runtime-генератор пишут одно и то же.
- **`confirmed` workflow уточнён** (R2) — user ставит status=confirmed через ✓, строка **остаётся** в файле. LLM на следующем cleanup-проходе удаляет confirmed-строки. Раньше было неточно сказано "row is deleted from file by the user" — на самом деле удаление = отдельное действие.
- **LLM не создаёт bug rows** (R3). Явно запрещено в обоих шаблонах. Новые баги — только через app "+ Add bug" (иначе id/date/DB-stats разъедутся). Если LLM находит баг во время работы — просит user'а завести, либо добавляет T-NNN в todo.md.
- **`created` — явно app-assigned** (R4). Добавлено в status-описание: "set by the app on creation. LLM only transitions from here."
- **Todo: `review` status добавлен** (R9) — для задач "implementation done, awaiting self/peer review". Парсер enum-agnostic, не требует изменений.
- **Todo id — LLM-assigned** (R10) — явно: "LLM assigns the next free id; user may also assign manually. No app-side counter."
- **Done policy переписан** (R6) — убрано противоречивое "append-only in practice". Теперь: "LLM or user moves completed tasks here."
- **Done `version` переформулирован** (R7) — "SemVer tag recommended. Any free-form string accepted. May be empty." (раньше было противоречивое "SemVer tag... or free-form").
- **Done пример чище** (R11) — убран двойной пробел в `-  |` (выглядело как опечатка), теперь `- |` с явным комментарием про empty id slot.
- **Escape scope расширен** (R12) — `version` добавлено в список free-form полей, к которым применяется экранирование `|`/`\n`.

### Fixed

- **parse_done_tasks: rejects 2-field lines** (R5). Раньше парсер толерантно принимал `- description | version` (без pipe для id); это противоречило шаблону "3 поля всегда, id может быть пустым". Теперь 2-поля → warning. Legacy 4-поля (`id | desc | date | commit`) пока оставлены для migration transition. Тесты обновлены: `test_parse_done_two_fields_rejected` + `test_parse_done_three_fields_empty_id_accepted`.

## [0.13.12] — 2026-04-21

### Changed (ревью глобального шаблона)
- **[`claude.md.global.tmpl`](src-tauri/templates/_global/claude.md.global.tmpl) переработан** по итогам ревью на однозначность + согласованность с кодом:
  - **severity `trivial` убран** — в UI BugItem его и не было (4 градации: critical/major/medium/minor)
  - **`VB-NNN` убран** из описания bug id — app их не генерирует, только парсит legacy (для читаемости новых проектов не нужно)
  - **`confirmed` убран из enum-values для записи** — это terminal state (row deleted), не значение которое LLM пишет в файл. Workflow: `created → in-progress → testing`, терминальные `confirmed` (удаление) / `rejected` (+1 attempt)
  - **`fix_attempts` формулировка уточнена** — "incremented each time status enters `testing` (= count of fix attempts made)" вместо неточного "on rejected"
  - **Done формат однозначен** — "3 поля всегда, id slot может быть пустым (pipes присутствуют)". Убрал дву́смысленное "2-field fallback"
  - **Escape rules вынесены в header** и уточнены — применяются только к free-form полям (`description`, `comment`); enum и id не содержат `|`
  - **Todo/done policy унифицирован** — LLM и user могут двигать задачи todo→done (раньше todo было "user-owned", противоречило)
  - **`api.md` статус `in-progress` добавлен** — между `planned` и `implemented` для эндпоинтов в разработке
  - **SemVer note** — "MAJOR/MINOR bumps agreed between user and dev" (явно, чтобы не автоинкрементилось)
  - Pipe-example для todo заменён на новый enum: `high`/`medium` вместо `must`/`should`

### Fixed
- **[priorityClass](src/lib/components/RepoDocsTab.svelte) обновлён** на новый todo priority enum: `critical`/`high` → pri-high; `medium` → pri-medium; остальное → pri-low. Старые значения (`must`, `высокая`, `средняя`) убраны. Проекты со старым todo.md визуально сваливаются в pri-low, пока не мигрируют.
- **[bugs.ts `addBug` default category](src/lib/stores/bugs.ts)**: `'unknown'` → `'other'` (которое реально в enum'е). Новые баги создаются с валидной категорией, в UI dropdown подсвечен.
- **[bugs.ts loadBugsFromFile](src/lib/stores/bugs.ts) normalizer**: если `category` в файле не из canonical set (`ui_ux / ux_flow / logic / auth / database / performance / security / integration / other`), на загрузке подменяется на `other`. Это убирает `unknown` из legacy файлов и любой шум от чужих категорий.

## [0.13.11] — 2026-04-21

### Added
- [`claude.md.global.tmpl`](src-tauri/templates/_global/claude.md.global.tmpl) расширен 4 новыми контрактами:
  - **Versioning (SemVer)** — правила bump MAJOR/MINOR/PATCH + таблица по project type (server/client/microservice/tool) + git-tag workflow
  - **Changelog.md** — Keep a Changelog структура
  - **`docs/api.md`** (server only) — access levels / matrix / endpoints group / statuses
  - **`docs/benchmarks.md`** (Go server only) — target metrics table / run-rules / results log format
- Всё на English (универсальный контракт). LLM читает правила единожды из глобального CLAUDE.md, в каждом проекте только специфичный context.

## [0.13.10] — 2026-04-21

### Changed
- **Разделение CLAUDE.md шаблона на два:**
  - [`claude.md.section.tmpl`](src-tauri/templates/_global/claude.md.section.tmpl) — теперь только project-context (имя проекта, репо, микросервисы, parents). Идёт в **локальный** `<repo>/CLAUDE.md` при Sync проекта. Правила форматов убраны — не место.
  - [`claude.md.global.tmpl`](src-tauri/templates/_global/claude.md.global.tmpl) **(новый)** — правила форматов `docs/todo.md` / `docs/done.md` / `docs/bug-reports.md` + LLM policy + escape rules. Идёт в **глобальный** `~/.claude/CLAUDE.md` через кнопку "Sync global CLAUDE.md" в Settings → AppDefaults. Без project-плейсхолдеров — contentful spec что парсеры реально принимают.
  - Раньше был один шаблон, использовался в обоих местах: в глобальный шли "—" плейсхолдеры (мусор), в каждом локальном CLAUDE.md дублировались правила. Теперь чистое разделение context vs rules.
- **docs/formats/todo.md** синхронизирован с глобальным шаблоном: `effort = hours`, `priority = critical/high/medium/low` (раньше было `S/M/L` + `must/should/could`). Парсер enum-agnostic, ест любые строки — спека теперь рекомендация, не constraint.

### Added
- Rust функция [`sync::update_claude_md_global`](src-tauri/src/sync.rs) для записи глобального блока (без project-контекста). Общий helper `write_claude_section` для обоих сценариев — маркер-replace/append логика единая.

## [0.13.9] — 2026-04-21

### Changed
- **`docs/done.md` v2 формат.** Переход от 4 полей (`id | desc | date | commit` с `[x]` checkbox) на 3 поля (`id | desc | version`) + дата из section-header `## YYYY-MM-DD`. Убрана `[x]` галочка — файл и есть "done", отмечать каждую строку избыточно. Парсер:
  - Читает дату из ближайшего `## <...date...>` (toлерантно к `DD.MM.YYYY`/`DD/MM/YYYY` + legacy headers типа `## День 29.03.2026`)
  - Приним 2-3 поля per line
  - Пустой id → auto-assign `D-NNN` (in-memory only, file не переписывается)
  - Legacy 4 поля с `[x]` тоже принимаются — inline date игнорится (section-header wins), commit → version slot
- **Модель**: `DoneTask.commit` → `DoneTask.version`. UI колонка "Коммит" → "Версия" (i18n `tasks.colCommit` → `tasks.colVersion`).
- **Спека** [docs/formats/done.md](docs/formats/done.md) переписана с учётом новых правил.

### Removed
- **F-028 Legacy v1 bug parser** убран из `parse_bug_reports` — user завершил миграцию через кнопку "⬇ v2" в 0.13.8. 10-полевые строки теперь выдают warning, как раньше планировалось в v0.14.0. F-028 закрыт раньше срока.
- Кнопка "⬇ v2" в BugNotes + связанные i18n ключи (`bugNotes.resave*`) удалены — служили одной цели (миграция), задача выполнена.
- Тесты `test_parse_legacy_v1_merges_screen_and_reproduction`, `test_parse_done_date_anchor_version_prefix`, `test_parse_done_3_fields_no_commit` удалены (устарели). Добавлены новые: `test_parse_done_v2_section_header_date`, `test_parse_done_auto_id_when_empty`, `test_parse_done_two_fields_no_id`, `test_parse_done_legacy_header_extracts_date`, `test_parse_done_no_section_header`.

## [0.13.8] — 2026-04-20

### Added
- **F-030 Changelog tab** в RepoDetail. Пятая вкладка между Tasks и Secrets (порядок: Bugs → Tasks → Changelog → Secrets → Stats). Читает `Changelog.md` из local_path, рендерит как preformatted text с sans-serif шрифтом и wrap'ом. Если файла нет — показывает подсказку "не найден в корне репозитория". Эффорт S: ~40 строк Svelte + 1 i18n + хук в RepoDetail. Использует существующую Tauri-команду `read_repo_file`.
- **"⬇ v2" кнопка в BugNotes** — форсированное пересохранение `bug-reports.md` в текущем v2 формате, даже без явных правок. Инструмент миграции для T-041: legacy 10-полевые файлы (v1, до 0.13.0) одним кликом превращаются в 8-полевые (v2), без необходимости мучать статус/комментарий ради триггера save. Парсер на load всегда приводит к v2 in-memory; кнопка просто принудительно flush'ит на диск.

### Docs
- `docs/formats/REVIEW-2026-04-20.md` — unified source-of-truth review для форматов todo/done/bug-reports. Собирает то, что *на самом деле* принимают парсеры и генерируют writer'ы, плюс расхождения с глобальным `~/.claude/CLAUDE.md` (там v1 bug-format + русские enum'ы в todo). Блокер для F-029 datatable.
- В `docs/todo.md` добавлены T-040 (format alignment), T-041 (manual migration всех bug-reports.md), T-042 (changelog-tab ревью).

## [0.13.7] — 2026-04-20

### Fixed
- **B-002 (4th retry) — корень найден: `each_key_duplicate`.** В RepoDocsTab ключи `{#each}` были `t.id + '|' + t.date` (done) и `t.id` (todo). Когда в `docs/done.md` несколько строк идут под одной версией и той же датой (`v0.10.0 | ... | 17.04.2026 | ...` ×N), композитный ключ совпадает у ≥2 элементов. Svelte 5 в production-build throw'ит `each_key_duplicate`, и рендер-фаза падает — state уже обновлён (`doneLoading=false`, `doneTasks=106`), но UI застревает на предыдущем DOM-снимке ("Загрузка..."), потому что применить новый render Svelte не смог. Диагностика через console.log + timeout (0.13.6) дала точный стектрейс.
- **Фикс**: ключи `{#each doneTasks as t, i (i)}` / `{#each todoTasks as t, i (i)}` — index-based. Tasks отображаются read-only, cross-list identity не нужна, дубликаты в источнике больше не ломают рендер.
- **Оставлен timeout 10s** из 0.13.6 как safety-net — если когда-нибудь IPC реально зависнет, user увидит явную ошибку вместо вечного спиннера. `console.log`-диагностика убрана из релиза.

## [0.13.6] — 2026-04-20

### Diagnostic (B-002 retry#4)
- В RepoDocsTab добавлен timeout 10s на `readRepoTodo` / `readRepoDone` (Promise.race) + `console.log`/`console.error` на каждый этап. Если IPC-вызов реально висит — через 10s UI покажет "readRepoDone: timeout after 10000ms" красным, и в devtools консоли будет виден последний достигнутый лог. Это даст точные данные для следующей итерации фикса.
- Добавлены regression-тесты против реального `docs/done.md` + `docs/todo.md` этого репо (`include_str!`) + тест serde-сериализации `ReadDoneResult`. Все проходят — 106 задач, JSON 23KB, 0 warnings. Значит baseline Rust-части чистый, баг где-то в Tauri IPC / webview-state на стороне пользователя.

## [0.13.5] — 2026-04-20

### Fixed
- **B-002 (3rd retry)** В 0.13.4 Done всё ещё показывал "Загрузка..." при открытии, а Todo работал. Оба завязывались на один shared `loading` + `Promise.allSettled` — но пользовательская среда как-то видела разные значения в разных `<details>`. Разнесли: `todoLoading` / `doneLoading` + `loadTodo()` / `loadDone()` запускаются параллельно независимо, ошибка одного не блокирует другой. Добавлены `todoError` / `doneError` с выводом сообщения красным — если что, сразу видно почему застряло.
- Счётчики в заголовках Todo/Done упрощены: вместо `openCount / inProgressCount` с English-only filter (`status === 'open'`) теперь total `todoTasks.length` / `doneTasks.length`. Проекты с русскими статусами (`открыта`) больше не показывают 0 при непустом списке. Бонус: badge-оформление счётчика (`surface` фон + border-radius).

### Deprecated
- i18n ключи `tasks.openCount` / `tasks.inProgressCount` больше не используются в рендере (оставлены в словаре на случай отката).

## [0.13.4] — 2026-04-20

### Fixed
- **B-002 (2nd retry)** Svelte `$state` + `<button>` + `{#if expanded}` → native `<details>`/`<summary>`. У пользователя в 0.13.3 Todo всё ещё работал, Done — нет, при одинаковом коде (загадка уровня "не воспроизводится"). Перевод на нативный HTML-элемент убирает всю Svelte-реактивность из пути клика: браузер сам ставит атрибут `open`, мы только стилизуем. Todo и Done теперь гарантированно идентичны, а заодно получили бесплатно правильную keyboard-доступность (Enter/Space/click). Заголовки-треугольники `▾` повёрнуты через CSS-селектор `.card-details[open] .chevron`.

### Fixed
- **B-002 (rejected → retry)** Done-секция в RepoDocsTab не тогглилась по клику. Прежний inline-хендлер `onclick={() => (doneExpanded = !doneExpanded)}` возвращал новое значение boolean'а — это и не должно было мешать, но браузер/webview мог неожиданно трактовать `false`-return. Вынесли оба переключателя (`toggleTodo` / `toggleDone`) в именованные функции с явным block-body (ничего не возвращают). Добавлены `data-section="todo"` / `data-section="done"` для devtools-диагностики при регрессе.
- **B-003 (rejected → retry)** Убран таб-в-табе collapse в Stats и Secrets:
  - **Stats**: удалён `statsCollapsed` + кнопка `▶/▼`. Таблица и кнопка "Reset" показываются сразу при переключении на таб. Заголовок `Stats` оставлен как статический `.stats-title`.
  - **Secrets** (SecretsPanel): добавлен prop `collapsible: boolean` (default `true`, обратная совместимость с ProjectDetail). RepoDetail передаёт `collapsible={false}` — header-toggle не рисуется, тело всегда открыто. Добавлен CSS-модификатор `.secrets-section.flat` (убирает border-top и vertical padding, т.к. tab-wrapper уже предоставляет отступы + tab-nav имеет разделитель).

### Fixed
- **B-001** Scroll в Secrets/Stats табах RepoDetail. Было: `flex-shrink:0` → длинный список секретов / большая stats-таблица обрезались нижней частью viewport, без возможности проскроллить. Стало: `flex:1; min-height:0; overflow-y:auto` на `.secrets-wrapper` + `.stats-section` — консистентно с уже работающими Bugs/Tasks.
- **B-003** Порядок табов в RepoDetail: **Bugs → Tasks → Secrets → Stats**. Default tab = `'bugs'` (раньше `'overview'`). Таб переименован "Обзор" → "Секреты" — фактически держал только SecretsPanel (остальное репо-info живёт в sticky-header выше tab-nav). `Tab` type + `activeTab` default + внутреннее значение `'overview'` → `'secrets'`. i18n ключ `repo.tabOverview` → `repo.tabSecrets`.
- **B-004** Category `ux_flow` добавлен в список для роли `tool` в BugItem dropdown. Раньше tool-репо (как сам этот проект) не мог ставить тег "UX Flow" багу, хотя имеет UI/UX флоу как и клиенты. Теперь `tool: ['ui_ux', 'ux_flow', 'logic', 'database', 'performance', 'other']`.

### Verification needed
- **B-002** "Done секция не раскрывается" — код Done структурно идентичен рабочему Todo (`$state`, onclick toggle, `{#if doneExpanded}` на body). Гипотеза: симптом остаточного B-001 scroll-бага в соседнем контексте или устаревший build у пользователя. После установки 0.13.2 и фикса B-001 нужна пере-проверка.

## [0.13.1] — 2026-04-20

### Changed
- **F-022 DeployManifest extras persisted (migration v15)** — non-core placeholders (ENV_FILE_PATH, ENTRY_POINT, GO_VERSION, BINARY_NAME, APP_PORT) теперь сохраняются в `deploy_manifests.extras` как JSON-map. До этого session-only — пропадали при закрытии экрана. Load-priority: manifest.extras > auto_detect > default. Save: дебаунс 400ms, теперь триггерится на ЛЮБОЕ поле (раньше только core-5). Пустое значение extras очищает ключ — fallback к auto_detect на следующем load.
- **F-022 Go: `ENV_FILE_PATH` → placeholder, не secret** — путь к .env на сервере не чувствителен (не credentials), зашивается в deploy.yml по общему паттерну с `ENTRY_POINT`/`APP_PORT`. Поле появляется в DeployScreen, default пустой. В generated deploy.yml шаблон подставляется в `ENV_FILE="@@ENV_FILE_PATH@@"` и bash-условие `if [ -n "$ENV_FILE" ] ...` добавляет `--env-file` только когда непусто. `ENV_FILE_PATH` удалён из `required_secrets`.
- **Secrets: normalize line endings перед шифрованием** — новая функция `normalizeSecretValue`: для single-line значений strip'ает `\r` (Windows clipboard trailing CR) и trim'ит; для multi-line (содержит `\n`) конвертирует `\r\n → \n`, убирает одинокие `\r`, обрезает крайние пустые строки/пробелы, добавляет один trailing `\n` (PEM-friendly). Классический кейс: SSH_KEY вставляется из Windows буфера с `\r\n` между строк → appleboy/ssh-action пишет ключ в temp файл, OpenSSH ругается "attempted methods [none publickey], no supported methods remain". Теперь шифруется чистый LF-формат. +6 vitest-тестов для edge cases.
- **Secrets: пост-push верификация** — после `createOrUpdateRepoSecret` перечитываем `listRepoSecrets` и проверяем что каждый отправленный name реально появился в списке. Если GitHub вернул 201, но секрета нет — показывается честный error toast с именами "потерянных" ("GitHub подтвердил отправку, но при перечитывании списка этих секретов нет: X, Y"). Решает случай silent-fail когда API статус 2xx, но секрет не сохранился (чаще всего из-за скоупа PAT — нужен `repo` + Secrets: write).

- **F-025 Reorder: per-row ▲▼ убраны** — hover + focus-within делали кнопки "залипшими" после клика. Заменено на одну пару ▲▼ в header Sidebar; целью служит currently-selected item (`selectedRepoId`, иначе `selectedProjectId`). Tooltip подписывает именем ("Переместить «swan_support_test» выше (с wrap в конец)"); при пустой selection кнопки disabled с подсказкой "Выберите репо или проект". Серия ▲▲▲ теперь гарантированно двигает один item без перескока курсора — кнопка не переезжает.
- **F-025 Post-reorder highlight** — после клика ▲/▼ переместившийся item подсвечивается как при hover (background-hover для репо, accent-цвет для проекта), чтобы было видно куда он уехал (критично при wrap-around на границах). Подсветка снимается когда мышь заходит на другой элемент списка; оставляется если пользователь не двигает мышь. Item также скроллится в видимую область после wrap'а (`scrollIntoView({ block: 'nearest', behavior: 'smooth' })`).
- **Sidebar-width 280 → 320px** — чтобы header с ⊞ ⊟ 🔤 ▲ ▼ + 3 action-кнопки не сжимал tooltips и не обрезал длинные имена проектов.

### Fixed
- **Nginx deploy: cert не подхватывался после создания** — NPM API возвращает `certificate_id` сразу после POST, но LE-выпуск асинхронный; PUT proxy-host мог пройти с ещё невалидным cert. Теперь ищем в `/api/nginx/certificates` только certs с `expires_on != null` (= фактически выпущенные), а после создания новго poll'им `GET /api/nginx/certificates/<id>` до заполнения `expires_on` (≤90s). Флаттер и Go шаблоны + regression fixture обновлены.
- **Go deploy: потерян `--env-file`** — восстановлено поведение оригинального `SwanQu/swanqu_server`. Путь к .env реализован как placeholder `ENV_FILE_PATH` (см. Changed выше). В SSH-скрипте bash-условие добавляет `--env-file $ENV_FILE` к `docker run` только если значение непустое — проекты без .env не заполняют поле, deploy работает без флага.
- **F-021 парсер `done.md`: "текст в поле дата"** — не-канонические строки (напр. `id | version | description | date`) сдвигали колонки. Введён date-anchor: парсер находит поле формата `YYYY-MM-DD` / `DD.MM.YYYY` / `DD/MM/YYYY` и центрирует парс относительно него. Всё до якоря после `id` — description; всё после — commit.
- **F-021 парсер `todo.md`: скрывал валидные строки** — требовал ровно 5 полей, пропускал 3/4/6-полевые варианты. Теперь толерантен: status всегда последнее поле, id — первое; при 6+ полях middle-slots склеиваются в description.
- **F-021 RepoDocsTab: одна ошибка обнуляла обе секции** — `Promise.all` → `Promise.allSettled`, todo и done теперь независимы.
- **F-025 Auto-sort сортировал по id (de-facto inverse alphabetical)** — заменено на алфавитное: проекты по `LOWER(name)`, репо по role-priority → `LOWER(COALESCE(github_name, description, ''))` внутри группы. Case-insensitive, стабильный tie-breaker по id.
- **F-022 Go Dockerfile: WORKDIR/-o name collision** — `WORKDIR /app` в builder + `go build -o /@@BINARY_NAME@@` с дефолтом `BINARY_NAME=app` = коллизия: `/app` уже директория, go build пишет бинарь внутрь неё под именем пакета (`./cmd/api/` → `/app/api`). `COPY` тянул директорию, `CMD ["./app"]` падал с `exec: "./app": stat ./app: no such file or directory`. Исправлено: builder использует `WORKDIR /src`, бинарь пишется в `/out/<binary>`, runtime `WORKDIR /app` + `COPY /out/<binary> ./`. Regression-тест усилен anti-assertion на старую схему.
- **F-021 RepoDocsTab: нет вертикального скролла** — `.docs-tab` не имел `flex:1; min-height:0; overflow-y:auto`, поэтому при раскрытой секции todo секция done уходила за viewport и была не достижима. Пользователь воспринимал это как "done не раскрывается". Теперь таб скроллится стандартным паттерном, как уже работает BugNotes.

## [0.13.0] — 2026-04-19

### Added
- **F-022 Go deploy template** — новый bundled-язык `go` в `src-tauri/templates/go/` (meta.json + deploy.yml.tmpl + dockerfile.tmpl). 9 placeholders (5 core + GO_VERSION/BINARY_NAME/ENTRY_POINT/APP_PORT), 7 required_secrets (SSH + NPM). Nginx-job идентичен flutter_web. Reference: `SwanQu/swanqu_server`.
- **meta.json schema v3** (rich-object placeholders) — `placeholders` теперь `{ label, description, default, type, auto_detect? }` с локализованными label/description (`{ru, en}`). `auto_detect` — pre-fill значения из файла репо по regex (GO_VERSION из go.mod). Loader поддерживает и v3, и legacy v1/v2 string-dict.
- **DeployScreen dynamic form** — форма рендерится из meta.json placeholders динамически (вместо hardcoded 5 полей). Go видит 9 полей, flutter_web — 5 (прежний опыт). Core-5 persist в DeployManifest, extras session-only через `extra_vars` параметр `render_deploy_files`.
- **Tauri команда `read_repo_file(repo_id, rel_path)`** — чтение произвольного файла из local_path репо. Используется auto_detect для go.mod, будет переиспользована в F-021.
- **F-025 Manual ordering** — migration v14 добавляет `sort_order INTEGER` в `projects` и `repositories`. Initial populate: projects по `id * 10`, repos по `role_priority * 1000 + id * 10`. Кнопки ▲▼ при hover в Sidebar с wrap-around на границах (▲ на первом → в конец, ▼ на последнем → в начало). D&D reorder внутри группы через always-rebalance (single CASE UPDATE). Cross-project drop кладёт репо в конец целевой группы. Кнопка `🔤 Auto-sort` в header (destructive, с ConfirmDialog) — сброс к initial formula.
- **Tauri команды** `reorder_project`, `reorder_repo`, `rebalance_repo_group`, `rebalance_projects`, `auto_sort_all`.
- **F-021 Docs viewer** — Tabs-layout в RepoDetail (Overview/Bugs/Tasks/Stats). `RepoDocsTab.svelte` — read-only просмотр `docs/todo.md` (5-полевой формат) и `docs/done.md` (4-полевой формат) с табличным выводом и мини-статистикой. Новые Rust парсеры `parse_todo_tasks` / `parse_done_tasks` (переиспользуют helper `split_pipe_respecting_escape`) + Tauri команды `read_repo_todo` / `read_repo_done`.
- **F-026 Bug format v2** — bug-reports.md сокращён с 10 до 8 полей: убраны `screen` и `reproduction` (фактически не использовались в UI). Новый формат: `id | date | description | severity | category | status | fix_attempts | comment`. Pipe-escape (`\|`), newline-escape (`\n`).
- **F-026 Legacy v1 parser** — двуглавый parse_bug_reports принимает и 10-полевой (screen + reproduction merged в description), и 8-полевой форматы. Generate всегда пишет v2. Transparent migration при первом save.
- **F-026 LLM-policy** — в `docs/formats/bug-reports.md` и в header bug-reports.md.tmpl зафиксировано: LLM правит только `status` и `comment`. Остальные поля user-owned / immutable / app-managed.
- **docs/formats/** — 5 формальных спек (bug-reports v2, todo, done, claude-md-section, project-md) с полями, типами, escape-правилами, LLM-policy, примерами.

### Changed
- **DeployScreen** — hardcoded 5-field form заменён на dynamic из meta.json. `flutter_web/meta.json` upgraded до v3 без изменения render-результата (regression fixture byte-equal сохранён).
- **render_deploy_files** — новый параметр `extra_vars: Option<HashMap>` для session-only overrides от UI. Priority: extras > manifest core > meta.json default.
- **Sidebar** — `sortedProjects` / `sortReposByRole` derived-сорты удалены. Источник правды — Rust `ORDER BY sort_order ASC, name ASC`. Role-priority не применяется после ручной сортировки (только как initial default).
- **assign_repository** — cross-project move теперь сбрасывает sort_order на MAX группы + 10 (ре движется в конец target).
- **RepoDetail** — layout разбит на 4 tab'а (Overview/Bugs/Tasks/Stats).

### Removed
- **`freshProjectIds`** session-only Set в projects.ts — заменён persist-логикой в БД (`create_project` → sort_order = MIN - 10, новый проект в начале списка).
- **Поля `screen` и `reproduction`** из `FileBugNote` struct и `BugNote` TS interface.

### Deprecated
- **Legacy v1 bug parser** (10-полевой) — останется в v0.13.x для transparent migration существующих файлов. Удаляется в v0.14.0 (F-028). User должен пройтись по всем проектам и force-save bug-reports.md для конверсии к v2 до upgrade на v0.14.0.

### Breaking (transparent)
- `bug-reports.md` при первом save через app конвертируется из 10-полевого формата в 8-полевой (screen + reproduction склеиваются в description). User увидит одноразовый git-diff — данные сохранены.

### Docs
- Новые спеки: `docs/formats/{bug-reports, todo, done, claude-md-section, project-md}.md`.
- Новый flow doc: `docs/flows/manual-ordering.md` — концепция sort_order, операции reorder/rebalance/auto-sort.

## [0.12.0] — 2026-04-19

### Added
- **F-024 Sidebar UX**: кнопки "⊞ развернуть все проекты" и "⊟ свернуть все проекты" в header sidebar'а (рядом с `+`, `📁`, `⟳`). Все проекты свёрнуты по умолчанию при первом запуске. Состояние collapsed/expanded для каждого проекта + unassigned-группы персистится в settings-таблице SQLite (`sidebar_collapsed_projects` = JSON `{projectId: bool}`, `sidebar_unassigned_collapsed` = bool).
- **F-023 About screen**: отдельный экран "О программе" — кнопка `ℹ About` в titlebar после Settings. Полноразмерный логотип (256×256), версия из `package.json`, credits "Сгоннов Д.А. / AI Claude" / "Sgonnov D.A. / AI Claude", кликабельная ссылка на GitHub-репо проекта (открывается в системном браузере через `@tauri-apps/plugin-opener`).
- **B-007 merge local-only ↔ GitHub**: новый enum `UpsertRepoOutcome` с вариантами `Inserted` / `Merged` / `Ambiguous`. `upsert_repository_with_outcome` при sync с GitHub теперь ищет в БД local-only записи (`github_name IS NULL`), у которых basename(local_path) совпадает с именем GitHub-репо (case-insensitive). При 1 матче — UPDATE существующей записи вместо INSERT новой (устраняет дубль). При 2+ матчах — `Ambiguous` outcome, frontend показывает `MergeChoiceDialog` с выбором: слить с конкретным (по полному local_path) / создать новую запись / пропустить. Новые команды `resolve_merge_with_local` + `force_insert_github_repo`.
- 6 новых db.rs тестов для B-007 (`test_b007_merges_single_local_only_by_basename`, `test_b007_ambiguous_when_multiple_local_match`, `test_b007_inserts_when_no_local_match`, `test_b007_basename_match_is_case_insensitive`, `test_b007_resolve_merge_with_local_updates_chosen`, `test_b007_force_insert_creates_new_entry`) → 118 passing total.
- Новый компонент `src/lib/components/MergeChoiceDialog.svelte` — модалка выбора для ambiguous merge'а с radio-списком кандидатов (показывает полный local_path для различения однотипных basename'ов).
- Новый store `pendingMergeCases: writable<AmbiguousMergeCase[]>` в `repos.ts` — очередь ambiguous-случаев, consumer — +page.svelte.
- 14 новых i18n ключей × ru/en (7 about.*, 2 sidebar.expandAll/collapseAll, 5 merge/toast).

### Fixed
- **B-007**: создание local-only репо из папки с именем, совпадающим с существующим GitHub-репо пользователя, больше не создаёт дубль при последующем `Sync from GitHub`. Local-запись обновляется (обогащается полями github_name/github_url/github_id), сохраняя local_path и project_id.
- **B-001**: `_global` язык-шаблонов больше не отображается в deploy-target dropdown RepoDetail. Placeholder `repo.deployTargetNone` локализован ("— без шаблона —" / "— none —").
- **B-002**: Суффикс `.tmpl` не отображается в списке файлов TemplateEditor и в file-path крошке (убран в UI; бэкенд-ключ `file_name` не тронут).
- **B-003**: Всё содержимое managed-вставок в MD-файлах теперь на английском — 3 `_global/*.tmpl` (claude.md.section, todo.md, bug-reports.md), `generate_project_md` и `update_claude_md_section` fallback-строки в sync.rs, header `generate_bug_reports` в export.rs. Мотивация: LLM проще парсить унифицированный английский, смена UI-языка не меняет содержимое файлов (сохраняется идемпотентность marker-блоков).

### Removed
- **Export path setting** (фича-зомби): удалён раздел `Settings → Export path`, store `exportPath`, команда `saveExportPath`, ключ `export_path` в SQLite settings-таблице и 9 i18n-ключей. Функционал никогда не использовался.

### Changed
- `bug-reports.md.tmpl` skeleton переведён на английский и приведён к 10-полевому pipe-формату парсера (id | date | screen | description | reproduction | severity | category | status | fix_attempts | comment).

### Changed
- Сигнатура Tauri команды `upsert_repository` — возвращает `UpsertRepoOutcome` вместо `Repository`. Frontend `syncFromGitHub` различает варианты и показывает toast / queue-ит ambiguous.
- `Sidebar.svelte`: `unassignedCollapsed` default изменён с `false` на `true`. Added `persistCollapsed()` вызов при каждом toggle.

### Docs
- Roadmap: F-023, F-024, B-007 → ✅ 0.12.0. F-018 autoupdater + F-019 full rebrand сдвинуты в v0.13.0.
- bug-reports.md: B-007 → статус `testing` + комментарий о фиксе (удаление — после `confirmed` от пользователя).

## [0.11.0] — 2026-04-17

### Added
- **F-016 Docs initialization**: skeletons `docs/todo.md` и `docs/bug-reports.md` copy-if-missing при `sync_project` pre-phase (расширение 0.10.0 auto-files pattern). Bundled шаблоны в `_global/todo.md.tmpl` + `bug-reports.md.tmpl` — seeder подхватывает автоматически через `include_dir!`, user редактирует в AppDefaultsScreen.
- Команда `init_docs_for_repo(repo_id)` + кнопка "📚 Инициализировать документацию" в RepoDetail — per-repo on-demand trigger. Обрабатывает 3 файла: `docs/todo.md` и `docs/bug-reports.md` (copy-if-missing), `.gitignore` через **merge-logic** — добавляет блок `# --- solo-dev-hub:begin … :end ---` к существующему `.gitignore` если его нет, либо обновляет содержимое блока. User-контент снаружи блока сохраняется.
- Функция `sync::sync_gitignore_section` заменяет `copy_gitignore_if_missing` в sync pre-phase — **dedup merge**: для каждой rule-строки из шаблона проверяется exact-match с user-правилами (вне блока); в блок `# --- solo-dev-hub:begin … :end ---` попадают только те правила, которых у пользователя нет. Если все template-правила уже у user — блок не создаётся (или удаляется).
- **F-020 Solo Dev Hub branding**: новая app-иконка. `icon.ico` сгенерирован через `npx @tauri-apps/cli icon` из Version B master (simplified glossy SDH hex) — PNG-compressed entries 16/24/32/48/64/256, включая критический 24×24 для Windows 11 taskbar на 100% DPI. Для больших Windows Store tiles (`Square142+`, `Square150+`, `Square284+`, `Square310+`) и `128x128.png` / `icon.png` — Version A (полный hub с 4 nodes) для Start Menu / большого отображения. Логотип в titlebar (20×20 Version B glossy hex). `app.title` → "Solo Dev Hub" в ru/en.
- 3 новых sync.rs теста (`test_copy_doc_skeleton_when_missing/skips_existing/empty_template_noop`) → 108 passing total.
- 4 новых i18n-ключа × ru/en = 8 записей (`repo.initDocsButton`, `toast.docsInitialized/docsAlreadyExist`, + изменён `app.title`).

### Changed
- `sync_project` pre-phase extended: fetch templates `todo.md.tmpl` + `bug-reports.md.tmpl` → copy-if-missing для всех репо и ms-server-repos.
- `app.title` UI-строка → "Solo Dev Hub" + логотип в titlebar + новая иконка приложения. **Нота**: `productName` в `tauri.conf.json`, installer filename, Windows taskbar-hover-tooltip остаются `github-repo-manager` до F-019 (полный rebrand identifier/DB path) — несоответствие визуального бренда в окне и внешних системных точках — ожидаемое промежуточное состояние.

### Docs
- Обновлён [docs/flows/repo-auto-files.md](docs/flows/repo-auto-files.md): таблица auto-generated файлов расширена с 3 до 5 строк (`docs/todo.md` + `docs/bug-reports.md`); упомянут `init_docs_for_repo` + кнопка как manual trigger.

## [0.10.0] — 2026-04-17

### Added
- **F-011 Local-only repos**: `github_name` теперь nullable (миграция v13). Новая команда `create_local_repository` + кнопка 📁 "+ Локальная папка" в Sidebar для добавления папок без GitHub. RepoDetail скрывает GitHub-секции (SecretsPanel, Actions, GitHub URL) для local-only репо. `getDisplayName` использует `description` как fallback для `github_name`.
- **F-010 project.md manifest**: автогенерация `docs/project.md` во всех репо проекта при каждом sync'e. Содержит: имя/описание/тип проекта, таблицу репозиториев с ролями, подключённые микросервисы, родительские проекты. Перезаписывается при каждом sync (app-owned).
- **F-014 CLAUDE.md section**: автогенерируемая секция `<!-- manager:begin -->` … `<!-- manager:end -->` в корневом CLAUDE.md каждого репо. Обновляется при каждом sync. Если CLAUDE.md нет — создаётся. Если есть без маркеров — секция дописывается в конец. Если с маркерами — содержимое между ними заменяется. Orphan-маркеры (один без пары) → Err с инструкцией ручной чистки. Кнопка "Синхронизировать в ~/.claude/CLAUDE.md" в AppDefaultsScreen — с подтверждением.
- **Auto-.gitignore**: при sync'e, если `.gitignore` в корне репо отсутствует — копируется из шаблона в настройках. После создания — user-owned, приложение больше не трогает.
- Новая `language_key="_global"` в templates table: bundled `.gitignore.tmpl` + `claude.md.section.tmpl`. Seeder подхватывает автоматически через `include_dir!`. Пользовательские правки (is_custom=1) сохраняются при обновлении приложения.
- Новый shared-компонент `TemplateEditor.svelte` (file-list + editor + save + reset-to-bundle).
- Новый экран `AppDefaultsScreen` (вход из Settings → "Шаблоны приложения") — редактирует `_global` шаблоны через `TemplateEditor`.
- 21 новый Rust-тест (migration v13: 4, local-repo insert: 2, generators + markers + gitignore: 15).
- 17 новых i18n-ключей × ru/en (sidebar, repo, settings, appDefaults, templateEditor namespaces).

### Changed
- `sync_project` выполняет pre-phase (project.md + CLAUDE.md section + .gitignore) для всех репо проекта и server-репо подключённых микросервисов перед REQ/api/handlers sync'ом.
- `Repository.github_name` nullable в TS (`string | null`). Null-safety guards добавлены в RepoList (фильтр-поиск), Sidebar (сортировка), ProjectDetail (сортировка), Settings (workspace scan), DeployScreen (branch list), SecretsPanel (push secrets).
- `TemplatesScreen` рефакторен: editor-часть извлечена в `TemplateEditor`, язык `_global` фильтруется из списка.

### DB
- Migration v13 пересоздаёт таблицу `repositories` чтобы сделать `github_name` nullable. Все constraints (CHECK role, DATETIME defaults, FK ON DELETE) сохранены. Existing data не затронуты.
- Bundled `_global` шаблоны вносятся через seeder (не миграцию) — автоматически при первом запуске.

### Docs
- Новый flow [docs/flows/repo-auto-files.md](docs/flows/repo-auto-files.md): project.md + CLAUDE.md section + .gitignore — зачем, когда, куда.
- Обновлён [docs/flows/templates-system.md](docs/flows/templates-system.md): секция про `_global` language_key, AppDefaultsScreen, фильтр `_global` в TemplatesScreen.

## [0.9.0] — 2026-04-17

### Added
- **Cross-repo sync расширен** на `handlers.md` (server → clients) и на направление microservice → parent-server (`api.md` + `handlers.md`). Ранее синхронизировался только `api.md` server → client; теперь покрыты все 4 варианта контрактных документов.
- Новая папочная структура на стороне получателя:
  - У клиента: `docs/server-api/api.md` и `docs/server-api/handlers.md` (раньше `api.md` был в корне `docs/`).
  - У parent-сервера: `docs/microservice-api/<ms-project-name>/api.md` + `handlers.md` — по одному подкаталогу на каждый подключённый microservice-проект.
- **Автомиграция `docs/api.md` → `docs/server-api/api.md` у клиентов**: при первом sync после обновления, если старый файл существует и новый отсутствует, приложение атомарно его переносит (copy, затем remove old — при сбое записи old остаётся). Счётчик миграций отображается в toast'е после sync'а.
- Поле `migrated: usize` в `SyncResult`, новый i18n-ключ `sync.syncCompleteFull` ("Скопировано: {0}, квитанций: {1}, перенесено: {2}").
- Два новых direction-значения в `RequirementInfo` — `microservice_to_server_api`, `microservice_to_server_handlers` — с отдельными секциями в SyncScreen и i18n-ключами `sync.microserviceToServerApi` / `sync.microserviceToServerHandlers` (ru/en).
- Sync-helper `sync::migrate_file(source, target)` + 4 новых теста (84 cargo test total).

### Changed
- Target для `api.md` на клиентах переехал из `docs/api.md` в `docs/server-api/api.md`. `list_project_requirements` теперь считывает новый путь; автомиграция одноразовая.
- Синхронизация микросервиса с parent-сервером дополнена копированием его `docs/api.md` и `docs/handlers.md` (помимо REQ-*.md, которые уже работали в 0.8.0). Имя подпапки — `ms-project.name`, тот же источник истины, что и для requirements.
- Отсутствующие `api.md` / `handlers.md` у отправителя — тихий skip без записи в `errors` (считаем эти файлы опциональными).

### Breaking
- Layout на клиенте: `docs/api.md` → `docs/server-api/api.md`. При первом sync выполняется автомиграция. Если старый файл остался (read-only FS, отсутствовал sync с момента обновления и т.п.) — перенести вручную: `git mv docs/api.md docs/server-api/api.md`.
- `SyncResult` теперь имеет дополнительное поле `migrated` (TS-интерфейс и Rust-struct). Вызывающий код должен знать о нём — фронт уже обновлён.

## [0.8.1] — 2026-04-16

### Fixed
- B-001: в пустом состоянии Microservices-секции показывался устаревший hint "Назначьте роль «Микросервис» репозиториям" — роль удалена в 0.8.0. Текст `project.noMicroservicesHint` заменён на "Нет доступных проектов Микросервис" (ru) / "No Microservice projects available" (en).
- B-002: поведение textarea описания проекта приведено к паттерну BugItem — `rows=2`, autoFocus, Enter без Shift сохраняет, Shift+Enter перенос строки, Escape отмена. Убрано фиксированное `max-width=600px`, добавлен `min-height=60px` чтобы не было vertical-jump при переключении display → edit.
- B-003: смена типа проекта переведена на `<select>` с двумя опциями (📁 Стандартный / ⚙ Микросервис). Стиль идентичен role-select в repo-table (системные цвета, корректно работают в обеих темах). Иконка 🔒 справа от select + tooltip когда смена заблокирована (обёртка `<span>` с title — `disabled` select не триггерит tooltip в браузерах надёжно). **Упрощена логика блокировки**: смена типа блокируется только когда проект — microservice, уже подключённый к parent-проектам. Репо и свои подключённые микросервисы не блокируют — user может свободно менять модель. Обновлён тест `test_update_project_type_blocked_only_when_connected_as_microservice`, актуализирован [docs/flows/microservice-server-sync.md](docs/flows/microservice-server-sync.md).

### Docs
- Созданы 7 новых флоу в [docs/flows/](docs/flows/): `microservice-server-sync.md`, `deploy-flow.md`, `requirements-sync.md`, `templates-system.md`, `secrets-management.md`, `bug-tracking.md`, `repository-deletion.md`.
- Обновлён [docs/roadmap.md](docs/roadmap.md): 0.4.0–0.8.0 отмечены выполненными, переструктурирован будущий roadmap (0.9.0 data-model completeness + handlers.md/gitignore, 1.0.0 UX/визуализация, 1.1.0 advanced).

## [0.8.0] — 2026-04-16

### Added
- **F-012 Microservice = Project Type**: микросервис теперь отдельный тип проекта (`projects.project_type` = `standard` | `microservice`), а не роль репо. Microservice-проект может содержать свои репо (сервер + клиенты), свои секреты, свой deploy.
- Sidebar: форма создания проекта с выбором типа (Стандартный / Микросервис). Иконка ⚙ для microservice-проектов в дереве.
- ProjectDetail: badge типа рядом с датой создания, кнопка "Сменить тип" (disabled если проект не пустой).
- ProjectDetail для microservice: секция "Подключён к проектам" со списком parent'ов.
- ProjectDetail: секция Microservices теперь работает со списком microservice-проектов (через `list_microservice_projects`), показывает имя server-репо каждого микросервиса (или предупреждение "no server"/"multiple servers").
- Rust: 4 новые команды — `list_microservice_projects`, `list_parents_of_microservice`, `update_project_type`, `server_repo_of_microservice`.
- Runtime cycle detection (DFS) при `connect_microservice` — циклы A→B→...→A блокируются.
- +9 новых db-тестов (cycle detection, server-repo validation, parent guards, type change conditions).
- 15 новых i18n ключей × ru/en в namespace `project.*`, `sync.microserviceSkipped`, `toast.projectTypeChanged/cycleDetected`.

### Changed
- `create_project` принимает параметр `project_type` (backward-incompatible для Rust, но frontend-store имеет дефолт `'standard'`).
- `connect_microservice` / `disconnect_microservice` / `list_project_microservices` работают с **project_id** микросервиса (раньше — repository_id).
- Sync `server → microservice`: теперь находит server-репо внутри microservice-проекта (через `server_repo_of_microservice`) и копирует требования туда. Microservice-проект **обязан** иметь ровно один server-репо для sync, иначе запись в errors и skip.
- `delete_project` для microservice-проекта блокируется если подключён к parent'ам.
- `update_project_type` разрешён только если проект полностью пустой (нет репо, parent-связей, подключённых микросервисов).

### Removed
- Роль `microservice` из `Role` union и role-dropdown'ов на RepoDetail/ProjectDetail/RepoList. (В ROLE_ICONS и i18n `role.microservice` оставлено для graceful degradation legacy значений в БД.)
- B-006 автоочистка project_id при смене роли на 'microservice' — больше не применимо.

### Breaking
- Миграция v12 **удаляет существующие связи** в таблице `project_microservices` (старая семантика `(parent_project_id, microservice_repo_id)` несовместима с новой `(parent_project_id, microservice_project_id)`). После апдейта необходимо пересоздать микросервисы:
  1. Для каждого бывшего репо с `role='microservice'` создать проект типа `microservice` (Sidebar → выбор типа).
  2. Перенести этот репо в новый проект с `role='server'`.
  3. Подключить microservice-проекты к parent'ам через ProjectDetail → секция Microservices.

  В текущем использовании таких связей не было, риск нулевой, но документация обязательна.

## [0.7.0] — 2026-04-16

### Added
- **Deploy feature (T-028)**: кнопка "🚀 Deploy" на экране репо (видна когда задан `deploy_target`). Открывает отдельный DeployScreen с формой манифеста (5 полей: workflow name, image tag, compose service, domain, deploy branch), checklist обязательных секретов (берётся из `meta.json` шаблона), кнопкой "Generate files".
- **Dropdown `deploy_target` в RepoDetail meta-row** — выбор шаблона из доступных языков. Репо без deploy_target не показывают кнопку Deploy.
- **Выбор ветки через GitHub API** — dropdown подтягивает реальные ветки репо через `octokit.repos.listBranches`, помечает default-ветку.
- **Side-by-side diff при перезаписи** существующих файлов в репо (через npm-зависимость `diff`). Чекбокс per-file "Overwrite"/"Create", UNCHANGED файлы отмечены и skipped.
- **Renderer `template_render.rs`** — чистая функция `render_template(tmpl, vars)` с синтаксисом `@@KEY@@`. Missing key → Err. Regression-тест против байт-копии боевого `swan_support_test/.github/workflows/deploy.yml`.
- **SQLite миграция v11**: поле `deploy_target` на `repositories`, таблица `deploy_manifests` per-repo (CASCADE при удалении репо).
- **meta.json v2**: поле `file_targets` (маппинг template file → путь в репо).
- **6 Tauri команд**: `set_deploy_target`, `get_deploy_manifest`, `save_deploy_manifest`, `render_deploy_files`, `read_repo_files`, `write_deploy_files`.
- **37 новых i18n ключей** (ru+en) в `deploy.*`, `repo.deploy*`, `toast.deploy*`.

### Changed
- **template_seeder** теперь делает auto-migrate: при обновлении приложения non-custom (`is_custom=0`) шаблоны автоматически синхронизируются с bundle, user-edited (`is_custom=1`) сохраняются. Бесплатная миграция meta.json v1 → v2.

## [0.6.0] — 2026-04-15

### Added
- Система шаблонов per-language (для будущей фичи Deploy): таблица `templates` в SQLite, bundled-seed из `src-tauri/templates/<lang>/` на старте приложения через `include_dir`. Первый шаблон — `flutter_web` с `deploy.yml.tmpl`, `dockerfile.tmpl`, `meta.json` (описание + список обязательных секретов).
- Settings экран: редактор шаблонов — переключение языков, список файлов, inline-редактирование с валидацией JSON для `meta.json`, "Reset to default" для возврата к bundled версии.
- 5 новых Tauri-команд: `list_template_languages`, `list_template_files`, `get_template_file`, `save_template_file`, `reset_template_file`.
- Поддержка multi-line секретов в env-тексте через triple-quoted синтаксис: `SSH_KEY="""\n-----BEGIN...\n-----END\n"""`.
- 20 новых i18n ключей в namespace `templates.*` и `secrets.multilineHint/unclosedQuote`.

### Fixed
- SecretsPanel: при ручном вводе значения существующего секрета теперь поддерживается multi-line (input заменён на textarea с CSS-маскировкой `-webkit-text-security: disc`). Раньше PEM-ключи не проходили — парсер бил по `\n`.

## [0.5.0] — 2026-04-15

### Added
- B-003: Кнопка удаления репозитория в RepoDetail с гибкими опциями (чекбоксы). Три независимых действия: удалить из программы (всегда), удалить с GitHub (требует scope `delete_repo`), очистить локальную `.git` папку (остальные файлы сохраняются). Подтверждение через ввод имени репо.

### Fixed
- B-001: Защита от создания документов в несуществующих папках. `sync_project` и `write_bugs_to_file` теперь проверяют существование корневой папки репо перед любыми операциями записи. Если папка удалена/перемещена — ошибка с понятным текстом, папки не пересоздаются.
- B-002: В SyncScreen требования теперь сгруппированы по отправителю (source_repo) внутри каждого направления синхронизации.

### Changed
- B-004: Роль `tool` переименована "Инструмент" → "Утилита" (ru only).
- `write_bugs_to_file` Tauri-команда принимает дополнительный параметр `repo_root` для валидации корневой папки.

## [0.4.0] — 2026-04-08

### Added
- GitHub Actions Secrets: просмотр, добавление, обновление и удаление секретов репозитория
- Secrets на уровне проекта: массовый push секретов во все репозитории проекта с выбором через чекбоксы
- Список существующих секретов с чекбоксами, inline-полем для нового значения, массовые обновление/удаление
- Текстбокс в env-формате (KEY=value) для добавления новых секретов
- Шифрование секретов через libsodium sealed box (клиентское, как требует GitHub API)
- 30 новых i18n ключей (ru/en) в namespace `secrets.*`

## [0.3.0] — 2026-04-02

### Added
- display_name (без owner/) во всём UI, github_id для стабильности при переименовании репо
- Секция "Сервер → Клиент" в SyncScreen (api.md)
- Категории ошибок по ролям (client: ui_ux/ux_flow/logic/integration, server: logic/db/perf/security)
- Новые категории: integration, ux_flow
- Лимит 20 багов, нумерация по первому свободному

### Fixed
- B-007: api.md копируется при sync + диагностика ошибок
- B-008: auto-save при переключении страницы (flushBugs)
- B-009: display_name без owner/ везде, папки sync по display_name
- B-012: попытки при переходе в testing (UI + auto-detect при reload)
- B-013: категории по ролям
- B-019: лимит 20, нумерация по первому свободному
- B-001: ProjectDetail — block layout, stats не сжимают repo list

### Changed
- Попытки фиксируются при testing, не при confirmed
- Приложение автоматически детектит переход в testing при reload (LLM не обязан считать)

## [0.2.0] — 2026-03-31

### Added
- Синхронизация требований: локальное копирование REQ-*.md + api.md между репо
- Экран синхронизации: список требований, статусы, confirm/reject
- Дашборд: 3 отчёта (по статусу, по категориям, по severity), строки = проект→репо
- resolved_count в статистике (миграция v8)
- Bug statistics: накопительная таблица bug_stats (severity × category × date per repo)
- Статистика по дням
- Сброс статистики = пересчёт из MD файлов
- Тёмная/светлая тема
- Микросервисы: подключение к проектам (many-to-many, toggle UI)
- Роли: microservice, landing, tool
- Microservice → автоматически без проекта
- Workspace root setting + автопоиск локальных репозиториев
- Project detail screen (inline edit, delete, repo list)
- Drag-and-drop репозиториев между проектами
- Bug category field (UI/UX, Backend, Network, etc.)
- File-based bug read/write (docs/bug-reports.md)
- Reload from file button

### Fixed
- B-001: Новый баг с пустым описанием
- B-002: Дашборд — param names mismatch + переделка формата
- B-003: Окно подтверждения шире
- B-004: Enter сохраняет, Shift+Enter перенос строки
- B-005: Логика 2-я в категориях
- B-006: Microservice скрывает project select
- B-007: Drag-and-drop через pointer events
- B-008: ID багов B-NNN
- B-009: Локальная дата
- B-010: "Попыток:" с заглавной
- Confirmed баги с зелёной полосой
- Папка local_path белым цветом
- Удаление бага: deletedIds предотвращает восстановление при merge
- Confirmed = попытка + resolved_count
- bug.id вместо порядкового номера #{index}
- Многострочные описания: \n экранируются в MD файле
- Навигация назад запоминает предыдущий экран
- Убран рудимент [ ] из формата MD
- Кнопка Sync перенесена к "Репозитории проекта"
- Sync копирует api.md сервера клиентам
- Кнопка "Удалить проект" прижата вправо
- Сортировка репо в экране проекта по ролям
- Очищен dead code (10 Rust warnings)

### Changed
- Bug reports: формат B-NNN, статусная модель (created/in-progress/testing/confirmed/rejected)
- Bug reports: severity (critical/major/medium/minor) вместо priority
- Bug reports: MD файл как источник истины (не SQLite)
- Bug reports: флоу документирован в заголовке bug-reports.md
- BugItem: компактный 3-строчный layout
- Sidebar: шире (280px), + рядом с заголовком, Sync иконкой
- RepoList: показывает только неназначенные репозитории
- RepoDetail: sticky header, local_path в шапке

## [0.1.0] — 2026-03-29

### Added (i18n)
- Localization system: Russian (default) + English
- Language switcher in Settings screen
- 122 translation keys covering all UI strings, tooltips, toasts, empty states
- Language preference persisted in SQLite settings
- Locale-aware date formatting
- Type-safe translation keys (missing key = compile error)

### Added
- Project scaffolded with Tauri v2 + SvelteKit + Svelte 5 (TypeScript)
- Rust backend: SQLite database with migration system (user_version pragma)
- Full CRUD for projects, repositories, and bug notes
- PAT storage via Windows Credential Manager (keyring crate)
- Markdown export with spec format (embedded bug IDs, priority tags, attempts)
- Markdown import with ID/title-based deduplication
- Settings management (export path)
- GitHub API client with pagination (@octokit/rest)
- TypeScript types matching Rust models
- Typed Tauri command bindings
- Svelte stores for all data entities (projects, repos, bugs, settings, UI)
- App shell with header, sidebar, content area, toast notifications
- Settings screen: masked PAT input, folder picker, token validation
- Sidebar: collapsible project tree, role icons, sync button, create project form
- Repository list: search, project/role assignment dropdowns
- Repository detail: GitHub info, editable fields, embedded bug notes
- Bug notes ToDo UI: checkboxes, priority badges, inline editing, fix attempts counter
- Export MD / Import MD buttons with file dialog
- Confirmation dialogs for destructive actions
- Empty states with actionable hints
- Tooltips on all interactive elements
- CSS variables for theming
- 31 Rust tests passing, 0 TypeScript errors
