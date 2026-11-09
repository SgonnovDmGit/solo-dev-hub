# Flow: Cross-repo announcements (proactive push)

**Введено в:** v0.26.0 (F-000040) — direct-write + delete-to-ack.
**Переработано в:** v1.23.0 (T-000245) — app-side транспорт через единый InOutBox + per-recipient `.ack/<key>` маркеры.
**Связанные файлы:** [src-tauri/src/sync/peer_channels.rs](../../src-tauri/src/sync/peer_channels.rs) (ack-механика), [src-tauri/src/sync/project_sync.rs](../../src-tauri/src/sync/project_sync.rs) (`sync_announce_channel`, dual-write), [src-tauri/src/sync/requirements.rs](../../src-tauri/src/sync/requirements.rs) (`migrate_announce_dir`).

## Модель

Announcement — **однонаправленный push-канал** для unsolicited информации, которая не помещается в REQ/receipt pattern. Используется когда sender'у нужно что-то сказать recipient'у, но recipient этого не просил.

С v1.23.0 announcement едет по **тому же парному транспорту InOutBox**, что REQ / NOTE / HANDOFF / PACT ([peer-channels.md](peer-channels.md)):

| Роль | Путь |
|---|---|
| Sender пишет | `<sender-repo>/docs/InOutBox/outbox/<recipient-canonical>/ANNOUNCE-NNN_<slug>.md` |
| Recipient читает | `<recipient-repo>/docs/InOutBox/inbox/<sender-canonical>/ANNOUNCE-NNN_<slug>.md` |

Приложение зеркалит outbox→inbox, переносит маркеры обратно и убирает закрытые пары. Парность — repo↔repo: анонс для N получателей = N файлов в N outbox-папках, broadcast-лейна нет.

Существуют **два вертикальных направления** (как в REQ-модели):

| Направление | Sender | Recipient |
|---|---|---|
| Server → client | server repo | каждый client repo |
| Microservice → parent server | ms repo | каждый parent server repo |

В обратные стороны (client→server, server→MS) announcement'ов нет — там флоу через REQ/receipt. Горизонтальные peer-пары (equal repos) технически несут тот же механизм через `peer_links`.

## Ack: `.ack/<key>` маркер вместо удаления файла

До v1.23.0 recipient подтверждал анонс **удалением файла** — это ломалось на shared-репо: одна общая `dev`-ветка видна всем участникам, и один удалённый файл не может сказать, кто именно прочитал.

Теперь recipient'ский агент кладёт **пустой файл-маркер** рядом с телом:

```
inbox/<sender>/ANNOUNCE-015_rename.md
inbox/<sender>/ANNOUNCE-015_rename.md.ack/<key>     # пустой файл
```

- `key` — **login участника** в shared-репо (`.sdh/project.json` roster), иначе **canonical name** recipient-репо (один логический читатель).
- Маркер **пустой** — любой непустой файл / невалидное имя / поддиректория / symlink внутри `.ack/` считается issue (`peer_channel_failed`), не ack'ом.
- Приложение маркеры не пишет — это честь читателя. Приложение их **транспортирует** обратно в outbox sender'а (union per key — чужой ранний ack никогда не исчезает) и валидирует.

### Ревизии тела

Автор меняет body → маркеры к старым байтам недействительны:

- На локальной паре: body в outbox и inbox разошлись → `.ack/` группа стирается на **обеих** копиях **до** доставки новой редакции.
- На shared-репо: маркер засчитывается, только если коммит, его внёсший, — потомок коммита, установившего текущие байты body (`git merge-base --is-ancestor`). Поздний merge со старым маркером = **conflict** (`version_conflict` issue), не ack. Незакоммиченный маркер в worktree — единственный доказуемо свежий (написан под тело этого worktree).
- Новый `NNN` не наследует маркеры — каждый номер начинает с чистого `.ack/`.

### Cleanup (owner-side)

Когда **все ожидаемые читатели** recipient-стороны подтвердили:

- non-shared: единственный ключ — canonical name репо;
- shared: roster `main.login ∪ participants[].login` из `.sdh/project.json` — **перечитывается на каждом проходе** (новый участник → его ключа нет → анонс остаётся открытым для него);

— приложение удаляет body и всю `.ack/` группу на обеих копиях (outbox + inbox). Без известного roster'а (нет БД / нет структуры) cleanup **не** выполняется — shared-репо без login'а читает, но не подтверждает и не подметает.

`.ack/` директория без тела — сирота: sweeper убирает её, body **не воскрешает**.

## Переходный протокол (v1.23.x)

Legacy-папки живут как проекция и источник миграции:

| Направление | Legacy-путь (на стороне recipient'а) |
|---|---|
| Server → client | `docs/server-announcements/<server-canonical>/` |
| MS → parent server | `docs/microservice-announcements/<ms-project-name>/` |

- **Миграция** (`migrate_announce_dir`): оставшиеся legacy-файлы переезжают в `inbox/<sender>/` + `outbox/<recipient>/` попарно, byte-identical; dedup при равных байтах, `migration_diverged` при разных (оба варианта живут). Non-shared recipient → legacy-файл убирается; shared → остаётся как проекция.
- **Dual-write** (`ANNOUNCE_LEGACY_DUAL_WRITE`, shared recipient'ы): каждое новое body в `inbox/<sender>/` проецируется byte-identical файлом на legacy-путь — **один раз**. Отсутствующий файл, бывший в git-истории, = старый delete-ack → **не воскрешается**. Существующий — собственность старого агента, не переписывается.
- **Retire**: пара закрыта в новой раскладке (body было в истории и исчезло) → legacy-проекция снимается. Файл, чьё new-body никогда не существовало — pre-migration остаток, живёт.
- Поздний старый маркер / удаление проекции не воскрешают закрытый анонс (`closed_announce_names` по git-истории).
- Release-гейт: когда все установки на ≥ v1.23.0, `ANNOUNCE_LEGACY_DUAL_WRITE` выключается — проекции перестают писаться, legacy-папки остаются архивом.

## Когда использовать announcement vs REQ receipt

| Ситуация | Канал |
|---|---|
| Recipient попросил X; sender сделал X и хочет дать integration notes | REQ receipt (`## Comment:` в receipt'е) |
| Sender сделал что-то по собственной инициативе, что влияет на recipient'а | Announcement |
| Sender делал работу для recipient'а A; side-effect задел recipient'а B | Announcement → B |
| Sender внутренне переработал (refactor, MS-review, deprecation) и recipient'у нужно подстроить интеграцию | Announcement |

**Threshold: actionable impact required.** Announcement оправдана **только** если recipient должен принять действие — изменить код / конфиг / поведение. Чисто факт sender-side изменения недостаточен. Pure surface additions (новый admin-only endpoint, internal refactor без wire-format impact) идут через `docs/api.md` sync — recipient узнает на свой timeline через synced contract.

**Positive criterion** (helper если sender не уверен): если recipient должен изменить код / конфиг / поведение чтобы продолжать корректно работать — пиши announcement. Если recipient может работать дальше unchanged и подобрать изменение позже через `api.md` — не пиши. Чрезмерные announcements обесценивают канал.

## Lifecycle (server → client пример)

Сервер переименовал rate-limit header `X-RateLimit-Hour-Remaining` → `X-RateLimit-Remaining-Hour`. Клиент не просил, но если не подстроится — будет читать undefined.

1. **Server-LLM канонизирует имена.** `docs/project.md` `## Repositories`: свою row (sender canonical) и row каждого client (`role` ∈ {`client`, `admin_client`, `test_client`}).

2. **Picks `NNN`** — `max(existing ANNOUNCE-NNN*.md) + 1` в `docs/InOutBox/outbox/<client-canonical>/` (per sender×recipient pair, monotonic; ранее закрытые номера не переиспользуются — это сохраняет читаемость истории).

3. **Пишет файл** в СВОЙ outbox: `<server-repo>/docs/InOutBox/outbox/<client-canonical>/ANNOUNCE-NNN_<slug>.md` — один файл на каждого client. 4-section структура:

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

   Отсюда доставка — дело приложения: на ближайшем sync body зеркалит в `inbox/<server>/` каждого client'а (а для shared client'а едет по git на общую `dev`-ветку до всех участников).

4. **Recipient в новой сессии scan'ит** `docs/InOutBox/inbox/<sender>/` (а на переходный период — и legacy-папку, где могут жить остатки). Для каждого `ANNOUNCE-*.md`:
   - Читает body, интегрирует изменения.
   - **Кладёт `.ack/<key>`-маркер** — пустой файл `ANNOUNCE-NNN_<slug>.md.ack/<key>`: свой login в shared-репо, canonical name репо иначе. Удалять body **не нужно** — это делает приложение после всех подтверждений.
   - Маркер коммитится вместе с вызванными code-changes (shared-репо — через обычный plumb на `dev`).

5. **Приложение закрывает пару**: все ожидаемые ключи собраны → body + `.ack/` удаляются с обеих сторон.

## Microservice → parent server flow

Зеркально: sender — ms repo, recipient — каждый parent server. MS пишет в свой `outbox/<parent-canonical>/`; parent читает `inbox/<ms-canonical>/`. Legacy-проекция на стороне parent'а: `docs/microservice-announcements/<ms-project-name>/` — имя папки от **ms project**, rename реплеится по `project_renames`.

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
| Generate `docs/project.md` (canonical names) | ✓ | — |
| Транспорт body outbox→inbox (включая shared `dev`-ветку) | ✓ | — |
| Перенос `.ack/` маркеров обратно в outbox (union per key) | ✓ | — |
| Сброс маркеров при ревизии body; ancestry-гард на shared | ✓ | — |
| Legacy-миграция + dual-write проекции + retire | ✓ (shared recipient'ы, пока gate on) | — |
| Cleanup body+`.ack/` после всех подтверждений | ✓ | — |
| Pick next NNN, написать body в свой outbox | — | ✓ |
| Прочитать, применить, положить `.ack/<key>` | — | ✓ |
| Track outbox / inbox count в UI | — | — (нет UI counter, badge, screen) |
| Audit trail | — | git history обоих репо |

## Carve-out из no-cross-repo-writes rule — снят

До v1.23.0 announcement был explicit exception: sender-LLM писал напрямую в filesystem recipient'а. С v1.23.0 агент пишет **только в собственный репо** (`outbox/<recipient>/`) — carve-out больше не нужен, границу репо пересекает приложение.

## Cross-reference

- Парный транспорт, teardown-семантика, registry: [peer-channels.md](peer-channels.md)
- REQ/receipt и триангулярный flow: [requirements-sync.md](requirements-sync.md), [microservice-server-sync.md](microservice-server-sync.md)
- Shared-режим, roster, plumb: [contributor-mode.md](contributor-mode.md)
- Нормативный текст для агентов — скиллы `sdh-cross-repo-announce-send` / `sdh-cross-repo-announce-read`
