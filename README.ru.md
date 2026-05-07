# Solo Dev Hub

> 🇬🇧 English version — [README.md](README.md)

**Личный кокпит портфеля для соло-разработчика. Баги, требования, деплой — всё в markdown.**

Когда ведёшь 10 GitHub-репозиториев в одиночку, начинают мешать вещи, которые сам GitHub не решает: баги размазаны по Issues отдельных репо, нет общего портфельного обзора, фичи через несколько репо (клиент + сервер + микросервис) приходится держать в голове, deploy-pipeline пишешь руками в каждом новом проекте — а как только начинаешь делегировать починку багов AI-агенту, "агент сказал что закрыл, я забыл проверить" незаметно становится основной точкой отказа. Solo Dev Hub — десктопное приложение в одном окне, которое организует портфель, **запирает каждый баг в проверяемый AI-agent workflow**, ведёт задачи в Markdown который коммитится прямо в репо, синхронизирует требования между репо и автоматизирует деплой. Всё под одной крышей.

<!-- TODO screenshot: hero — главное окно с раскрытым сайдбаром (дерево проектов), активный таб Dashboard, период Quarter, видны KPI-тайлы + top-3 горячих + daily flow chart. Ширина ~1200px, тёмная тема. -->

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

<!-- TODO screenshot: RepoDetail с открытым табом Bugs, сайдбар свёрнут до иконок, видны 4-5 багов разной severity и status. -->

<!-- TODO screenshot: ProjectGraph для проекта с сервер-репо в центре + 3-4 подключёнными микросервисами по кольцу, dashed-линии для cross-project ms. -->

<!-- TODO screenshot: DeployScreen master-view (таблица deploy-окружений) + drill-down DeployDetail с per-secret role/scope флагами. -->

## Технологии

- **Фреймворк** — Tauri v2 (Rust-бекенд + WebView2-фронтенд, single-binary distribution)
- **Фронтенд** — SvelteKit + Svelte 5 + TypeScript
- **Бекенд** — Rust: SQLite через `rusqlite`, file I/O для sync, Windows Credential Manager через `keyring`
- **GitHub API** — `@octokit/rest` (вызывается напрямую с JS-стороны, не проксируется через Rust)
- **Граф** — Cytoscape.js, concentric-раскладка, theme-aware
- **i18n** — Русский (по умолчанию) + Английский, ~390 type-safe ключей, без runtime-зависимостей
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

<!-- TODO screenshot: Settings с четырьмя картами (PAT, Внешний вид, Рабочее пространство, Шаблоны), PAT-поле скрыто но виден 👁 toggle. -->

### Повседневный flow

- **Сайдбар** — твои проекты → репо. Клик по репо → табы Bugs / Задачи / Сделано / Changelog / Статистика / Секреты.
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
cd src-tauri && cargo test --lib   # ~290 Rust-тестов
npm test                            # vitest на фронтенде (~40 тестов)
npm run check                       # svelte-check
```

### Production-сборка

Production-релизы собираются GitHub Actions по push'у `v*`-тега — локально не собираем для distribution (unsigned, без `latest.json`):

```bash
git tag -a v0.25.0 -m "v0.25.0"
git push origin master v0.25.0
```

Полный release-runbook (ротация ключей, CI troubleshooting, hotfix-flow) — [docs/RELEASING.md](docs/RELEASING.md).

### AI-правила

`CLAUDE.md` (gitignored) хранит in-project правила для AI. Фича приложения "Sync to ~/.claude/CLAUDE.md" пушит глобальную секцию в твой user-level Claude Code конфиг. Per-project CLAUDE.md лежит в корне каждого репо.

## Roadmap

- **v0.25.0** *(текущий цикл)* — pre-rebrand cleanup: смена display-name, branches workflow, README polish.
- **v1.0.0** — public launch. Rebrand технических идентификаторов, репо переходит из private в public, README и Releases становятся видны миру.
- **v2.0.0** — Windows Authenticode code signing (убирает SmartScreen warning) + read-only API viewer экран.

## Поддержать разработку

Приложение бесплатное и без рекламы. Если оно экономит твоё время, можешь поддержать разработку:

- **Boosty** — [boosty.to/sgonnovdm/donate](https://boosty.to/sgonnovdm/donate) (₽ / карты / СБП)
- **TON** — `UQA-0I3SN2vw8F2ZzEoOTXT36-ToF0mu4Yp4_6pVmsR_dI0S`

Или открой экран About внутри приложения — там one-click ссылки и copy-to-clipboard для TON-адреса.

## Лицензия

[MIT](LICENSE) © 2026 Sgonnov D.A.

Сделано на [Tauri](https://tauri.app), [SvelteKit](https://kit.svelte.dev) и AI-ассистентах.
