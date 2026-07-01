# Changelog

Формат: [Keep a Changelog](https://keepachangelog.com/)

## [Unreleased]

## [1.5.0] — 2026-07-01

Веха внутреннего рефакторинга. Декомпозирует два самых крупных Svelte-компонента в фокусные суб-компоненты перед работой над типизированным secrets-vault (v1.6.0). User-facing поведение не меняется; MINOR-бамп отмечает веху рефакторинга по роадмапу.

### Changed
- **ProjectDetail и SecretsPanel разбиты на фокусные компоненты (без изменения поведения).** `ProjectDetail.svelte` (980 строк) выносит `ProjectHeader.svelte` (заголовок, inline-редактирование имени/описания, выбор типа проекта, действия edit/delete + confirm-диалоги) и `ProjectMicroservicesTab.svelte` (подключение/отключение других microservice-проектов + список подключённых parent'ов, владеет своим connection-состоянием и грузится лениво при монтировании); `parentsOfMicroservice` остаётся в родителе и прокидывается в оба дочерних компонента пропсами. `SecretsPanel.svelte` (920 строк) выносит `SecretsList.svelte` (список существующих секретов: чекбоксы, per-row автосохранение, bulk-удаление) с экспортируемым `reload()`, который родитель вызывает после push для refresh-and-verify одним запросом. Flat flex-shrink каскад (T-000129) сохранён через собственное правило на корне нового списка.

### Removed
- **Мёртвый project-mode в SecretsPanel.** Флоу project-wide push секретов (`mode='project'` — проп, состояние, функции, confirm-диалог) не имел ни одного call site — SecretsPanel использовался только в repo-режиме из RepoDetail. Удалён вместе с 4 осиротевшими i18n-ключами. Восстановимо из git, если project-wide push когда-нибудь понадобится.

### Tests
- 400 cargo / 86 vitest / 0 svelte issues (без изменений — чистое перемещение кода).

## [1.4.1] — 2026-06-15

Patch-релиз. Продолжает работу v1.4.0 по внутреннему рефакторингу — декомпозирует самый крупный из оставшихся обработчиков команд — и добавляет self-heal для локального dev-сервера. User-facing поведение не меняется.

### Changed
- **`sync_project` вынесен из слоя команд (без изменения поведения).** Обработчик `sync_project` (~712 строк) извлечён из `commands/sync.rs` в новый доменный модуль `sync/project_sync.rs` (`run_project_sync`), Tauri-команда сократилась до 3-строчной обёртки. Тело разбито на фокусные хелперы — аккумулятор `SyncCounters`, `load_skeleton_templates`, `write_repo_skeletons` (схлопывает два дублирующихся Phase-0 цикла скелетов) и `sync_client_to_server` / `sync_server_to_microservice` / `sync_microservice_to_parents` — с сохранением всех load-bearing граней (ранний выход B-001 при перемещённой папке сервера, идемпотентность rename-replay, гейтинг предупреждений «нет сервера / нет клиентов» для standard-проектов, единственное чтение `project_type` в начале).
- **dev: авто-освобождение порта 1420 перед `tauri dev`.** Прерванный `tauri dev` оставляет vite-сироту, продолжающую слушать порт 1420; поскольку `strictPort` у vite намеренный (`devUrl` в Tauri жёстко привязан к порту), следующий запуск жёстко падал с «Port 1420 already in use». Новый шаг `predev` (`scripts/free-port.mjs` — без зависимостей, кросс-платформенный, с учётом IPv6) сбрасывает зависший листенер, так что `npm run dev` / `tauri dev` стартует чисто каждый раз.

### Tests
- 400 cargo / 86 vitest / 0 svelte (без изменений — декомпозиция перенесла код, не меняя поведение).

## [1.4.0] — 2026-06-14

Веха внутреннего рефакторинга. Монолитный слой команд разъезжается по доменным модулям с обеих сторон — Rust `lib.rs` (103 Tauri-команды, 3346 строк) в `commands/*.rs`, а TypeScript-биндинги `tauri-commands.ts` в каталог `tauri-commands/` — и ручка ресайза сайдбара вынесена в отдельный компонент. Новой user-facing capability нет; MINOR-бамп фиксирует веху рефакторинга по роадмапу (v1.4–v1.6). Заодно — полиш экрана «Наборы секретов» и фикс сломанной темы, всплывшие на dogfood'е.

### Changed
- **Внутренний рефакторинг модулей (без изменения поведения).** `lib.rs` разбит на `commands/{project,repo,bug,dashboard,sync,templates,deploy,timeline,misc}.rs` (хендлеры регистрируются по полному пути модуля, `lib.rs` сократился до 223 строк); `tauri-commands.ts` разбит на каталог `tauri-commands/` с реэкспортом через index — спецификатор `$lib/api/tauri-commands` резолвится в каталог, поэтому импорты потребителей не менялись; ручка drag-ресайза сайдбара вынесена в `SidebarResizer.svelte` с bindable-интерфейсом `width` / `collapsed` / `isResizing` / `previewWidth` и колбэком `onCommit`.
- **Полиш экрана «Наборы секретов» (dogfood).** Поля значений теперь маскированные textarea (как в панели секретов репо): свёрнуты в одну строку, раздвигаются на фокусе, есть переключатель показа и тянущийся угол — многострочные ключи вроде `SSH_KEY` наконец читаются и редактируются (старый однострочный `input` их корёжил). В списке наборов имя слева, счётчик секретов справа в той же строке, между наборами разделители, активный набор подсвечен акцентом. «Новый набор» свёрнут в одну кнопку, раскрывающую форму создания прямо на месте (кнопка «Создать» активна только с именем, рядом «Отмена») — убран дублирующий заголовок. Кнопка «Добавить секреты» прижата вправо.

### Fixed
- **Сломанная тема на экранах «Секреты» и «Деплои».** `SecretBundles`, `DeploySecretsTable` и `DeployScreen` ссылались на CSS-переменные (`--hover-bg`, `--border-light`), которых нет в теме и у которых нет fallback — поэтому фон панелей, hover строк и границы рендерились прозрачными/невалидными. Переназначены на реальные токены (`--surface` / `--surface-hover` / `--border`).

### Tests
- 400 cargo / 86 vitest / 0 svelte (без изменений — рефакторинг перенёс код, не меняя поведение; `cargo check` чист после `#[cfg(test)]`-гейта на test-only read-хелперах event-логов).

## [1.3.0] — 2026-06-14

Добавляет переиспользуемые, локально-зашифрованные наборы секретов — вводишь одни и те же SSH / DB / npm значения один раз и применяешь к секретам любого репо или deploy-окружения, вместо повторного ввода в каждом репо. Заодно фиксит протекание repo deploy-config между репо, всплывшее на dogfood'е. MINOR-бамп обоснован новым экраном 🔐 «Наборы секретов» — новой user-facing capability.

### Added
- **Наборы секретов — новый экран 🔐 «Наборы секретов» в титлбаре.** Переиспользуемые именованные наборы значений GitHub-секретов, **зашифрованные at-rest** (AES-256-GCM, 32-байт data-key в OS keyring — без мастер-пароля; та же модель доверия, что у PAT). Master-detail редактор: создание / переименование / удаление наборов, добавление секретов через **bulk-поле `KEY=VALUE`** (тот же dotenv-формат + per-line валидация, что в панели секретов репо), маскированные значения с per-row reveal-toggle, удаление отдельных секретов. **Применение набора с двух поверхностей** — SecretsPanel (→ секреты уровня репо) и DeploySecretsTable (→ секреты deploy-окружения) — мёрж расшифрованных значений в существующую форму пуша (набор побеждает при конфликте имён); существующие пути пуша в GitHub не тронуты. Новые таблицы `secret_bundles` + `secret_bundle_items` (миграция v26 — значения только как зашифрованный BLOB + nonce, никогда не plaintext), модуль `crypto/bundle_cipher`, keyring data-key helper, 7 Tauri-команд и pure-хелпер `bundle-apply` с round-trip-безопасной сериализацией (triple-quote / escaped-double-quote fallback — значение с `"""` или переносами строк переживает повторный парсинг перед пушем).

### Fixed
- **Протекание repo deploy-config между репо.** Фокус поля «Shared image config» в репо A с последующим переключением на репо B записывал shared-конфиг A в B — save на blur резолвил целевой репо из живого глобального выбора (`$selectedRepoId`), а не из репо, которому принадлежала правка. Теперь save захватывает id своего репо при маунте (снимок через `untrack` — deploy-блок keyed по репо, один репо на инстанс) и гардит запись по текущему выбору, так что blur во время teardown'а переключения отбрасывается, а не утекает. Обычные save в том же репо не затронуты.

### Tests
- 400 cargo (+14: AES-GCM cipher roundtrip / tamper / wrong-key / nonce-length ×7; migration v26 ×1; bundle db CRUD + CASCADE + UNIQUE + decrypt ×6) / 86 vitest (+14: apply-merge map overlay + dotenv-text merge по 8 round-trip классам значений вкл. triple-quote-в-multiline и literal-backslash edge-кейсы) / 0 svelte.

## [1.2.0] — 2026-06-02

Добавляет портфельный отчёт по деплоям и managed-шаблон `.gitattributes`, плюс два dogfood-фикса (Дашборд / секреты). MINOR-бамп обоснован новым top-level экраном «Деплои» — новая user-facing capability.

### Added
- **Отчёт по деплоям — новый экран «Деплои» в титлбаре.** Портфельный обзор всех deploy-окружений по всем репо, сгруппированный по проектам (orphan-репо под «Без проекта»). Колонки: репозиторий, окружение (цветной badge), домен (клик → открыть в браузере), ветка, image tag, число подключённых секретов, дата обновления конфига. Фильтры проект / окружение / поиск; колонки выровнены между секциями через фиксированную раскладку таблицы. Клик по строке проваливается во вкладку Deploy этого репо с открытым окружением — через one-shot `deployDrillTarget` (паттерн Timeline deep-link). Read-only — live-статус GitHub Actions отложен на следующий релиз. Новый запрос `list_deploy_report` (JOIN `deploy_environments` × `repositories` × `projects` + счётчик секретов, имя репо через `display_name()`) + Tauri-команда + `DeployReport.svelte`.
- **Managed-шаблон `.gitattributes` (B-000024).** Новый `_global/.gitattributes.tmpl` для managed-реп: `* text=auto eol=lf` для исходников, CRLF для Windows-скриптов (`*.bat`/`*.cmd`/`*.ps1`), бинарные маркеры. Section-merge логика общая с `.gitignore` — вынесена в `sync/managed_block`, `sync_gitignore_section` стал тонкой обёрткой. Обвязан в `init_docs_for_repo` + `sync_project`; появляется в Настройки → Шаблоны по умолчанию сам (seeder + редактор zero-touch через `include_dir!`).

### Fixed
- **B-000026 (регрессия, major).** Кастомный период дат в Дашборде не работал — выбор пресета «Кастом» только подсвечивал кнопку и не показывал поля дат. Добавил поля начала/конца (`<input type="date">`, засеянные текущим окном), подключённые к уже существовавшему, но осиротевшему `setCustomPeriod`, с guard'ом `start ≤ end`.
- **B-000025.** Поле массового ввода секретов теперь помнит содержимое per-repo и восстанавливает при возврате, а не затирается при каждом переключении репо. Реализовано через module-level map черновиков по репо в `SecretsPanel` (load/save при смене репо); очистка после пуша сохраняется per-repo. (Первая попытка — `{#key}` remount с очисткой поля — была отклонена; это переделка.)

### Tests
- 386 cargo (+6) / 72 vitest / 0 svelte.

## [1.1.0] — 2026-05-25

Первый MINOR после public launch v1.0.0. Добавляет verdict-rollback путь для багов плюс два dogfood-всплывших UX фикса той же недели. MINOR-бамп обоснован T-000130 — новая user-facing capability (↩ reopen кнопка) — не polish-фиксами что пошли вместе с ней.

### Added
- **T-000130 — Reopen bug action (↩ кнопка).** До этого ✓ и ✗ на `testing`-баге были one-way: клик по неосторожности или на второй взгляд оставлял user'а без обратного пути. Новая кнопка ↩ на `confirmed` и `rejected` строках переоткрывает баг в `testing` чтобы verdict можно было пересдать без потери fix-истории. Заменяет non-interactive ✓ mark на `confirmed`-строках (status badge уже называет состояние) и заполняет ранее-пустой action-слот на `rejected`. Никакого confirm-диалога — вся точка кнопки в быстром rollback'е. Новый Tauri command `reopen_bug(repo_id, display_id)` + DB method `reopen_bug(bug_id)` атомарно: `status='testing'`, `confirmed_at = NULL`, `archived_from_md_at = NULL`. `fix_attempts` намеренно сохранён — reopen это undo verdict'а, не новая fix-попытка. Event `reopened` пишется в `bug_events` (с `from_status` = original) чтобы Dashboard activity feed увидел action, но он НЕ контрибутит в KPI5 (avg attempts per closed in period фильтрует по `event_type='entered_testing'`). Invariant `COUNT(entered_testing) == bugs.fix_attempts` сохраняется через reopen. Amber `#f59e0b` для кнопки отличает "rollback" от завершающего ✓ (зелёный) и rejecting ✗ (красный).

### Changed
- **`dialog.confirm` i18n key** — `Подтвердить` / `Confirm` → `ОК` / `OK`. Затрагивает каждый `ConfirmDialog` site (13 usages: удаление бага, reject бага, удаление deploy env, GlobalClaudeEditor discard, project type change, удаление проекта, удаление репо, secrets bulk delete, project secrets push, sidebar project delete, template revert). Заголовок диалога уже называет action — кнопке достаточно быть confirmation primitive. Закрывает B-000022.

### Fixed
- **T-000129 — SecretsPanel bulk-paste textarea теперь растёт вертикально** заполняя таб Секреты. Раньше залочено на `rows="4"` (~70px) независимо от viewport'а — на 1337px-высоком окне инпут занимал 5% vertical real estate, остальное стояло пустым. `.secrets-wrapper` в RepoDetail теперь раскладывается как `display: flex; flex-direction: column`, а `.secrets-section.flat` каскадит `flex: 1 / min-height: 0` через `.secrets-body` → `.new-secrets` → `.secrets-textarea`. У `.existing-secrets` `flex-shrink: 0` — длинный список сохраняет natural-высоту, wrapper скроллит вместо сжатия. Минимум высоты 70px оставлен как floor; resize-vertical handle preserved.

### Tests
- 380 cargo (+4 для `reopen_bug`: confirmed → testing чистит confirmed_at + сохраняет attempts; rejected → testing сохраняет attempts; archived_from_md_at очищается чтобы баг вернулся в MD; bug_events invariant `COUNT(entered_testing) == fix_attempts` держится через reopen) / 72 vitest / 0 svelte issues.

## [1.0.4] — 2026-05-25

Template-only мини-патч — gitignore шаблон для managed репозиториев пропускал один folder-паттерн и имел trailing-slash несостыковку на другом. Downstream impact: любой managed репо чей `.gitignore` был сгенерирован из этого шаблона до v1.0.4 может иметь `docs/microservice-announcements/` untracked-but-visible в git status, плюс `docs/server-announcements` матчит loose файлы (не только папку).

### Fixed
- **`src-tauri/templates/_global/.gitignore.tmpl`** — добавлен `docs/microservice-announcements/` (симметрия к `docs/server-announcements/`; оба — recipient-side acknowledgement-by-delete каналы по правилам `# Cross-repo announcements`, ни тот ни другой не должен попадать в git history). Добавлен trailing `/` к `docs/server-announcements` чтобы паттерн однозначно матчил папку, а не одноимённые loose файлы. Существующие managed репо подхватят оба паттерна при следующем "Sync global rules".

### Tests
- 376 cargo / 72 vitest / 0 svelte issues (без изменений — template-only).

## [1.0.3] — 2026-05-25

Dogfood-патч — пять багов всплыли в ежедневном использовании v1.0.2, плюс унификация brand identity и bump Tauri runtime, выпавший из расследования B-000017.

### Added
- **T-000127** — PNG-иконки Windows bundle'а (`32x32.png`, `64x64.png`, `128x128.png`, `128x128@2x.png`) перегенерированы из нового hex+Y-tree дизайна, теперь матчат `icon.ico` от B-000017 v8. In-app бренд (титлбар `logo.png`, About-hero `logo-large.png` в `src/lib/assets/`) намеренно оставлен на старом детальном full-logo по предпочтению пользователя. macOS/iOS/Android sets отложены (локально не билдим; см. T-000125).
- **Refresh Dashboard'а** теперь реконсилит MD→DB по всему портфелю перед перезагрузкой — и ручная кнопка `↻`, и initial `onMount` зовут новую backend-команду `reconcile_all_projects`, которая обходит каждый репо и гонит `reconcile_bugs_for_repo` + `sync_tasks_for_repo`. Раньше refresh был DB-read only, поэтому MD-правки LLM оставались невидимыми до ручного sync'а каждого проекта.
- **`scripts/cleanup-target.sh`** — helper для disk recovery в cargo `target/`. Safe-режим (по умолчанию) сносит только `target/debug/incremental/` (обычно освобождает 5-10GB сохраняя скомпилированные deps); `--full` режим запускает `cargo clean` для полного debug+release wipe'а. Smoke на локальном репо: 19G → 12G в safe-режиме.
- **`sync::confirm_pair(source_repo, target_repo, filename)`** — новый helper для bilateral delete REQ-пары, резолвит пути напрямую из sender+recipient repo-записей, без зависимости от "server в текущем проекте". Заменяет ~90 строк ветвящейся path-resolution логики из `confirm_requirement`.

### Changed
- **Tauri 2.10.3 → 2.11.2** (tao 0.34.8 → 0.35.3, wry 0.54.4 → 0.55.1). Изначально investigation-шаг для B-000017; оставлено после фикса иконки потому что регрессий не всплыло.
- **`icon.ico` перегенерирован** через Python/PIL чтобы матчить структуру working sibling Tauri-приложения на той же Windows 11 / high-DPI системе. Новый файл: 6 фреймов (16/24/32/48/64/256) все 32bpp PNG-compressed, 22KB. Старый: 10 фреймов смешанных 8/24/32-bpp с uncompressed-BMP большими фреймами, 419KB.

### Fixed
- **B-000016** — кнопка `↻` refresh на Dashboard'е была DB-read only; не подхватывала MD-side правки багов / задач от LLM. Добавлена backend-команда `reconcile_all_projects` обходящая все репо и зовущая `reconcile_bugs_for_repo` + `sync_tasks_for_repo`; и ручная кнопка, и initial mount Dashboard'а теперь идут через неё.
- **B-000017** (после 8 попыток) — иконка в таскбаре Windows на high-DPI дисплеях (3072×1920 + 200% scale → ~32-48 физических px таскбар) рендерилась размазанной кляксой. Первые 4 попытки итерировали выбор дизайна, следующие 3 пробовали Tauri / Cargo-feature изменения (drop `set_icon`, drop `image-png` feature, bump версии Tauri) — все dead-end. **Реальный фикс (v8):** diff против `F:\Development\MySafeSpace` (working sibling Tauri-приложения на той же машине) выявил что виновник — structural mismatch `icon.ico`: у нас mixed bit-depths (16-фрейм 8-bpp palette, большие фреймы uncompressed BMP), у sibling'а uniformly 32bpp PNG-compressed. Перегенерил через Python/PIL зеркалируя структуру sibling'а. Methodology lesson — "для runtime/render багов сначала diff vs known-working sibling, а уже потом itерировать speculative fixes" — записан в auto-memory ассистента для будущих bug-сессий.
- **B-000018** — кнопка `+` в Sidebar'е сливалась с окружающим ASCII текстом; заменил глиф на ➕ + добавил chip-style min-width и центрированное выравнивание для визуального баланса с соседними контролами.
- **B-000019 + B-000020** — нажатие Sync на microservice-проекте было no-op'ом: `sync_project` итерировал циклы `clients` и `microservice_ids`, оба пустые для микросервиса. Добавлен MS-driven sync блок, fan'ит к каждому connected родительскому серверу через `list_parents_of_microservice` — копирует api.md и handlers.md от MS к parent, REQ файлы от parent к MS, response.md файлы от MS обратно к parent. Зеркало parent-driven блока уже существовавшего для server-проектов.
- **B-000021** — кнопка confirm-✓ на REQ-парах работала только из server-project SyncScreen'а. Открыть SyncScreen самого микросервиса по тому же REQ'у — REQ показывался как `is_reverse_lookup` и кнопка была спрятана, вынуждая user'а навигировать к parent-проекту чтобы подтвердить. Root cause: `confirm_requirement` резолвил пути через "server в current project", который в reverse-lookup view это сам MS (не реальный sender REQ'а). Вынес `sync::confirm_pair(source, target, filename)` который резолвит пути напрямую из sender + recipient repo-записей — confirm теперь работает симметрично из любой стороны SyncScreen'а. UI-гард `!req.is_reverse_lookup` снят вместе с теперь-избыточным ↩ хинт span'ом; поле `is_reverse_lookup` на `RequirementInfo` оставлено как informational/audit flag.
- **T-000128** — pending LLM-правки в `docs/bug-reports.md` silently wipe'лись когда user кликал любую bug-mutation кнопку в app'е (`+ Add bug`, ✓ confirm, ✗ reject, edit fields, delete). Root cause: 5 Tauri-команд (`create_bug`, `resolve_bug`, `update_bug_fields`, `delete_bug`, `reject_bug` в `lib.rs`) делали `mutate DB → regenerate_bugs_md` БЕЗ `reconcile_bugs_for_repo` первым — поэтому regen писал stale DB-состояние, перезаписывая всё что LLM только что отредактировал в MD. Всплыло live во время v1.0.3 closure для B-000021 (статус `testing` + comment бага постоянно откатывались в `created` / empty после каждого app-action'а). Фикс: добавлен `let _ = sync::reconcile_bugs_for_repo(&db, repo_id);` в начало каждой из 5 команд — LLM-правки ингестятся в DB первыми, mutation поверх, regen отражает обе. Pattern задокументирован + asserted в новом регресс-тесте `test_t000128_reconcile_before_mutate_preserves_llm_edits` в `sync/bugs.rs`.

### Tests
- 376 cargo (+5 новых для `sync::confirm_pair`: client→server / server→MS happy paths, sibling-NNN disambig invariant от v0.27.1 сохранён, unknown source role errors out, missing local_path silent no-op; +1 новый T-000128 регресс `test_t000128_reconcile_before_mutate_preserves_llm_edits`) / 72 vitest / 0 svelte issues на 495 файлах.

## [1.0.2] — 2026-05-18

Фикс инфраструктуры release-signing. Первый реальный end-to-end autoupdate цикл на публичном репо (v1.0.0 → v1.0.1) всплыл давнюю latent рассинхронизацию: `tauri.conf.json` нёс pubkey (`7135A97A3C3F89EF`) который **не** соответствовал приватнику в GH Secret `TAURI_SIGNING_PRIVATE_KEY` (`4D58133D6147291E`). Расхождение появилось в момент прошлой keypair-ротации — новый приватник записали в GH Secret, но соответствующий pubkey забыли пропагировать в `tauri.conf.json`. Каждый последующий релиз отправлял installer'ы чьи `.sig` верифицируются ключом которому embedded в бинаре pubkey не доверяет — невидимо пока репо был приватный (autoupdate endpoint требовал auth), невидимо пока все ставились вручную с локальных сборок, всплыло как только открылся реальный autoupdate path.

### Fixed
- **Verification подписи в autoupdate end-to-end**. Pubkey в `src-tauri/tauri.conf.json` приведён к `4D58133D6147291E` — матчит реальный CI signing key. Начиная с v1.0.2 новые installs вкомпилируют правильный pubkey и успешно verify'ят подписи.
- **⚠ Существующим v1.0.0 и v1.0.1 installs необходима разовая ручная переустановка.** Эти бинари вкомпилировали stale pubkey и продолжат отвергать v1.0.2 (и любые будущие) подписанные updates через autoupdate. Решение: скачать `Solo.Dev.Hub_1.0.2_x64-setup.exe` со страницы этого релиза и запустить один раз. После этого autoupdate работает automatically для всех будущих релизов. Новые installs от v1.0.2 onwards не затронуты.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues на 495 файлах (нет кодовых изменений кроме manifest version bump и pubkey field в `tauri.conf.json` — baseline унаследован от v1.0.1).

## [1.0.1] — 2026-05-18

Первый post-launch патч. Два бага из dogfood'а на v1.0.0 — оба регрессии от F-000041 (первый local `git` CLI shellout проекта, появившийся в v0.34.0) и layout'а path-row рядом с ним.

### Fixed
- **B-000014** (critical) — на Windows release-сборке при каждом клике по репо в сайдбаре мигало окно консоли. Root cause: `$effect` для `canUntrack` в `RepoDetail.svelte` дёргает `check_git_available_for_repo` → backend спавнит `git --version` через bare `std::process::Command::new`, на Windows-GUI-хосте subprocess наследует `STARTUPINFO` без `CREATE_NO_WINDOW` → cmd.exe всплывает на время жизни subprocess'а. Добавил helper `spawn_cmd()` который ставит `CREATE_NO_WINDOW` (`0x08000000`) через `CommandExt::creation_flags` на Windows; применил ко всем 5 production-callsite'ам в `git_ops.rs` (`check_git_available` × 2, `list_gitignored_tracked`, `untrack_files`, `count_other_staged_changes`). `#[cfg(windows)]` — no-op на macOS/Linux. Test-callsite'ы оставил на bare `Command::new` — `cargo test` на Windows гоняется в console host где флаг не имеет значения.
- **B-000015** (major) — двухчастная проблема, всплывшая по ходу smoke. Часть 1: глубоко вложенный путь в `.local-path` (например `📁 F:\Development\some\long\subdir\to\repo`) выталкивал кнопки `📚 Init docs` и `🧹 Untrack` на следующую строку `meta-row`. Закаппил `.local-path`: `max-width: 40ch` + `overflow: hidden` + `text-overflow: ellipsis` + `white-space: nowrap` + `min-width: 0` (последнее обязательно чтобы flex-child реально сократился); полный путь по hover через `title`. Часть 2 (retest): при каждой смене репо обе row-action кнопки моргали. Root cause: `$effect` для `canUntrack` синхронно ставил `false` ДО запуска async backend-проверки, поэтому `{#if canUntrack}` вырывал untrack-кнопку из DOM и возвращал обратно через ms, init-docs визуально подёргивался от flex-row reflow. Убрал sync-reset, добавил stale-response guard (`repo?.id === repoId`) чтобы более медленный ответ для репо A не перезаписывал более быстрый ответ для репо B при быстром клике A → B.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues на 495 файлах.

## [1.0.0] — 2026-05-18

**Публичный релиз.** Solo Dev Hub становится open source под лицензией MIT. Breaking API changes относительно v0.34.0 нет — релиз маркирует переход с `0.x` (unstable contract) на `1.x` (frozen contract starts here). Tauri identifier (`com.solodevhub.app`) и lib name (`solo_dev_hub_lib`) стабильны с v0.25.0, autoupdate `v0.34.x → v1.0.0` проходит как обычный in-place апдейт.

### Changed
- **T-000064** — репо `SgonnovDmGit/solo-dev-hub` переключён с private на public. Эндпойнт autoupdate `https://github.com/SgonnovDmGit/solo-dev-hub/releases/latest/download/latest.json` теперь резолвится без GitHub auth — установки на `v0.25.x..v0.34.x` подхватят `v1.0.0` через встроенный апдейтер.
- **T-000064** — старый репо `SgonnovDmGit/github-repo-manager` заархивирован (readonly) с redirect-нотой `moved to solo-dev-hub`. Полная история pre-rebrand сохранена там.
- **T-000074** — version bump в `1.0.0` по `package.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues на 495 файлах (baseline унаследован от v0.34.0 — кодовых изменений в этом релизе кроме version bump нет).

## [0.34.0] — 2026-05-17

Финальный pre-launch patch перед публичным флипом в v1.0.0. Две заметные user-видимые фичи: воркфлоу "Очистить из индекса" (F-000041) одним кликом убирает уже закоммиченные файлы которые после обновления `.gitignore` стали игнорируемыми — первый локальный shellout `git`-бинарника в проекте; и привязка имени проекта в header'е SyncScreen чтобы cross-repo flow всегда показывал контекст. Плюс правка глобальных AI-правил (ретро одним блоком, разрешение коммитить и пушить на integration-ветку, правило `&&` для PowerShell-портабельности) и CRLF-нормализация через `.gitattributes` убирающая фантомные diff'ы на Windows.

### Added
- F-000041 / T-000119 — backend-модуль `git_ops` обёрткой над локальным `git` CLI: поиск бинарника (PATH + Windows-fallback), детект состояния репо (clean / mid-merge / mid-rebase через marker-файлы), листинг через `git ls-files -ci --exclude-standard -z`, чанковый `git rm --cached`, счётчик других staged-изменений для UI info-предупреждения. Первый subprocess shellout в проекте; 12 новых unit-тестов.
- F-000041 / T-000120 — три Tauri-команды (`check_git_available_for_repo`, `list_gitignored_tracked`, `untrack_files`) + TS wrappers + boundary DTO (`UntrackReport`, `GitignoredListing`).
- F-000041 / T-000121 — `UntrackGitignoredDialog.svelte` (Svelte 5 runes, по образцу MergeChoiceDialog): select-all / deselect-all / чекбокс на каждый файл, блок при mid-merge / mid-rebase, info-варнинг при других staged-изменениях, partial-error toast. 🧹 триггер в header'е RepoDetail рядом с 📚 Init docs (housekeeping cluster). 11 i18n-ключей × ru+en + 2 toast-ключа.
- T-000123 — в header'е SyncScreen теперь отображается текущий проект (`Синхронизация — {project}`) — пользователь сразу видит scope.

### Changed
- T-000118 — `.gitattributes` добавлен в корень репо: `* text=auto eol=lf` baseline + per-extension overrides для source / data / binary групп. Убирает фантомные CRLF-модификации на Windows с `core.autocrlf=true`.
- T-000124 — глобальный AI-rules template поджат (распространяется на `~/.claude/CLAUDE.md` при следующем глобальном sync): ретро одним блоком вместо conversation-paced шести ходов; multi-branch flow (dev → master через merge) — ассистент может `git commit` и `git push origin <branch>` на integration-ветку без подтверждения на каждое действие (теги, release-merge'ы и финальный push в master остаются user-only); новая секция "Shell command portability" — избегать `&&` (падает в дефолтном Windows PowerShell 5.1), предпочитать одну команду на вызов или `;` для cross-shell совместимости.
- Internal: cargo fmt baseline (27 файлов переформатированы до rustfmt-clean) — вынесен отдельным коммитом чтобы feature-изменения не несли format-шум.

### Fixed
- F-000041 / T-000121 smoke — кнопка Untrack рендерилась посередине row 2 потому что два соседних `.row-action` (margin-left: auto) flex-сиблинга делили доступное пространство. Перенесена в row 1 рядом с Init docs с override margin'а чтобы Init Docs якорил пару к правому краю.

### Tests
- 370 cargo / 72 vitest / 0 svelte issues на 495 файлах.

## [0.33.0] — 2026-05-17

Pre-launch polish перед v1.0.0 public flip. Четыре потока: фикс format-консистентности `docs/project.md` (выявлено в dogfood'е), глобальное правил-tightening (scope `docs/handlers.md` + новая секция Release lifecycle), расширение формулы Top-3 hot, интеграция hero/feature скриншотов в README.

### Added
- **T-000112** новая подсекция `## docs/handlers.md` в `# API contract sync` global template'а — определяет опциональный server-side файл internal handler notes (transaction boundaries, side-effect chains, cross-cutting concerns), синкается симметрично с `api.md` в `docs/server-api/handlers.md` (server → client) и `docs/microservice-api/<ms>/handlers.md` (microservice → parent server). Hard rule: handler-level documentation MUST NOT идти в `README.md` (cross-repo sync пропагирует только `api.md` + `handlers.md`; README остаётся sender-side, invisible downstream).
- **T-000113** новая top-level секция `# Release lifecycle` в global template'е, между `# Phase work workflow` (per-task) и `# Manual-smoke verification`. 11 стадий (Запрос → Анализ → Спека → Ревью спеки → План → Ревью плана → Внедрение → Тест → Закрытие релиза → План на следующий релиз → Ретро) с soft permission-gated transitions и optional loop-backs (по запросу user'а или forced обстоятельствам). Каждая review-стадия — 3-step procedure (само-ревью агентом на ambiguities/contradictions/gaps → уточняющие вопросы user'у если реальные → user OK). Mandatory 6-point retro checklist (что сработало / что не сработало / готовность релиза + проекта / оценка действий LLM в сессии / оценка действий user'а в сессии / process-lessons) сохраняется как `project`-type memory file в auto-memory dir (`retro_v<X_Y_Z>.md`), не коммитится в docs/.
- **T-000115 + T-000116** Расширение формулы Top-3 hot projects. Новый weighted heat-score `critical × 50 + major × 15 + active × 1 + closed_in_period × 2 + tasks_done_in_period × 1` заменяет прежний "active bugs only" фильтр — task-active проекты теперь всплывают в top-3 когда нет severity-багов нигде. Threshold: любой ненулевой сигнал квалифицирует. `top_hot_projects` SQL принимает `Option<(period_start, period_end)>` — `Some` для dashboard window, `None` для Stats tab lifetime режима (sentinel-даты `0001-01-01` / `9999-12-31`). Frontend chips расширены до `N crit / N maj / N act / N closed · N tasks` (slash для bug-domain, middle-dot перед task-chip); native `title=""` tooltip на section header показывает полную формулу. Тот же фикс отражён в `top_hot_repos_in_project` для consistency со StatsSummary.
- **T-000073** 8 hero/feature скриншотов интегрированы в `README.md` + `README.ru.md` заменой TODO-placeholder комментариев: dashboard hero (Quarter period, KPI + top-3 hot + daily flow), repo bugs (variety severity/status/attempts), repo tasks (DataGrid с version-колонкой от T-000109), project graph (сервер центр, dashed cross-project edges), deploy master + deploy drill-down (Flutter, BUILD/DEPLOY/RUNTIME role variety), requirements sync (cross-repo REQ flow в 4 направлениях), settings (PAT/Appearance/Workspace/Templates/Global AI rules карточки).

### Changed
- **`docs/project.md` теперь в gitignore template.** Содержит user-specific локальные filesystem-пути и регенерируется на каждом sync — та же regenerated-view семья что и `docs/todo.md` / `docs/done.md` / `docs/bug-reports.md`. Добавлен в `.gitignore.tmpl`.

### Fixed
- **B-000013** Консистентность формата секций `docs/project.md`. Секции Connected microservices и Parent projects были bullet-list'ами, а Repositories — markdown-таблицей; теперь все три рендерятся как параллельные таблицы с колонками `| Microservice/Parent project | Server repo | Path | GitHub |`. Маркер-строки `_no local path configured_` и `⚠ server repo not resolvable` перенесены в ячейки таблицы; behavior announcement-LLM grep'а не меняется (матчит docstring text внутри ячеек). Global template `claude.md.global.tmpl` и spec `docs/formats/project-md.md` обновлены под новый табличный формат.

### Tests
- 358 cargo (354 → +4 от T-000115: tasks-only-qualifies / closed-in-period-contributes / one-critical-dominates-50-tasks / lifetime-mode), 72 vitest (без изменений от v0.32.0), svelte-check 0/0/0 на 493 файлах. T-000114 subagent behavioral verification (3 параллельных general-purpose агентов на симулированных user-prompts → проверка template rule-routing) — все PASSed, refinement rule-текста не понадобился.

## [0.32.0] — 2026-05-15

Три точечные задачи, закрывающие UX-пробелы от v0.31.0 dogfood'а + одна documentation-driven фича для Tasks-вью. Всё в одну вечернюю сессию.

### Added
- **T-000109** SemVer-aware колонка «Версия» в TasksTab + симметричный апгрейд в DoneTab. Rust-парсер todo.md теперь трекает `## vX.Y.Z — <описание>` заголовки как release-grouping signal и присваивает наследуемую версию каждой задаче ниже до следующего такого заголовка. Не-version `##` заголовки (`## Format`, `## Backlog` и т.п.) игнорируются — только `v<digit>...` активирует signal. Задачи выше первого version-заголовка несут пустую версию. `parse_todo_tasks` и `TodoTask` получают поле `version: String`; sync пишет это в `tasks.version` на insert И обновляет existing rows когда задача переезжает между version-секциями (без event'а в `task_events` — это metadata shift, не workflow-переход). DataGrid обзаводится `sortCompare?: (a, b) => number` колоночным hook'ом для произвольных компараторов. Новый `src/lib/utils/semver.ts` экспортит `compareSemVer` (парсит `MAJOR.MINOR.PATCH`, сортирует pre-release tags перед матчинговым релизом, fallback на `localeCompare` для non-semver значений; null/empty в конец). TasksTab получает новую «Version» колонку (sortable, text-filter, SemVer-aware sort); existing version-колонка DoneTab переключена на тот же comparator. Convention задокументирована в `_global/claude.md.global.tmpl` line 50 — plain markdown reader'ы по-прежнему видят заголовки как комментарии; tool'ы вроде Solo Dev Hub могут opt-in'нуть в release-grouping интерпретацию.
- **T-000110** `auto_detect` runner + `value_if_match` predicate-режим. Раньше `auto_detect` блок в meta.json был dead-spec — декларирован на `NODE_VERSION` / `GO_VERSION`, но никогда не исполнялся, юзер всегда получал static `default`. Новый pure-функция runner `src/lib/api/auto-detect.ts` читает файлы через инжектированный `readFile` callback (бекенд — существующая `readRepoFile` Tauri-команда) и применяет regex. Два режима: capture (старый — group `[1]` становится значением; пример: `NODE_VERSION` из `.nvmrc`) и predicate (новый — `value_if_match` static string при попадании regex'а; пример: `PRE_BUILD_COMMAND = "npm run paraglide:compile"` если `package.json` содержит `"@inlang/paraglide-js"`). `path` может быть одним string'ом или массивом — runner перебирает по порядку, останавливается на первом match'е. `DeployScreen.loadRepoConfig` вызывает runner для каждого repo-scope placeholder'а с `auto_detect` И пустым stored value, persist'ит результат на первом detect'е через `setRepoDeployConfig`. Override юзера сохраняется (непустые значения skip'аются); re-detection фиксируется только если поле очистить. `vite_static/meta.json` дополнен: `PRE_BUILD_COMMAND` получил predicate-mode auto-detect для Paraglide; `BUILD_OUTPUT_DIR` получил capture-mode auto-detect из `vite.config.{js,ts}` или `svelte.config.js`.

### Fixed
- **T-000111** Secrets-parser bare-multiline hint. Когда `parseEnvText` ловит `missing '='` и на этом run'е уже распарсен секрет, ошибка теперь читается `Line N: looks like a multi-line value for 'SSH_KEY'. Wrap it in triple quotes: SSH_KEY="""<newline>...<newline>"""` вместо generic-сообщения. Generic preserved когда никакого секрета ещё не распарсено (orphan-line-first случай). Hint срабатывает один раз per run-on — `prevSecretName` ресетится после emit'а. Pattern dogfoodнут 2026-05-14 на T-000107 деплое, где bare-multiline SSH-key вставка тихо фейлила с generic'ом.

### Tests
- 354 cargo (350 → +4 от T-000109 parser + sync logic), 72 vitest (50 → +3 T-000111, +10 T-000110, +9 T-000109 semver), svelte-check 0/0/0 на 493 файлах (было 489 — добавлены `auto-detect.ts` + `semver.ts` + 2 новых test-файла).

## [0.31.0] — 2026-05-14

Переделка deploy-конфигурации + третий встроенный шаблон. Multi-env dogfood Go-проекта в v0.29.2 выявил, что repo-wide placeholder'ы (`GO_VERSION`, `BINARY_NAME`, `ENTRY_POINT`, `APP_PORT`) хранились per-env в `extras{}` — каждый env дублировал их, и расходящиеся значения давали «last Generate wins» сюрпризы, поскольку все они рендерят один общий `Dockerfile`. v0.31.0 чинит модель хранения (T-000103) и поверх неё выкатывает третью deploy-цель (T-000107).

### Added
- **T-000103** Split repo-wide и env-specific placeholder'ов. Новая колонка `repositories.deploy_repo_config TEXT NOT NULL DEFAULT '{}'` (миграция v25) хранит repo-scope значения один раз на репо; per-env `extras{}` остаётся только для env-scope. Миграция v25 читает `meta.json` каждого template'а из таблицы `templates`, находит placeholder'ы со `"scope": "repo"`, поднимает их value из первого env'а (по `sort_order ASC`), вычищает из всех env'ов. First-env-wins при расхождении значений с записью в `sync_events` и JSON conflict-деталями (`{"conflicts":[{"key":"GO_VERSION","kept_env":"prod","kept_value":"1.26-alpine","discarded":[{"env":"test","value":"alpine"}]}]}`). Idempotent: пропускает data-loop если `deploy_repo_config != '{}'`. DeployScreen получает collapsible-секцию «Общие настройки образа» над списком env'ов; DeployDetail фильтрует repo-scope placeholder'ы из своего loop'а. Per-key autosave on blur (mirror DeploySecretsTable чтобы избежать B-000009 потери фокуса). Sticky-header в DeployDetail (`position: sticky; top: 0; z-index: 10`). Schema-aware render merger `template_render::build_placeholder_vars` берёт `scope: "repo"` ключи из `deploy_repo_config`, `scope: "environment"` (default) — из `env.extras`. Activity feed (Timeline, RecentActivityFeed, DashboardActivityFeed) распознаёт `sync_type='migration'` с JSON-парсингом. +32 теста (314 → 346).
- **T-000107** Третий встроенный deploy-шаблон `vite_static` для Vite-based static SPA (Svelte/React/Vue/Solid + Vite → `nginx:alpine` через SSH push + NPM upsert). Архитектурный близнец `flutter_web` (тот же downstream); отличается только build stage'ом: `node:lts-alpine` + `npm ci` вместо Flutter SDK + dart-define. Новая derived value `@@DOCKERFILE_ENVS@@` (пятая в семействе `BUILD_ARGS` / `RUNTIME_ENV_ARGS` / `DOCKERFILE_ARGS` / `DART_DEFINES`) emits `ENV NAME=$NAME` per build-secret — обязательная ARG→ENV transition, потому что npm-процесс читает `VITE_*` из `process.env`, не из ARG-scope. Repo-scope placeholder'ы: `NODE_VERSION` (default `lts-alpine`, auto-detect из `.nvmrc`), `BUILD_OUTPUT_DIR` (default `dist`), `PRE_BUILD_COMMAND` (default `true` = shell no-op; выставлять `npm run paraglide:compile` для Paraglide-проектов). `deploy.yml.tmpl` — байт-копия `flutter_web/deploy.yml.tmpl` (SSH + NPM-машинерия общая). Reference shape: Digital-mech-lab landing. +4 теста (346 → 350).

### Changed
- **Scope vocabulary split (T-000103 Task 2).** Две scope-семантики сосуществуют в `meta.json`: `placeholders.<KEY>.scope ∈ {"repo", "environment"}` (default `"environment"`) и `required_secrets[*].scope ∈ {"deploy_repo", "environment"}` (без default — обязательное явное). Renamen `"deploy_repo"` (с pre-v0.31.0 `"repo"`) дезамбигуирует «это Repository Secret в GH Actions, не Environment Secret» от «этот placeholder рендерит в один shared repo-wide файл». Pre-v1.0.0, не было shipped юзеров → без back-compat shim. Кастомные template'ы с обсолетным значением фейлят загрузку с человеко-читаемым error'ом, указывающим точное поле.
- **Strict-mode валидация `meta.json` на seed + parse time.** `template_meta::validate_meta_json` запускается на каждый seed bundled template'а (`template_seeder.rs:42`); bundled-template с невалидным scope-значением фейлит app startup с понятным сообщением. `parse_meta_placeholders` и `parse_meta_secret_hints` — parser-эквиваленты, вызываемые из `render_files_for_deploy_env`; оба отвергают неизвестные scope-значения вместо тихого fallback'а. Фронтенд читает freeform `label` / `description` / `default` / `type` / `auto_detect` напрямую из raw JSON, без изменений.
- **Go template (`templates/go/meta.json`)** обновлён до `version: 5`. Четыре placeholder'а (`GO_VERSION`, `BINARY_NAME`, `ENTRY_POINT`, `APP_PORT`) теперь несут `"scope": "repo"`. Два NPM secret'а переименованы `"scope": "repo"` → `"scope": "deploy_repo"`.
- **Flutter_web template (`templates/flutter_web/meta.json`)** обновлён до `version: 5`. Два NPM secret'а переименованы `"scope": "repo"` → `"scope": "deploy_repo"`.

### Tests
- 350 cargo (было 314 в v0.30.x — +32 от T-000103, +4 от T-000107), 50 vitest (без изменений), svelte-check 0/0/0 на 489 файлах (file count не менялся — vite_static template'ы это bundle-assets, не TS-исходники).

## [0.30.1] — 2026-05-14

Второй проход механического рефакторинга поверх v0.30.0. Ещё три split'а — `export.rs` по доменам парсеров, `sync/claude_md.rs` разбит на три концерна, `i18n/translations.ts` разнесён по префиксам ключей. Pure лексический move, поведение не менялось. Тестовые наборы без изменений (314 cargo, 50 vitest, svelte-check clean — file count 454 → 489 из-за новых `i18n/strings/`).

### Changed
- **T-000104** `src-tauri/src/export.rs` (1123 строки) разнесён в `src-tauri/src/export/`: `mod.rs` (barrel) + `util.rs` (shared pipe-парсер, escape/unescape) + `bugs.rs` (v2 8-field generate/parse) + `bugs_legacy.rs` (pre-v2 import: `parse_header`, `parse_bug_entry`, `parse_markdown_legacy`) + `todo_done.rs` (F-021 todo/done парсеры). Commit `4ca36e0`.
- **T-000105** `src-tauri/src/sync/claude_md.rs` (872 строки, три концерна) расщеплён: `claude_md.rs` (448 строк — только CLAUDE.md section), `project_md.rs` (238, новый — `generate_project_md` для cross-repo announcement Path lookup'ов), `gitignore.rs` (221, новый — `sync_gitignore_section`). Commit `0684235`.
- **T-000106** `src/lib/i18n/translations.ts` (1594 строки, ~727 ключей × ru/en flat objects) разнесён по префиксу ключа в `src/lib/i18n/strings/<domain>.ts` (35 файлов, по одному per top-level prefix). Каждый file экспортит `ru` + `en` срез своего домена. Корневой `translations.ts` (теперь 116 строк) merge'ит через spread. `TranslationKey` type-narrowing сохранён через `as const`. Commit `336ec38`.

### Tests
- 314 cargo (без изменений), 50 vitest (без изменений), svelte-check 0/0/0 на 489 файлах (было 454 — +35 `i18n/strings/*.ts`).

## [0.30.0] — 2026-05-14

Pre-v1.0.0 механический refactor-bundle. Шесть задач (T-000093/094/095/096/097/102) разнесли четыре самых больших Rust-модуля и TypeScript `types.ts` по доменам. Чистый лексический move + extraction — поведение не менялось. Все тестовые наборы остаются зелёными (314 cargo, 50 vitest, svelte-check clean).

### Changed
- **T-000093** Удалены 9 no-op Tauri-команд `*_stat` (`increment_bug_stat`, `decrement_bug_stat`, `add_attempts_stat`, `subtract_attempts_stat`, `increment_resolved_stat`, `transfer_bug_stat`, `reset_repo_stats`, `reset_all_stats`, `recalculate_all_stats`) — legacy-стабы со времён v0.16.0 миграции stats-table→VIEW: тела `Ok(())`, нет вызывающих. Удалены и фронтовые wrapper'ы в `tauri-commands.ts`. Оставшиеся 3 команды (`get_repo_stats_summary`, `get_project_stats_summary`, `get_project_graph`) под подчищенным комментарием-заголовком.
- **T-000094** `src-tauri/src/db.rs` (7 314 строк, 261 метод) разнесён в директорию `src-tauri/src/db/`: `mod.rs` (struct + ctor + free fns + `pub mod`) плюс 10 доменных под-модулей — `migrations` (967), `projects` (978), `repos` (1290), `bugs` (1081), `dashboard` (787), `deploy` (805), `tasks_events` (339), `stats` (427), `timeline` (406), `graph` (194). Несколько `impl AppDb` блоков в разных файлах; публичный API не менялся.
- **T-000095** Рефакторинг god-fn `run_migrations` (~530 строк, 24 inline-schema blob'а) → один диспетчер с массивом `(target_version, name, fn)` и per-version free fns (`mig_v1_initial` … `mig_v24_project_renames`). Каждая миграция — самостоятельная fn принимающая `&Connection`: читать проще, тестировать проще, добавлять новые проще. Per-migration тесты живут рядом со своей миграцией.
- **T-000096** Извлечён хелпер `run_count_with_project_filter(&self, base_sql, fixed_params, project_ids)` в `db/dashboard.rs`. Четыре counter call-site'а (`count_active_bugs`, `count_active_bugs_with_severity`, `count_closed_bugs_in_period`, `count_opened_bugs_in_period`) сжаты с ~10 строк `params_from_iter` + `extend(ids_refs)` boilerplate'а до 5–6 строк. Более сложные queries (avg-attempts, top-hot, bugs-per-day, category-efficiency) оставлены со своим SQL — туда хелпер не ложится чисто.
- **T-000097** `src-tauri/src/sync.rs` (3 054 строк, ~30 free fns) разнесён в `src-tauri/src/sync/`: `mod.rs` (16-строчный barrel) + 5 под-модулей — `fs` (path safety + file primitives, 334), `requirements` (rename-replay + nested-folder migration, 489), `claude_md` (CLAUDE.md / project.md секция rendering, 872), `bugs` (Bug MD↔DB sync, 900), `tasks` (Task MD↔DB sync, 502). Promotion'ы видимости не понадобились.
- **T-000102** `src-tauri/src/models.rs` (715 строк, 48 структур) разнесён в `src-tauri/src/models/`: `mod.rs` (barrel) + 10 под-модулей (`core`, `bugs`, `dashboard`, `deploy`, `graph`, `stats`, `sync`, `tasks`, `templates`, `timeline`). Фронтовый `src/lib/types.ts` (405 строк) свёрнут в 14-строчный barrel, re-export'ит из новых `src/lib/types/*.ts` зеркально Rust split'у. Все 40 импортов `from '$lib/types'` компилируются без изменений через flat `export *`.

### Tests
- 314 cargo (без изменений), 50 vitest (без изменений), svelte-check 0/0/0 на 454 файлах (было 444 — новые `types/*.ts` добавились в счёт).

## [0.29.2] — 2026-05-14

Hotfix-патч из multi-deploy Go dogfood-сессии. Шесть багфиксов и одно уточнение template-правила в sidebar, deploy-экране, dashboard'е и Go-шаблоне.

### Fixed
- **B-000006** DeployScreen drill-down state (открытый env detail) переживал смену репо — переключение на другой репо в sidebar'е продолжало рендерить env первого репо. `<DeployScreen />` обёрнут в `{#key repo.id}` — компонент пере-монтируется при смене репо, drill-down сбрасывается.
- **B-000007** Reorder ▲/▼ кнопки таргетили stale-selection сразу после создания нового проекта. `handleCreateProject` теперь очищает `selectedRepoId`, выставляет фокус на новый проект и открывает его экран — ▲/▼ становятся сразу actionable на только что добавленном проекте.
- **B-000008** Deploy-таб обрезал контент за viewport'ом при длинном списке env'ов или секретов. У Deploy-таба в `RepoDetail` отсутствовал scroll-контейнер (в Secrets/Stats табах он был); добавлен `.deploy-wrapper` mirror'ом этого паттерна.
- **B-000009** Ввод значения секрета: typing-then-tab в DeployTable per-secret инпуте запускал `await load()` после каждого сохранения — full-reload листа крал фокус у следующей textarea. Заменено на optimistic local update. SecretsPanel per-existing-secret edit переведён на тот же per-row autosave паттерн — два экрана теперь имеют одну механику.
- **B-000010** Сгенерированный workflow имел пустые `build-args:` и `docker run -e` lines даже после включения секретов. Корень: `ensure_deploy_secrets_populated` дефолтил unknown секреты в `role="deploy"` (ни build, ни runtime фильтр их не берёт). Поменян дефолт на `runtime` — meta hints уже покрывают деплой-инфра (SSH/NPM) и explicit build (Flutter API_BASE_URL); вне хинтов почти всегда app config. Также вынес `"-alpine"` суффикс из Go Dockerfile template в значение `GO_VERSION` placeholder'а (дефолт `"alpine"` = последний stable Go в alpine через Docker Hub auto-track); прежний `golang:@@GO_VERSION@@-alpine` ломался на пустом значении, `"latest"`, и double-suffix при вводе полного тега. Empty-required валидация: meta.json теперь принимает `"optional": true` per placeholder; DeployDetail подсвечивает пустые required поля красной рамкой и блокирует Generate. Go template: `migrations` COPY раскомментирован по умолчанию (Go веб-серверы обычно их embed'ят), `@@BUILD_ARGS@@` / `@@DOCKERFILE_ARGS@@` placeholder'ы прокинуты через `docker/build-push-action` и Dockerfile builder stage.
- **B-000011** Top-3 hot projects мета-строка ("0 crit / 0 maj / 2 act") был hardcoded английский. Переведён в i18n ключи (`крит` / `важн` / `актив`).
- **B-000012** Тот же stale-selection корень что B-000007 но через `openProject` и `clickProjectInCollapsed` — клик на соседний проект после reorder'а репо удерживал ▲/▼ на repo'шке. Обе функции теперь очищают `selectedRepoId` перед `selectedProjectId`.

### Changed
- **Уточнение в template'е** правила cleanup'а confirmed-багов в global CLAUDE.md — cleanup теперь срабатывает на user-signal ("посмотри баги", "I added bugs" и т.д.), а не "когда LLM в следующий раз правит bug-reports.md по любой причине". Confirmed-строки больше не висят между сессиями.

### Tests
- 314 cargo (было 311) — +1 каждый для default-role-runtime, GO_VERSION bare-alpine, GO_VERSION 1.26-alpine regression rename.
- 50 vitest, svelte-check clean.

## [0.29.1] — 2026-05-13

Patch-релиз. Фиксы UX ввода секретов во всех 3 entry-point'ах, чтобы multi-line значения (типичный SSH-key), inline `# comments` и значения в кавычках работали единообразно в bulk `.env` paste, в per-secret боксе репо-секретов и в per-deploy-secret override-боксе.

### Added
- `secrets-parser` теперь принимает dotenv-style single-line values: окружающие `"..."` / `'...'` кавычки strip'ятся, `\n \r \t \\ \"` escape-последовательности декодируются внутри double-quoted значений (single-quotes остаются literal'ом), inline `# comment` после значения отбрасывается если предшествует whitespace. Triple-quote `"""..."""` форма блока без изменений. SSH-ключ теперь можно ввести в одну строку через `\n`-escape'ы.

### Fixed
- DeploySecretsTable per-env override-value бокс был `<input type="password">` и не принимал multi-line paste вообще — SSH_KEY override был невозможен без создания пустого secret'а в репо-стороне и редактирования его там. Заменён на `<textarea>` с `-webkit-text-security: disc` маскированием, mirror'ом SecretsPanel per-secret-box. Два бокса теперь visually consistent.

### Tests
- +11 vitest case'ов в `secrets-parser.test.ts`: inline comments, quote stripping, escape decoding, single-vs-double-quote semantics, unclosed-quote errors, SSH-key one-row round-trip. 50 vitest total, svelte-check clean, 311 cargo без изменений.

## [0.29.0] — 2026-05-13

Pre-screenshot polish-бандл перед public launch. Два deferred-пункта прошлого ревью (P7, KPI/StatsSummary drift) закрыты; multi-deploy Go isolation покрыт интеграционными тестами.

### Added
- **T-000092** Таблица `project_renames` (migration v24) — симметрична `repo_renames`, но scoped'ом на проект, не на репо. `update_project` логирует смену имени; sync-preamble proигрывает её как `microservice-api/<old>/ → <new>/` на parent-сервер'е. `repo_renames` это не покрывал: папка keyed by `projects.name`, не canonical repo name. Идемпотентно по fs; collision (и `old/`, и `new/` существуют одновременно) surface'ится как ручное вмешательство. Flow-doc `docs/flows/api-handlers-sync.md` дополнен секцией rename-replay.
- Multi-env Go integration coverage — три новых теста в `render_deploy_tests`: same-repo prod+test рендерят env-isolated `deploy-{name}.yml` (network, branch, domain, container, runtime secrets), общий `Dockerfile` env-agnostic при совпадающих repo-wide placeholders (`GO_VERSION`, `BINARY_NAME`, `ENTRY_POINT`, `APP_PORT`).

### Fixed
- **T-000091** Dashboard KPI5 `avg attempts` (читает `bug_events.entered_testing`) расходился с per-repo / per-project `StatsSummary` (читает `bugs.fix_attempts`) после `migrate_bugs_for_repo` на свежедобавленном репо с готовым MD-контентом. Причина: `migrate_bugs_transactional` вставлял bug'и с `fix_attempts > 0`, но не создавал synthetic `bug_events`, а `backfill_bug_events_for_existing` имел глобальный one-shot guard и скипал последующие миграции. Фикс: синтезировать `created` + N×`entered_testing` + optional `confirmed` события внутри транзакции миграции, mirror'я backfill-логику. Инвариант `COUNT(entered_testing) == bugs.fix_attempts` теперь держится на всех entry-point'ах.

### Tests
- 311 cargo-тестов (было 308): +1 для T-000091 migration→events synthesis, +3 для T-000092 project_renames, +3 для multi-env Go isolation, +1 для v24 schema migration.

## [0.28.0] — 2026-05-12

Второй проход code-review после v0.27.1 — 32 находки в 4 доменах (bug/stats/dashboard, tasks/timeline/datagrid, cross-repo sync, deploy/secrets/settings). Закрыты 4 батчами: 3 critical + 8 high + 11 medium + 9 polish + 2 deferred (P7 microservice-api rename-replay требует новой схемы, P10 COMPOSE_SERVICE copy UX — дизайн-вопрос). Без новых фич, без migration'ов.

### Fixed (critical)
- **C1 | `confirm_requirement` удалял не ту microservice-пару при collision NNN** — ветка server→MS итерировала всех connected microservices и удаляла из первого где filename match'ил, без disambiguation по target. Каждая MS-папка ведёт свой NNN-счётчик, два MS могут независимо иметь `REQ-001.md` — клик ✓ на одной строке стирал sibling-пару. Tauri-команда теперь принимает `target_repo_id`, frontend SyncScreen.handleConfirm резолвит и source и target через `getDisplayName`. Включает M8: source role не `client*`/`server` (e.g. `tool`/`landing`) теперь возвращает explicit error вместо silent no-op'а.
- **C2 | `valid_transition` пропускал LLM `testing → confirmed`** — функция whitelist'ила transition хотя комментарии настаивали что path UI-only. LLM пишущий `status: confirmed` в `bug-reports.md` обходил user-verification gate. Удалены `testing → confirmed` и `testing → rejected` из whitelist'а; `confirmed_at` теперь устанавливается только через UI `resolve_bug`. Тест переименован в `test_reconcile_rejects_testing_to_confirmed_from_md` asserting новый guard.
- **C3 | `DeployDetail.load()` early-exit'ил когда `repo` не reactive** — `repo` это `$derived` от `$allRepos`, загружающийся параллельно и может быть not-yet-ready на mount'е. Предыдущий `if (!env || !repo) return` silently скипал GitHub Environment auto-ensure и branches fetch без retrigger'а. Split на env-fetch на mount + `$effect`-driven GitHub-side bootstrap firing один раз когда оба resolved.

### Fixed (high)
- **H1** `refreshBugs` теперь skip'ит reconcile для remote-only репо (no `local_path` → `bugs_migrated_at IS NULL` → error toast на каждый bug-tab remount). Store трекает `currentRepoHasLocalPath` рядом с `currentRepoId`.
- **H2** Удалены legacy `backend` / `network` из `BugItem.defaultCategories` (не в 9-value DB CHECK enum с v0.13.12 — выбор давал raw SQLite error toast). Добавлен `auth` взамен.
- **H3** Timeline `kind` / `repo_ids` / `project_ids` filters push'нуты в SQL `WHERE` clause до `LIMIT/OFFSET`. Раньше Rust фильтровал после fetch'а — когда большинство rows filtered out, frontend видел `r.length < PAGE_SIZE` и stop'ил pagination с matching events на дальнейших страницах. `search` substring остаётся в Rust.
- **H4** DoneTab date column + default sort с `created_at` (task-creation, часто months before completion) на `updated_at` (set'ится `update_task_source` на todo→done transition).
- **H5** Historical done.md entries с empty `dt.date` (no section header context) теперь fallback'ятся на `done.md` mtime вместо `todo.md` mtime — entry родился в done.md, не todo.md.
- **H6** `sync_tasks_for_repo` теперь resolve'ит split-state когда тот же `task_id` существует в обоих `todo` и `done` sources (post-crash или manual MD edit). `done` wins т.к. отражает later intent. +1 unit test.
- **H7** `delete_pat` также wipe'ит legacy keyring entry (`github-repo-manager`). Без этого `migrate_legacy_pat` resurrect'ил deleted token на next cold start.
- **H8** `write_deploy_files` Timeline event записывает `written.len()` вместо `files.len()` — path-rejects + write failures больше не inflate metric. Event details migrate'нуты на `serde_json::json!` для consistency.

### Fixed (medium UX)
- **M2** BugItem comment row visible когда comment непустой, не только `fix_attempts > 0`. Раньше comment set в `created`/`in-progress` был invisible до first testing transition.
- **M3** Timeline убран double `loadFirstPage` на deep-link mount — `$effect` уже fire'ит на initial mount.
- **M4** DataGrid filter dropdown закрывается на outside-click + Esc через `svelte:window` listeners.
- **M5** Server's `docs/api.md` отсутствующий при client sync больше не push'ится в `errors` — silent skip, симметрично с `handlers.md`.
- **M6** `init_docs_for_repo` surface'ит `"(project.md + CLAUDE.md skipped — repo has no project assigned)"` в result list для orphan репо.
- **M7** `replay_rename_in_dir` возвращает `RenameOutcome { Renamed, NoOp, Collision }` enum вместо ambiguous `bool`. Callers surface collisions как explicit warnings.
- **M9** DeployDetail Generate button отражает workflow-stale state после secret role changes (build/deploy/runtime cycle). Amber tint + «Перегенерировать workflow-файлы» label + tooltip. `DeploySecretsTable` принимает `onRoleChange` callback prop.
- **M10** DeployDetail surface'ит YAML-unsafe-value warning перед Generate button когда placeholder values содержат chars ломающие YAML в unquoted scalars (`:`, `#`, quotes, backticks, newlines, leading flow-indicator).
- **M11** Updater silent-mode preserve'ит error category для `network` / `signature` / `unknown` — только `notFound` (expected на приватном репо до public-flip) остаётся тихим, чтобы About card мог surface real errors на next user-initiated check.

### Fixed (polish)
- **P1** `addBug` store default severity 'minor' → 'medium' aligned с UI call site.
- **P2** Comment на `active_bugs` KpiCard объясняющий intentional absence compare-period delta (point-in-time metric).
- **P3** `DashboardTopHot` meta line показывает `major` count рядом с `critical` и `active` — backend sort weighs critical → major → active.
- **P4** DataGrid search placeholder migrate'нут на i18n key `grid.searchPlaceholder` (ru + en).
- **P5** `parse_done_entries_in_period` принимает legacy `DD.MM.YYYY` / `DD/MM/YYYY` date headers (matching `parse_done_tasks` tolerance). Normalize'ит в `YYYY-MM-DD` для range comparison.
- **P6** «No clients found» warning suppressed когда server тоже отсутствует — server-only build-out phase это legitimate state, без warning spam'а.
- **P8** `SyncScreen.loadRequirements` migrate'нут с `onMount` на `$effect(projectId)` — reload'ится на project change без unmount.
- **P9** `AppDefaultsScreen.excludeFiles` moved в module-level const чтобы `TemplateEditor.$effect` не re-fire'ил на каждый parent render.

### Deferred
- **M1** Dashboard KPI5 vs `StatsSummary` avg attempts drift — теоретический, только после `migrate_bugs_for_repo` без backfill `entered_testing` events. Требует structural rework. → v0.29.0.
- **P7** `microservice-api/<project-name>/` rename-replay — требует `project_renames` таблицы (`repo_renames` repo-scoped only). → v0.29.0.
- **P10** COMPOSE_SERVICE copy-from-CONTAINER_NAME direction — design call, лучше tooltip; future Deploy UX pass.

### Tests
- 303 cargo passing (+1 от H6 split-state test, net после C2 test rename)
- svelte-check 444 / 0 errors / 0 warnings

## [0.27.1] — 2026-05-12

Patch-релиз: code-review фиксы — 2 критичных бага + 5 important issues + 1 cleanup. Никаких новых фич, schema без изменений.

### Fixed
- **Critical | ✓ Confirm-кнопка в SyncScreen молча no-op'ит для всех GitHub-backed репо** (регрессия с v0.25.0). После фикса B-000001 `Repository::display_name()` стал возвращать last segment `github_name` (`web-app-client`, не `owner/web-app-client`), но `findRepoId` в [SyncScreen.svelte:57](src/lib/components/SyncScreen.svelte#L57) всё ещё матчил против полного `r.github_name`. Итог: каждый клик ✓ на GitHub-репо возвращал `null` и выходил молча — backend никогда не вызывался. Фикс: заменить сравнение на `getDisplayName(r) === name` чтобы TS-сторона зеркалила Rust-семантику. Local-only репо случайно работали через description fallback.
- **Critical | Path traversal в `write_deploy_files`** — `meta.json` `file_targets` join'ился к repo root без проверки на `..`-escape или абсолютные пути. User-edited template через TemplatesScreen мог писать за пределами репо. Новый helper [sync::is_safe_subpath](src-tauri/src/sync.rs) отклоняет абсолютные пути, drive letters, `..`, root-component пути. Применён в [write_deploy_files](src-tauri/src/lib.rs#L2553) (write loop) и в [read_repo_files](src-tauri/src/lib.rs#L2532) (симметричный read-side). +4 unit-теста (`test_is_safe_subpath_accepts_normal` / `_rejects_parent_dir` / `_rejects_absolute` / `_rejects_windows_absolute`) → 302 total.
- **`NaiveDate::succ_opt().unwrap()` panic risk** в Dashboard daily flow loops — заменено на `match ... break` в [lib.rs:1021](src-tauri/src/lib.rs#L1021) и [db.rs:2951](src-tauri/src/db.rs#L2951). Malformed или far-future date filter (`9999-12-31`) в итоге переполнял бы и панил IPC thread.
- **JSON injection в event details column** — `record_deploy_secret_event` и `record_secret_event` интерполировали `secret_name` / `action` напрямую в raw JSON через `format!`. Заменено на `serde_json::json!({...}).to_string()` — кавычки и спецсимволы в input больше не corrupt'ят сохранённый JSON.
- **`SyncResult.errors` false positives для non-standard проектов** — `sync_project` безусловно push'ил «No server found» / «No clients found» даже для microservice-проектов (которые intentionally без сервера/клиентов). Теперь scope'нуто на `project_type == "standard"`. UI warning-toast'ы перестают firing'ить на каждый sync microservice проекта.
- **`migrate_flat_to_nested` partial-copy rollback** (sync.rs Case C, multi-parent same-content ветка) — если `fs::copy` падал mid-loop после успешных более ранних копий, код early-return'ил оставляя ghost files в некоторых parent subfolder'ах. Теперь rollback'ает successful копии и emit'ит warning, оставляя flat source intact для retry на следующем sync.
- **`read_repo_files` принимал raw `local_path` от frontend** — рефакторнуто на `repo_id` с lookup'ом из DB (mirror'ит shape `read_repo_file`). Закрывает wider-than-necessary read surface; frontend caller в [DeployDetail.svelte:176](src/lib/components/DeployDetail.svelte#L176) обновлён передавать `repo.id`.

### Removed
- Dead TypeScript interface `DeployManifest` в [types.ts](src/lib/types.ts) — Rust-эквивалент был заменён на `DeployEnvironment` в v0.18.0. Импортов больше не было.

## [0.27.0] — 2026-05-12

### Changed
- **T-000081 | Project CLAUDE.md full refactor** — раздел «Ключевые решения»
  с inline-историей v0.16→v0.25 сжат в компактную «Эволюцию» с
  одностроками per-version (детали и так в Changelog). Stale-link cleanup:
  product display name «GitHub Repo Manager» → «Solo Dev Hub», autoupdate
  endpoint URL обновлён на `solo-dev-hub` repo, устаревшие `docs/doc1_global_rules.md`
  / `doc2` / `doc3` ссылки заменены актуальными путями (`docs/flows/`,
  `docs/formats/`, global template). Test count re-baselined на 298.
  Components-таблица консолидирована (Dashboard sub-components сгруппированы
  в одну строку). Aligned с section taxonomy глобального шаблона.
- **T-000089 | Changelog EN + RU split** — переименовали `Changelog.md`
  (русский) в `Changelog.ru.md`, создан английский primary `Changelog.md`
  для public-аудитории. Pre-v0.16 версии в EN сжаты до одностроков
  (исторический контекст полностью сохранён в RU-mirror'е). EN — основной
  changelog; RU поддерживается параллельно. Аналогично паттерну
  `README.md` (EN) + `README.ru.md` (RU). `docs/RELEASING.md` обновлён —
  release closure теперь требует обновления обоих файлов.
- **T-000090 | Template: explicit reverse-direction disclaimer для REQ** —
  усиление `# Cross-repo requirements` секции global CLAUDE.md template
  после обнаружения misuse в живой сессии (server-LLM написал «REQ для
  админки» и положил его в `docs/client-requirements/<client>/` инверсно
  — фактически это был announcement). Изменения: (1) в `## Folders`
  после flat/nested explainer'а добавлено explicit «Reverse directions
  (server → client, microservice → parent server) do not exist as REQ —
  sender-initiated changes flow through announcements». Симметрично уже
  существующему disclaimer'у в `## Directions` announcement-секции.
  (2) В `## LLM policy > LLM must NOT:` добавлен новый bullet —
  «author REQ-*.md в собственной recipient-folder (server's
  client-requirements/, MS's server-requirements/) — те folders'ы
  заполняются Solo Dev Hub'ом из sender-outgoing, не вручную; REQ там
  не пропагируется и будет принят за incoming-request». Server→client
  и MS→server инициативные сообщения = announcement, не REQ.

### Added
- **T-000084 | GitHub repo description + topics** — pre-public-launch
  SEO/discovery подготовка. Применено через GitHub web UI 2026-05-12
  (Settings → About). Description: «Solo developer's portfolio cockpit.
  Bugs, requirements, deploy — all in markdown.» (matches README hero,
  80 chars, в пределах GitHub 350-char limit). Topics (12): `tauri`,
  `svelte`, `sveltekit`, `rust`, `project-management`, `github`,
  `solo-developer`, `indie-dev`, `bug-tracker`, `deploy-automation`,
  `developer-tools`, `desktop-app`. Последние два добавлены по
  замечанию second-agent для broader discoverability. Топики работают
  и на приватном репо; станут визуально-доступными после public flip
  (T-000064 в v1.0.0).

## [0.26.1] — 2026-05-12

### Added
- **F-000040 | Cross-repo announcements (proactive push)** — новая секция
  в global CLAUDE.md template для unsolicited информации, не помещающейся
  в REQ/receipt pattern. 2 направления: server→client (sender пишет
  напрямую в `<client-path>/docs/server-announcements/<sender-canonical>/ANNOUNCE-NNN_<slug>.md`)
  и microservice→parent-server (зеркально, в `docs/microservice-announcements/<ms-project>/`).
  Recipient читает + удаляет = implicit ack (audit trail в git history
  recipient'а). No app-side sync, no receipts, no confirm-✓. Sender
  получает recipient'ский local path из собственного `docs/project.md`:
  `## Repositories` Path column (для clients) или `## Parent projects`
  (для parent servers, теперь с path после расширения `generate_project_md`).
  Если path отсутствует ("no local path configured") — announcement не
  деливерится, surface в own todo. Explicit carve-out из правила
  "LLM never copies across repo boundaries" — для announcements это
  разрешено, для REQ — нет. Use cases: server self-initiated change
  affecting client (e.g. new admin endpoint requires client integration);
  side-effect change affecting other clients; post-internal-review
  rework affecting client; MS-side change affecting parent. **NOT** для
  "client asked → server did → integration notes" — то идёт в REQ
  receipt `## Comment:`.

### Changed
- **`docs/project.md` Parent projects section** теперь включает локальный
  path parent server-репо: `- **<parent-name>** — server repo: <name>
  (path: <local-path>)` или `(no local path configured)`. Источник —
  `db.server_repo_of_microservice(parent_id)` + `.local_path`. Нужно для
  F-000040 announcement push'а MS→server: MS-LLM получает целевой
  filesystem path из своего project.md без cross-repo sync infrastructure.
  +2 unit-теста (`test_generate_project_md_microservice_parent_includes_server_path`,
  `test_generate_project_md_microservice_parent_without_server_path`) → 298 total.
- **T-000086 | F-000040 template clarifications** — два уточнения в
  `# Cross-repo announcements` секции global CLAUDE.md template после
  pilot-проверки subagent'ом, выявившей места требующие интерпретации.
  (1) **NNN counter behavior** — explicit правило для непустой folder:
  использовать `max(existing NNN) + 1`; NNN — monotonic counter, не
  slot allocator. Закрывает кейс когда часть entries ранее acknowledged
  и удалена — новые номера всё равно идут после максимального
  использованного, не переиспользуют свободные слоты. (2) **Threshold:
  actionable impact required** — новый `### Threshold` подраздел после
  основной таблицы "When to use". Strict criterion: announcement
  оправдана только если recipient должен принять действие (изменить
  код / конфиг / поведение). Чистое существование sender-side изменения
  (e.g. новый admin-only endpoint) — недостаточно; pure surface
  additions идут через `docs/api.md` sync. Positive criterion (helper
  для unsure-sender'ов): если recipient должен изменить код/конфиг/
  поведение чтобы продолжать работать — пиши announcement; если может
  работать дальше и подобрать изменение через api.md позже — не пиши.
  Rationale: чрезмерные announcements обесценивают канал.
- **T-000085 | docs/flows/cross-repo-announcements.md** — новый flow
  doc для F-000040 announcement channel, mirror'ом структуры
  `microservice-server-sync.md`. Содержание: модель (one-way push, 2
  направления, no app-side sync), таблица "Когда использовать vs REQ
  receipt" + threshold/positive criterion summary, lifecycle (server→client
  пример с rate-limit header), microservice→parent server flow (с
  упоминанием project.md path lookup и v0.26.0 расширения), сценарии
  "когда уместен" (sender-initiated / side-effect / post-review-rework
  / deprecation) vs "когда НЕ уместен" (admin-only surface / silent fix
  / reactive case / reverse REQ / undefined impact), таблица "Где
  Solo Dev Hub помогает / не помогает" (app участвует только в
  generate_project_md), carve-out из no-cross-repo-writes rule с
  обоснованием через асимметрию flow, cross-reference на нормативные
  H1/H2/H3 в global CLAUDE.md template'е и на `microservice-server-sync.md`
  § Triangular flow для REQ-based флоу.

### Changed
- **T-000088 | RepoDetail header — 2-ряд chip layout** —
  шапка переписана после iterative design review с ui-ux-pro-max
  скиллом (вариант C). Заменяет первоначальный 5-ряд layout на
  компактный 2-ряд chip-based: (1) `[lang] last-pushed · 📁 path
  [папка]` с правой стороны `[📚 Обновить документацию]`; (2) editable
  chips `[Проект: ▼] [Роль: ▼] [Шаблон деплоя: ▼]` с правой стороны
  `🗑 Удалить`. Удалено: back-button «Назад к Дашборду» (Dashboard
  доступен через top-bar + sidebar, кнопка была redundant + mislabel —
  всегда вела в Dashboard, не back), header-top cluster с
  role-badge/project-tag (значения дублировались с chips ниже),
  описание `repo.description` (низкий signal, шумит шапку), derived
  `roleLabel`/`roleIcon`/`projectName` (больше не используются),
  импорт `ROLE_ICONS`. Editable controls стилизованы как pill chips
  (radius 14px, surface bg, hover'ом accent border) — native `<select>`
  inside transparent-styled. Delete button — ghost (transparent
  border в покое, danger-border на hover'е, без слова «репозиторий» —
  контекст из шапки). New CSS: `.chip`, `.chip-label`, `.chip-select`,
  `.row-action`, `.meta-dot`. Removed CSS: `.header-top`,
  `.header-right`, `.back-btn`, `.repo-desc`, `.settings-row`,
  `.actions-row`, `.meta-pair`, `.meta-label`, `.role-badge`,
  `.project-tag`, `.inline-select`. HTML-preview итераций в
  `docs/superpowers/plans/2026-05-12-repo-detail-header-variants.html`.
  Параллельно тот же подход применён к **ProjectDetail** — удалена
  back-кнопка «← Назад» (вела в Dashboard hardcoded; redundant с
  top-bar + sidebar) включая non-found state, удалён `header-top-row`
  div, функция `goBack()`, CSS `.header-top-row` / `.back-btn`.
  Чистка мёртвых i18n keys: `repoDetail.backToRepos`,
  `repoDetail.backToReposTooltip`, `project.backToRepos` (ru + en).
  Adaptive narrow-window behaviour: chips получили `white-space:
  nowrap` + `flex-shrink: 0` (label «Шаблон деплоя:» больше не
  wrap'ится на 2 строки в узком окне). Action-кнопки сворачиваются
  в icon-only через container query `@container repo-header
  (max-width: 760px)` — `.sticky-header` объявлен named
  size-container, ниже порога `.row-action .btn-label` скрывается,
  остаётся только icon + tooltip. Текст «Обновить документацию»
  и «Удалить» обёрнут в `<span class="btn-icon">` + `<span
  class="btn-label">`. Emoji `📚` убран из i18n value
  `repo.initDocsButton` (теперь в template как icon-span).
- **Dark theme contrast в native `<select>` dropdown'ах** — добавлен
  `color-scheme: dark` на `:root` / `[data-theme="dark"]` и
  `color-scheme: light` на `[data-theme="light"]` в `app.css`. Это
  стандартная подсказка браузеру использовать тёмное native UI для
  scrollbars, dropdown'ов, datepicker'ов. Раньше native `<option>`
  popup рендерился WebView2 в OS-default white-bg → серый текст
  становился нечитаемым. Дополнительный fallback `option { background:
  var(--bg); color: var(--text); }` на случай платформ не уважающих
  color-scheme.
- **T-000080 / T-000079 | Deploy переехал из отдельного экрана в таб
  RepoDetail** — раньше `Deploy` открывался отдельным screen-route'ом
  через кнопку 🚀 в шапке RepoDetail (`currentScreen.set({name:
  'deploy'})`). Архитектурно это master-detail внутри одного репо
  (список deploy-инстансов → drill-down per-env), поэтому логичнее
  жить рядом с Bugs/Tasks/Done/Changelog/Secrets/Stats. Tab вставлен
  между Changelog и Secrets. Symmetry с другими табами: state локальный
  (`$state<Tab>`), drill-down (выбранный env) — внутри DeployScreen, не
  в ui-store, при tab-switch сбрасывается. Закрывает T-000079 (контекст
  репо очевиден из tabs-nav + RepoDetail header). Изменения: drop
  `'deploy'` из `ScreenName` union в ui-store, drop route в
  `+page.svelte`, drop back-button + H2 header в `DeployScreen.svelte`,
  drop `openDeploy()` + 🚀 кнопку + `.deploy-btn` стили в RepoDetail,
  drop dead i18n keys (`deploy.back`, `deploy.deploymentsTitle`,
  `repo.deployButton`), новые i18n keys (`repo.tabDeploy`,
  `repo.deployBlocked`) × ru/en. Empty-state если репо без
  `github_name` или `deploy_target` (с инструкцией указать в шапке).

## [0.25.0] — 2026-05-12

### Added
- **T-000078 | Triangular REQ-flow rules в global CLAUDE.md template** —
  расширение секции `# Cross-repo requirements` для случая client → server
  → microservice. Две новые H2 секции: `## Receipt format` (4 hard-enforced
  status values: `implemented` / `partially` / `declined` / `clarification-needed`
  + workflow перезаписи receipt'а при clarification-loop) и `## Forwarding
  (triangular flow)` (Server-side responsibility — classify/forward/wait/
  resume; Linkage header `Forwarded-from: <client>/REQ-NNN` для chain-tracing;
  MS-side responsibility — игнор client identity, audience-via-body). LLM
  policy расширен правилом про игнор `Forwarded-from:` header'а как server-
  side metadata. Sync flow — disambiguation: final receipt + unhappy sender
  пишет REQ-N+1; `clarification-needed` receipt → sender обновляет
  оригинальный REQ inline. Старый H3 `### Receipt content (convention, not
  enforced)` удалён — clash'ил с hard-enforce Rule 4. `### Sync state`
  промоутнут из H3 в H2 (потерял parent). Behavioral validation через
  5 fresh subagent-сценариев на pre-impl drafts (all PASS, coverage: 4/4
  rules, 4/4 status values, 4 edge cases — multi-MS, clarification-loop,
  audience leak, resuming across sessions). Flow doc
  `docs/flows/microservice-server-sync.md` дополнен секцией Triangular flow
  (lifecycle + Solo Dev Hub vs LLM responsibility таблица).
- **T-000075 | `CONTRIBUTING.md`** — pre-public-launch artifact: build
  prerequisites (Node v18+, Rust, MSVC Build Tools, WebView2), getting
  started, project layout, code style (Rust + TS/Svelte), commits
  (Conventional Commits), tests, PR rules (target `dev`, не `master`),
  AI-agent section (link to CLAUDE.md), releases (link to RELEASING.md).
- **T-000076 | `.github/FUNDING.yml`** — `custom: [boosty.to/sgonnovdm/donate]`
  для появления Sponsor-кнопки на repo header после public-flip'а. TON-кошелёк
  остаётся в README + About (FUNDING.yml не поддерживает crypto).
- **T-000062 | README RU+EN drafts (public-launch quality)** — text-only
  pass: marketing-tone преамбула (3 абзаца: tagline / problem+AI-failure-
  mode kicker / solution), AI-bug-closure with safety net как #1 фича
  (4 гарантии: protected fields, auto-attempts counter, explicit user
  confirm, full event log), Why / Features / Tech Stack / Getting started
  / Development / Roadmap / Support / License, ссылка на русскую версию
  `README.ru.md` сверху английской. RU-tagline — «личный пульт управления»
  (EN — «cockpit»). Скриншоты вынесены в T-000073 (placeholder'ы с
  captions расставлены в обоих файлах).
- `LICENSE` — MIT, ранее только в `package.json` без файла.

### Changed
- **T-000063 | Technical identifier rebrand**: Cargo `[package].name`
  (`github-repo-manager` → `solo-dev-hub`) + `[lib].name`
  (`github_repo_manager_lib` → `solo_dev_hub_lib`) + `main.rs` call site,
  `tauri.conf.json` identifier (`com.user2.github-repo-manager` →
  `com.solodevhub.app`), `package.json` name + regenerated `package-lock.json`.
  DB path migration `%LOCALAPPDATA%\github-repo-manager\data.db` →
  `%LOCALAPPDATA%\solo-dev-hub\data.db` через copy-once на первом старте
  (idempotent, legacy остаётся как recovery breadcrumb). Keyring service
  rename `github-repo-manager` → `solo-dev-hub` через `migrate_legacy_pat()`
  (read legacy → write new → delete legacy, idempotent, best-effort).
  Autoupdate-break: v0.24.x → v0.25.x проходит как fresh install (новый
  identifier = новый Windows app entry); v0.25.x → v1.0.0 чисто.
- **T-000061 | Display-name rebrand "GitHub Repo Manager" → "Solo Dev Hub"**:
  `productName` в `tauri.conf.json` + window title + Cargo description
  + auto-generated MD footers + i18n строки `appDefaults.syncGlobalConfirm`
  (ru+en) + About `githubUrl` + README заголовок + RELEASING.md / formats /
  deploy_template_spec / release.yml releaseName.
- **Autoupdate endpoint** → `https://github.com/SgonnovDmGit/solo-dev-hub/releases/latest/download/latest.json`.
  Репозиторий приватный до v1.0.0 public-flip'а — `latest.json` без auth не
  отдаётся, autoupdate приостановлен на v0.25.x. Pubkey уже свежий
  (сгенерирован в T-000059).
- **T-000060 | Release flow на master/dev split**: все коммиты цикла идут
  на `dev`, релиз = fast-forward в `master` + тег. Hotfix'ы прямо в
  `master`, потом `git rebase master` на `dev`. Документировано в
  `docs/RELEASING.md` секцией "Ветки" + suggested git aliases.
- **CI lint**: убрана env-строка `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` из
  `.github/workflows/release.yml` (текущий ключ solo-dev-hub без пароля).
  Секрет помечен как опциональный в `docs/RELEASING.md`.
- **F-000037 | Deploy: CONTAINER_NAME из secret → placeholder** + UI rework.
  Раньше CONTAINER_NAME лежал в GitHub Environment secrets — но это не
  secret-данные, просто имя контейнера per-env. Перенесён в placeholder
  (`extras` JSON в SQLite per-deploy_env). `${{ secrets.CONTAINER_NAME }}`
  в deploy.yml.tmpl (3 места × 2 шаблона) → `@@CONTAINER_NAME@@`. После
  следующего "Generate workflow files" значение запекается прямо в
  `.github/workflows/deploy-{env}.yml` (visible в репо — но имя контейнера
  и так публично из домена/labels, не raises rsks). Logical placeholder
  reorder в meta.json: WORKFLOW → IMAGE_TAG → DOMAIN → DEPLOY_BRANCH →
  NETWORK_NAME → CONTAINER_NAME → COMPOSE_PROJECT → COMPOSE_SERVICE →
  (язык-specific). UI: copy-кнопка ↩ справа от Service Label (99%
  случаев = container name). REQUIRED_KEYS gate (Generate-button) теперь
  включает CONTAINER_NAME. Migration: legacy `CONTAINER_NAME` secret в
  GitHub Environment остаётся orphaned (вреда нет), user заполняет
  placeholder в DeployDetail. Stale `CONTAINER_NAME_PROD` references в
  deploy_template_spec.md + flows/deploy-flow.md убраны.

### Fixed
- **GitHub Environment auto-ensure on DeployDetail open** — линтер GH Actions
  ругал `Value '<env-name>' is not valid` для workflow с `environment: <name>`,
  если соответствующий объект Environment в `Settings → Environments` не
  существовал. Несколько кейсов когда GH-side env отсутствовал: (1) legacy
  envs от migration v20 (`deploy_manifests` → `deploy_environments`)
  автогенерились с `name='prod'` в DB, но `createEnvironment` PUT не звался;
  (2) если у env нет ни одного override-secret'а, неявная триггер-цепочка
  через `createOrUpdateEnvironmentSecret` тоже не срабатывала, env оставался
  только в DB. Фикс: `createEnvironment(owner, repo, env.name)` идемпотентно
  вызывается в `DeployDetail.load()` (на mount) — covers все entry-paths
  (open existing / clone / fresh create). PUT — no-op если уже существует.
  Surface API-ошибок через warning-toast (с i18n key `deploy.envCreateFailed`)
  чтобы PAT permission issues / fine-grained PAT без "Environments: write"
  были диагностируемы.
- **Deploy YAML build-args indent (10→12 пробелов)** — `render_build_args`
  в [template_render.rs](src-tauri/src/template_render.rs) джойнил multi-secret
  через `\n          ` (10 пробелов). В шаблоне `@@BUILD_ARGS@@` стоит на
  12 пробелах под `build-args: |`. На >1 секрете второй и далее вылетали на
  10 spaces — становились sibling'ами `build-args` вместо continuation, YAML
  ломался: `APP_API_KEY: "${{ secrets.APP_API_KEY }}"` интерпретировался как
  отдельный ключ соседнего map'а. Pre-fix комментарий в коде врал ("Indent =
  10 spaces … matches template"). Фикс: `\n            ` (12 пробелов) +
  regression-тест который рендерит реальный flutter_web template с 3
  секретами и assert'ит column 12 на каждом. 296 cargo tests pass.
- **B-000005 (critical) | Deploy-файлы не записывались в папку** —
  TS↔Rust API mismatch в `write_deploy_files`. TS в
  [DeployDetail.svelte:178](src/lib/components/DeployDetail.svelte#L178)
  маппил `RenderedFile[]` через `(f) => ({rel_path: f.path, content: f.content})`,
  переименовывая поле `path` в несуществующее `rel_path`. Rust struct
  `RenderedFile` ожидает `path` без rename — serde fail на missing
  field, команда возвращала Err, файлы не писались. Silent от прошлых
  релизов (catch показывал toast, но возможно user не видел). Фикс:
  передаём `toWrite` напрямую (уже корректный `RenderedFile[]`).
  Параллельно пофикшен тип `writeDeployFiles` в
  [tauri-commands.ts:452](src/lib/api/tauri-commands.ts#L452) — был
  анонимный `{rel_path; content}[]` + `{written; skipped}` shape; `skipped`
  тоже не существует (Rust возвращает `errors`). Теперь ссылается на
  shared `RenderedFile` и `WriteResult` из types.ts — гарантирует
  единый контракт между двумя сторонами.
- **B-000004 | DeployScreen secrets refresh-кнопка** — добавлена ↻
  "Обновить из GitHub" в [DeploySecretsTable.svelte](src/lib/components/DeploySecretsTable.svelte)
  header-row, mirrors SecretsPanel pattern. Reuse i18n key
  `secrets.refresh`.
- **Deploy_secrets orphan-cleanup при изменении meta.json**:
  `ensure_deploy_secrets_populated` теперь дополнительно DELETE rows
  whose `secret_name` is in NEITHER current GitHub repo secrets NOR
  `meta.json` required_secrets. Раньше row жил в `deploy_secrets`
  навсегда — после F-000037-перевода CONTAINER_NAME из secret в
  placeholder это оставляло orphan-строку в DeployDetail. Caller
  должен звать только с successfully-fetched `repo_secret_names`
  (empty-due-to-failure ложно бы прунил легитимные rows). +1 cargo
  test → 295 total.
- **B-000003 | Удалённые секреты репо не пропадают из UI до рестарта**:
  GitHub `list secrets` endpoint имеет eventual consistency — refetch
  сразу после DELETE может ещё несколько секунд возвращать удалённый
  секрет, и старый код (`loadSecrets()` после delete) ре-показывал его.
  Фикс в [SecretsPanel.svelte](src/lib/components/SecretsPanel.svelte):
  (1) optimistic update — `existingSecrets` фильтруется локально сразу
  по `succeeded` deletes; (2) filtered refetch — после refresh свежий
  ответ от GitHub дополнительно фильтруется `deletedSet` (denylist),
  чтобы stale-ответ не вернул удалённое; (3) кнопка ↻ "Обновить из
  GitHub" в header'е "Текущие секреты" для manual reload в любых
  staleness-сценариях.
- **B-000001 | SyncScreen показывал `owner/repo` вместо `repo`**: backend
  `Repository::display_name()` возвращал полный `github_name`, а frontend
  `getDisplayName()` уже отдавал last segment — асимметрия Rust↔TS пробивалась
  через `RequirementInfo.source_repo / target_repo` (18 точек) в SyncScreen.
  Переписан `display_name()` симметрично frontend'у (`gh.rsplit('/').next()`).
  Все 18 точек RequirementInfo + sync-error-логи автоматически очистились.
  `canonical_folder_name()` остаётся отдельно — он SoT для filesystem-папок
  с другим fallback (`local-<id>`). +4 unit-теста.
- **B-000002 | "Обновить документацию репозитория" теперь покрывает project.md
  и CLAUDE.md**: команда `init_docs_for_repo` раньше трогала только
  user-ownable skeletons (`todo.md`, `bug-reports.md`, `.gitignore`), а
  app-owned файлы (`project.md`, `CLAUDE.md` секция) обновлялись только из
  `sync_project`. Теперь идемпотентно регенерит обе категории — кнопка
  зеркалит pre-phase из Sync для одного репо. Для orphan-репо без `project_id`
  app-owned часть пропускается (rendering project-context не из чего).
- **B-000002 (часть 2) | Silent skip в `sync_project` теперь в errors[]**:
  если у репо нет `local_path` или папка отсутствует на диске, `sync_project`
  раньше тихо пропускал репо (project.md/CLAUDE.md/.gitignore не писались, без
  warning в toast). Теперь push'им explicit error — user видит причину
  пропуска. Применено к обоим циклам (own repos + microservice server-repos).
- Кнопка "📚 Инициализировать документацию" → "📚 Обновить документацию
  репозитория" / "📚 Update repo docs". "Init" подразумевал one-time, тогда
  как кнопка идемпотентна и теперь перезаписывает app-owned файлы каждый раз.

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
