# Flow: Cross-repo announcements (proactive push)

**Введено в:** v0.26.0 (F-000040)
**Связанные файлы:** [src-tauri/src/sync.rs](../../src-tauri/src/sync.rs) (`generate_project_md` — `## Parent projects` секция расширена local path для MS use case)

## Модель

Announcement — **однонаправленный push-канал** для unsolicited информации, которая не помещается в REQ/receipt pattern. Используется когда sender'у нужно что-то сказать recipient'у, но recipient этого не просил.

Канал отличается от REQ/receipt тем, что **Solo Dev Hub в нём не участвует**: LLM пишет файл напрямую в filesystem recipient'а, recipient читает и удаляет файл — deletion = implicit ack. Никаких `.response.md` companion'ов, никакого app-side sync'а, никакой confirm ✓ кнопки в UI.

Существуют **два направления** announcement'а, симметрично:

| Направление | Recipient folder |
|---|---|
| Server → client | `<client-repo-path>/docs/server-announcements/<server-canonical>/ANNOUNCE-NNN_<slug>.md` |
| Microservice → parent server | `<parent-server-repo-path>/docs/microservice-announcements/<ms-project-name>/ANNOUNCE-NNN_<slug>.md` |

В обратные стороны (client→server, server→MS) announcement'ов нет — там флоу через REQ/receipt.

## Когда использовать announcement vs REQ receipt

