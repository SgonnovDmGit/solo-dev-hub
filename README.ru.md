# Solo Dev Hub

> 🇬🇧 English version — [README.md](README.md)

**Личный пульт управления для соло-разработчика. Баги, требования, деплой — всё в markdown.**

Когда ведёшь 10 GitHub-репозиториев в одиночку, сам GitHub быстро упирается в потолок: баги размазаны по Issues отдельных репо, нет общего портфельного обзора, фичи через клиент + сервер + микросервис приходится держать в голове, deploy-pipeline переписываешь в каждом новом проекте. А как только начинаешь делегировать починку багов AI-агенту, "агент сказал что закрыл, я забыл проверить" незаметно становится основной точкой отказа.

Solo Dev Hub — десктопное приложение в одном окне, которое организует портфель, **запирает каждый баг в проверяемый AI-agent workflow**, ведёт задачи в Markdown который коммитится прямо в репо, синхронизирует требования между репо и автоматизирует деплой. Всё под одной крышей.

![Solo Dev Hub — Дашборд с портфельными KPI, top-3 горячих проектов и дневными графиками flow](docs/screenshots/hero-dashboard.png)

## Зачем это

Сделано для соло-разработчиков, indie-хакеров и фрилансеров, ведущих 5+ активных GitHub-репозиториев, которым не хочется:

- Платить за team-tier project-management SaaS (Linear / Jira) одному
- Использовать GitHub Issues по N репо и терять любой портфельный обзор
- Заново писать deploy YAML / Dockerfile в каждом новом проекте
- Терять трек, в каком репо сейчас больше всего активных багов

**AI-готовность из коробки.** Баги, задачи, требования, метаданные проектов и CLAUDE.md секции — всё лежит как Markdown прямо внутри репо. Любой AI-ассистент (Claude, ChatGPT, Copilot) видит весь портфель без отдельной API-интеграции — `git clone` и есть интеграция.

## Возможности