| Ситуация | Канал |
|---|---|
| Recipient попросил X; sender сделал X и хочет дать integration notes | REQ receipt (`## Comment:` в receipt'е) |
| Sender сделал что-то по собственной инициативе, что влияет на recipient'а | Announcement |
| Sender делал работу для recipient'а A; side-effect задел recipient'а B | Announcement → B |
| Sender внутренне переработал (refactor, MS-review, deprecation) и recipient'у нужно подстроить интеграцию | Announcement |

**Threshold: actionable impact required.** Announcement оправдана **только** если recipient должен принять действие — изменить код / конфиг / поведение. Чисто факт sender-side изменения недостаточен. Pure surface additions (новый admin-only endpoint, internal refactor без wire-format impact) идут через `docs/api.md` sync — recipient узнает на свой timeline через synced contract.

**Positive criterion** (helper если sender не уверен): если recipient должен изменить код / конфиг / поведение чтобы продолжать корректно работать — пиши announcement. Если recipient может работать дальше unchanged и подобрать изменение позже через `api.md` — не пиши. Чрезмерные announcements обесценивают канал.

Полная таблица + threshold подраздел — в global CLAUDE.md section `# Cross-repo announcements`.

## Lifecycle (server → client пример)

Сервер переименовал rate-limit header `X-RateLimit-Hour-Remaining` → `X-RateLimit-Remaining-Hour`. Клиент не просил, но если не подстроится — будет читать undefined.

1. **Server-LLM открывает свой `docs/project.md`** → `## Repositories` table, находит row(s) с `role` ∈ {`client`, `admin_client`, `test_client`}, берёт значение колонки `Path`. Это локальный путь client-репо в файловой системе разработчика.

2. **Server-LLM канонизирует своё имя.** В том же `docs/project.md` `## Repositories` находит row, у которого `Path` совпадает с текущим working directory (или server-LLM знает имя своего проекта по контексту). Берёт last-segment github_name — e.g. `web-app-backend`, не `SgonnovDmGit/web-app-backend`. Это subfolder name на recipient-стороне.

3. **Resolves recipient announcement folder:** `<client-path>/docs/server-announcements/<server-canonical>/`. Если её нет — создаёт. Если path missing (`no local path configured` в project.md) — **announcement не deliverable**, server-LLM поднимает gap у юзера (own session log / `T-NNNNNN` в todo.md), файл "в никуда" не пишет.

4. **Picks `NNN`** — `max(existing ANNOUNCE-NNN_*.md) + 1` если folder не пустой, иначе `001`. `NNN` — **monotonic** counter per (sender × recipient) pair, не slot allocator: previously deleted (acknowledged) entries не освобождают свои слоты для переиспользования. Это сохраняет читаемость git history recipient-репо — номера всегда растут.

5. **Пишет файл** напрямую в recipient'ский filesystem по пути `<client-path>/docs/server-announcements/<server-canonical>/ANNOUNCE-NNN_<slug>.md`. 4-section структура:

   ```markdown
   # ANNOUNCE-007: Rate-limit header renamed

   ## Why this matters to you

   Клиенту нужно обновить парсинг rate-limit header'а, иначе rate-limit
   values начнут читаться как undefined.

   ## What changed

   Renamed `X-RateLimit-Hour-Remaining` → `X-RateLimit-Remaining-Hour`
   (приведено к стандарту GitHub / Stripe / RFC draft).

   ## What you need to do

   1. Найти места парсинга старого header'а.
   2. Обновить ключ в коде.
   3. Прогнать тесты.

   ## References

   - commit abc1234 в web-app-backend
   - `internal/middleware/ratelimit.go:42`
   ```

   **Sender НЕ хранит outbox-копию.** Git history recipient-репо — audit trail. Если sender'у нужно вспомнить что было отправлено — `git log` в recipient-репо (или session-log).

6. **Recipient в новой сессии scan'ит** свой `docs/server-announcements/` (если client) или `docs/microservice-announcements/` (если parent server). Каждый subfolder — это sender (по canonical name). Для каждого найденного `ANNOUNCE-*.md`:
   - Читает body (4 секции).
   - Интегрирует / применяет изменения в собственный код.
   - **Удаляет файл.** Deletion = implicit ack.
   - Commit'ит deletion (вместе с code-changes которые она вызвала) в свой git. Это audit trail.

## Microservice → parent server flow

Зеркальный flow с одной asymmetry: MS получает recipient path из `## Parent projects` секции своего `docs/project.md`. С v0.26.0 каждая строка читается как:

```
- **<parent-name>** — server repo: <name> (path: <local-path>)
```

(До v0.26.0 path в строку не выводился — F-000040 добавил расширение в `generate_project_md`.)

Если в строке `(no local path configured)` или `⚠ server repo not resolvable` — announcement не deliverable, MS-LLM не пишет файл.

Folder на стороне parent server'а: `<parent-path>/docs/microservice-announcements/<ms-project-name>/`. Subfolder name — **canonical name of MS project** (тот же что используется в `microservice-requirements/<ms-project>/` на server-стороне для REQ flow).

Всё остальное (NNN counter, 4-section structure, ack-via-delete) идентично server→client направлению.

## Сценарии — когда announcement уместен

### Sender-initiated change

Server поменял internals, что напрямую отражается на client (renamed field, changed semantics, deprecated endpoint). Client не просил — но если не подстроится, сломается. → Announcement server→client.

### Side-effect impact на third party

Client A попросил у server'а feature X через REQ. Server сделал, попутно изменив общий компонент, который читает и client B. Client A узнаёт о результате через REQ receipt. Client B — через announcement: он не просил, но затронут.

### Post-internal-review rework

MS-LLM получил code-review feedback и переделал внутреннее устройство (например ужесточил cache TTL). Public API не поменялся, но parent server'у желательно подстроить retry timeout. → Announcement MS→parent.

### Cross-repo deprecation timeline

Server планирует через 2 минорки убрать legacy endpoint, который всё ещё используется client'ом. Standard `docs/api.md` отмечает endpoint как `deprecated`, но client может это пропустить. → Announcement с явной датой и шагами миграции.

## Сценарии — когда announcement НЕ уместен

| Ситуация | Почему | Правильный канал |
|---|---|---|
| Новый admin-only endpoint, client его не использует | Pure surface addition; recipient узнает через synced api.md когда захочет | `docs/api.md` sync |
| Sender починил баг, фикс прозрачен для recipient'а (ничего менять не нужно) | Нет actionable impact | Никакой (только `Changelog.md` на sender-стороне) |
| Client попросил X, server сделал X, хочет дать integration notes | Это reactive ("ты попросил, вот ответ") | REQ receipt `## Comment:` |
| Server хочет попросить client сделать что-то | Это REQ (request от server клиенту), а не announcement (push информации) | Reverse REQ канал на момент v0.26.0 не существует — server-LLM поднимает у юзера / открывает T-NNN в todo.md |
| Sender изменил что-то, но не знает влияет ли на конкретного recipient'а | Не пиши на всякий случай — only когда actionable impact доказан | Удерживать в Changelog'е; recipient увидит через api.md если затрагивает контракт |

## Где Solo Dev Hub помогает / не помогает

| Шаг | Solo Dev Hub | LLM |
|---|---|---|
| Generate `docs/project.md` (recipient paths + canonical names) | ✓ | — |
| Сшить parent server path в MS-side project.md (v0.26.0) | ✓ (`generate_project_md`) | — |
| Discover recipient path | — | ✓ (читает project.md) |
| Subfolder canonical naming | — | ✓ (берёт last-segment github_name из project.md row) |
| Pick next NNN | — | ✓ (grep + max+1) |
| Write `ANNOUNCE-*.md` в recipient-filesystem | — | ✓ (direct fs write, carve-out из no-cross-repo-writes rule) |
| Detect файлы recipient'ом | — | ✓ (scan на session start) |
| Delete after ack | — | ✓ |
| Commit deletion | — | ✓ (вместе с triggered code changes) |
| Track outbox / inbox count в UI | — | — (нет: нет UI counter, badge, screen для announcements) |
| Audit trail | — | git history recipient-репо |

В отличие от REQ/receipt flow, где Solo Dev Hub несёт основную работу по перемещению файлов между репо, в announcement flow **app не делает ничего за рантайм**. Единственная роль app — генерация `docs/project.md`, в которой content для recipient paths и canonical names уже доступен.

## Carve-out из no-cross-repo-writes rule

Global CLAUDE.md секция `# Cross-repo requirements` запрещает LLM копировать / перемещать / удалять файлы через границы репо вручную — это владение Solo Dev Hub'а для REQ pairs. Announcements — **explicit exception**:

- Sender-LLM **пишет** `ANNOUNCE-*.md` в filesystem recipient'а напрямую.
- Recipient-LLM **удаляет** `ANNOUNCE-*.md` из собственного репо после интеграции.
- Solo Dev Hub **не отслеживает, не пропагирует, не surface'ит** announcement-файлы — нет UI counter, нет badge, нет confirm-✓.

Carve-out обоснован асимметрией flow: announcement не имеет recipient-response (а значит нет необходимости в bidirectional sync), и потеря/дубликат сообщения не разрушает invariants (recipient просто узнает позже / получит ту же info дважды и удалит).

## Cross-reference

Нормативный текст — в global CLAUDE.md section `# Cross-repo announcements (proactive push)`:
- `## When to use an announcement vs a REQ receipt` — таблица + `### Threshold: actionable impact required` подраздел
- `## Directions` — server→client / MS→parent server
- `## How the sender writes an announcement` — 3 numbered шага (project.md path lookup → NNN → write)
- `## How the recipient reads an announcement` — scan + ack-via-delete
- `## Carve-out from the no-cross-repo-writes rule` — explicit exception

Для триангулярного flow (client → server → microservice) через REQ/receipt — см. [microservice-server-sync.md § Triangular flow](microservice-server-sync.md).