- **Закрытие багов AI-агентом со страховкой** — workflow status'ов (`created` → `in-progress` → `testing` → `confirmed` / `rejected`) разделяет роли: AI-агент берёт баг, применяет фикс, ставит `testing` с комментарием что именно сделано; **ты** верифицируешь и тыкаешь ✓ или ✗. Агент **не может** править `description`, `severity`, `category` и `fix_attempts` — ему доступны только `status` и `comment`. Счётчик попыток автоматически инкрементируется на каждом переходе в `testing`, так что "сколько попыток ушло на этот баг" — честная история, а не self-report. В сумме: ни один баг не падает в трещину между "агент сказал что починил" и "ты забыл проверить".
- **Портфельный дашборд** — KPI с фильтром по периоду (открыто / закрыто / fix rate / попыток за период), top-3 горячих проекта, дневные графики flow по багам и задачам, эффективность по категориям. Состояние портфеля одним взглядом.
- **Markdown-баги** — каждый баг живёт в `docs/bug-reports.md` своего репо. SQLite — source-of-truth, MD — двусторонне-синхронизируемый view для LLM. Severity, категория, append-only event-лог по каждому багу.
- **Cross-repo требования** — `REQ-NNN.md`-обмен между клиент ↔ сервер ↔ микросервис. Отправитель пишет ask, получатель — receipt, приложение разносит файлы между репо. Никаких GitHub Issues, никакой переписки в email.
- **Граф проекта** — визуализация проекта как 1-hop граф: сервер в центре, репо и подключённые микросервисы по кольцу. Клик по узлу — навигация. На Cytoscape.
- **Multi-environment деплой** — генерация Docker + GitHub Actions deploy-пайплайнов под разные окружения (prod / staging / test / любое имя) с native-интеграцией GitHub Environments и per-secret флагами role/scope.
- **Задачи (todo.md / done.md)** — каждый репо ведёт append-only лог завершённых задач с авто-проставленными версиями. Универсальный data-grid: фильтр, сорт, persist preferences по табу.
- **Хронология активности** — multi-source events (баги, задачи, sync'и, деплои, переименования репо) по всему портфелю. Фильтры по дате / типу события / репо / поиску.
- **Шаблоны** — per-language seed'ы для `.gitignore`, deploy YAML, CLAUDE.md секций. Настраиваешь один раз в приложении — синхронизирует во все проекты.
- **PAT в OS keyring** — твой GitHub-токен лежит в Windows Credential Manager (OS-уровень), никогда не в SQLite, не в `.env`, не plaintext-файлом.
- **Один .exe, ~11 MB** — Tauri v2 + WebView2. Без Electron-bloat'а. Без daemon'а. Без телеметрии. Единственный фоновый сетевой вызов — update-checker ходит в GitHub Releases один раз при старте; всё остальное только по твоему явному действию.

![Таб Bugs — per-repo список багов с severity, категорией, статусным workflow и счётчиком попыток на каждый баг](docs/screenshots/repo-bugs.png)

![Таб Tasks — todo.md как сортируемая и фильтруемая таблица с колонками priority, status и release-версии](docs/screenshots/repo-tasks.png)

![Граф архитектуры проекта — сервер в центре, клиенты и подключённые микросервисы по периметру (dashed-рёбра для cross-project микросервисов)](docs/screenshots/project-graph.png)

![Deploy-окружения — мастер-список per-repo deploy-таргетов (prod / staging / test, любое имя)](docs/screenshots/repo-deploy-master.png)

![Deploy drill-down — per-secret role-флаги (BUILD / DEPLOY / RUNTIME) и per-env include/override контроли](docs/screenshots/repo-deploy-detail.png)

![Requirements Sync — cross-repo REQ-обмен между клиент ↔ сервер ↔ микросервис, со статусами sent / responded и одно-кликовым подтверждением](docs/screenshots/requirements-sync.png)

## Технологии

- **Фреймворк** — Tauri v2 (Rust-бекенд + WebView2-фронтенд, single-binary distribution)
- **Фронтенд** — SvelteKit + Svelte 5 + TypeScript
- **Бекенд** — Rust: SQLite через `rusqlite`, file I/O для sync, Windows Credential Manager через `keyring`
- **GitHub API** — `@octokit/rest` (вызывается напрямую с JS-стороны, не проксируется через Rust)
- **Граф** — Cytoscape.js, concentric-раскладка, theme-aware
- **i18n** — Русский (по умолчанию) + Английский, ~750 type-safe ключей, без runtime-зависимостей
- **Autoupdate** — `tauri-plugin-updater` с Ed25519-подписью; production-сборки через GitHub Actions по push'у `v*`-тега

## Начало работы

### Установка

> Текущие билды — **только Windows x64**. Tauri архитектурно поддерживает macOS и Linux; не-Windows билды могут появиться в release-пайплайне по запросу.

1. Скачать `solo-dev-hub_<версия>_x64-setup.exe` со страницы [Releases](https://github.com/SgonnovDmGit/solo-dev-hub/releases)
2. Запустить installer.
3. **При первом запуске Windows SmartScreen может показать предупреждение** ("Издатель не определён"). Authenticode code-signing — в roadmap'е v2.0.0. Пока что: "Подробнее" → "Выполнить в любом случае".

### Первичная настройка

1. **Сгенерировать GitHub Personal Access Token** на [github.com/settings/tokens](https://github.com/settings/tokens) с такими scope'ами:
   - `repo` — полный доступ к репо (читать твои репо, управлять Actions secrets)
   - `workflow` — нужен для deploy-автоматизации
   - `read:user` — читать твой профиль
2. Открыть Solo Dev Hub → **Настройки** (значок шестерёнки) → вставить PAT → сохранить. Токен ляжет в Windows Credential Manager — никогда не остаётся на диске plaintext.
3. **Указать workspace root** — Настройки → Рабочее пространство. Это директория, под которой приложение ожидает увидеть твои репо в клонированном виде (например, `C:\Users\Ты\Development\`).
4. Нажать **🔄 Sync** в сайдбаре. Приложение подтянет список твоих репо с GitHub.
5. **Организовать**: перетащить репо в проекты в сайдбаре, или кликнуть по репо чтобы назначить роль (server / client / microservice / landing / tool / и т.д.).

![Настройки — GitHub PAT в OS keyring, внешний вид, рабочее пространство и шаблоны репозиториев](docs/screenshots/settings.png)

### Повседневный flow

- **Сайдбар** — твои проекты → репо. Клик по репо → табы Bugs / Задачи / Сделано / Changelog / Deploy / Секреты / Статистика.
- **Завести баг** через "+ Добавить баг" — сразу коммитится в `docs/bug-reports.md` (MD — view, SQLite — SoT).
- **Дашборд** (📊 в сайдбаре) — портфельные KPI с фильтрами по периоду и проектам.
- **Хронология** (📅) — хронологический feed событий по всему портфелю.
- **Deploy** — клик по deploy-capable репо → таб Deploy → настроить окружения + секреты → одной кнопкой сгенерировать Dockerfile + workflow.

## Разработка

### Prerequisites

- Node.js v18+
- Rust (через [rustup](https://rustup.rs))
- Microsoft C++ Build Tools (нужны Tauri'ю)
- WebView2 Runtime (preinstalled на Windows 11)

### Локально

```bash
npm install
npm run tauri dev          # локальный dev с hot reload
```

### Тесты

```bash
cd src-tauri && cargo test --lib   # ~370 Rust-тестов
npm test                            # vitest на фронтенде (~70 тестов)
npm run check                       # svelte-check
```

### Production-сборка

Production-релизы собираются GitHub Actions по push'у `v*`-тега — локально не собираем для distribution (unsigned, без `latest.json`):

```bash
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin master vX.Y.Z
```

Полный release-runbook (ротация ключей, CI troubleshooting, hotfix-flow) — [docs/RELEASING.md](docs/RELEASING.md).

### AI-правила

`CLAUDE.md` (gitignored) хранит in-project правила для AI. Фича приложения "Sync to ~/.claude/CLAUDE.md" пушит глобальную секцию в твой user-level Claude Code конфиг. Per-project CLAUDE.md лежит в корне каждого репо.

## Roadmap

- **v1.0.0** *(текущий — 2026-05-18)* — public launch, MIT open source, начало эры frozen contract.
- **v1.0.x** — post-launch polish: внутренние рефакторы (split `lib.rs` / `tauri-commands.ts`, декомпозиция 570-строчного `sync_project` handler'а), `docs/ARCHITECTURE.md` для контрибьюторов, SQLite ER-граф, in-app мультиязычный help-экран.
- **v2.0.0** — Windows Authenticode code signing (убирает SmartScreen warning), read-only API viewer + матрица совместимости клиент/сервер, REQ auto-accept через `## Status:` frontmatter.

Полный backlog и per-version задачи — [`docs/roadmap.md`](docs/roadmap.md).

## Поддержать разработку

Приложение бесплатное и без рекламы. Если оно экономит твоё время, можешь поддержать разработку:

- **Boosty** — [boosty.to/sgonnovdm/donate](https://boosty.to/sgonnovdm/donate) (₽ / карты / СБП)
- **TON** — `UQA-0I3SN2vw8F2ZzEoOTXT36-ToF0mu4Yp4_6pVmsR_dI0S`

Или открой экран About внутри приложения — там one-click ссылки и copy-to-clipboard для TON-адреса.

## Лицензия

[MIT](LICENSE) © 2026 Sgonnov D.A.

Сделано на [Tauri](https://tauri.app), [SvelteKit](https://kit.svelte.dev) и AI-ассистентах.
