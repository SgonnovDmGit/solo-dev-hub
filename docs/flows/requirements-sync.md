# Flow: Requirements Sync (REQ ↔ Response ↔ Impl)

**Введено в:** v0.2.0 (F-012 в v0.8.0; nested-per-parent + rename-replay в v0.14.0/F-033; `.impl.md` авто-close в v0.26.x/F-000039; авто-синк/авто-коммит в v1.7.0; **единая раскладка InOutBox + миграция legacy-папок — v1.23.0/T-000190**)
**Смежный flow**: api.md/handlers.md one-way copy — [api-handlers-sync.md](api-handlers-sync.md); унифицированный канал — [peer-channels.md](peer-channels.md).
**Связанные файлы (после распила монолитов):**
- `src-tauri/src/commands/sync.rs` — Tauri-команды `sync_project` (async, T-000154), `list_project_requirements`, `confirm_requirement`
- `src-tauri/src/sync/project_sync.rs` — `run_project_sync`: оркестрация одного прохода (все направления + авто-close + авто-коммит)
- `src-tauri/src/sync/requirements.rs` — `confirm_pair`, `auto_close_impl_pairs` (legacy-фолбэк), `migrate_req_pair`, rename-replay хелперы
- `src-tauri/src/sync/peer_channels.rs` — `sync_peer_pair`: kind-aware канал, на котором теперь ездит и REQ-тройка
- `src-tauri/src/sync/fs.rs` — `scan_requirements` / `scan_responses` / `scan_impl_files` / `copy_file_if_changed`
- [src/lib/components/SyncScreen.svelte](../../src/lib/components/SyncScreen.svelte) — UI; `src/lib/stores/autosync.ts` — фоновый таймер

## Суть

Требования между репо передаются через **локальные MD-файлы**, а не через git push. Приложение копирует `REQ-*.md` файлы между папками репо по паттерну «сообщение-квитанция». Merge-конфликтов нет по дизайну: у каждого файла один owner, вторая сторона read-only через протокол.

## Раскладка (T-000190, v1.23.0)

REQ-тройка живёт на **единой раскладке InOutBox** — той же, что канал равных (`peer-channels.md`): имя папки называет направление, а не роль контрагента.

| Направление | Файлы отправителя | Файлы получателя |
|---|---|---|
| sender → recipient | `docs/InOutBox/outbox/<peer>/REQ-*.md` (+ `.impl.md`) | `docs/InOutBox/inbox/<sender>/REQ-*.md` (реплика) + `.response.md` |

`<peer>` — `Repository::canonical_folder_name()` контрагента. Вертикальные пары и пары равных делят одну механику: REQ — просто ещё один род файла в канале, рядом с NOTE/HANDOFF/PACT/ANNOUNCE. Зеркало с прунингом kind-aware: `*.response.md` и `*.ack/` в инбоксе **не зеркалируются и не прунятся** — их пишет получатель; ответ едет обратно `inbox → outbox`.

**Legacy-раскладка заморожена, не стёрта.** `docs/backend-requirements/`, `docs/client-requirements/<c>/`, `docs/microservice-requirements/<m>/`, `docs/server-requirements/<s>/` — read-only архивы: миграция переносит из них только `REQ-*`-тройки; `registry.md`, прочие файлы и подпапки остаются на месте (директория с остатком не удаляется). Запись туда больше никто не делает.

## Миграция legacy → InOutBox (`migrate_req_pair`)

На каждом проходе пары приложение планирует перенос **обеих** сторон и применяет атомарно для пары:

- Файл без цели — move (rename, cross-volume → copy+verify+remove).
- Цель есть и байты совпадают — dedup: источник удаляется, копия остаётся.
- Цель есть и байты разные — **конфликт**: ничего не перезаписывается, обе версии сохраняются (legacy-файл остаётся), поднимается `migration_diverged`; номер файла всё равно учитывается в global max.
- Сбой fs откатывает уже сделанные шаги обеих сторон — канал никогда не работает «половина по-старому, половина по-новому»; на этом проходе продолжает legacy-копия (`sync_req_pair_legacy`).
- Опустевший legacy-каталог удаляется; с остатком — остаётся архивом.
- Идемпотентно: повторный проход видит пустой источник и переходит сразу к каналу.

Переносятся **только** имена `REQ-*` с суффиксами `.md` / `.response.md` / `.impl.md`.

## Паттерн «сообщение-квитанция»

1. **Отправитель** создаёт `REQ-XXX_topic.md` в своём `outbox/<получатель>/`.
2. **Sync** зеркалит файл в `inbox/<отправитель>/` получателя. Правка REQ у отправителя перезаписывает копию (**sender = source of truth** для `.md`); удаление тела у отправителя **прунит реплику у получателя**.
3. **Получатель** реализует и создаёт `.response.md` рядом с копией в своём инбоксе (**не редактирует тело**). Зеркало его не трогает — `.response.md` recipient-authored.
4. **Sync** копирует `.response.md` обратно в `outbox` отправителя (**inbox = source of truth** для `.response.md` — единственное место, где реплика является источником).
5. **Отправитель** смотрит ответ и **закрывает пару** — ручным ✓ или `.impl.md` в своём outbox. Не устраивает ответ — новый `REQ-N+1_<slug>.md` (reject-flow убран в 0.13.27).

Тройка файлов одной пары: `REQ-XXX.md` (base, sender, outbox) + `REQ-XXX.response.md` (receipt, recipient, inbox→outbox) + опционально `REQ-XXX.impl.md` (close-signal, sender, outbox).

## Три направления

### Client → Server

```
Отправитель: client
Получатель: server
├─ client/docs/InOutBox/outbox/<server>/REQ-001_feature.md                  (пишет клиент)
├─ server/docs/InOutBox/inbox/<client>/REQ-001_feature.md                   ← mirror by sync
└─ server/docs/InOutBox/inbox/<client>/REQ-001_feature.response.md          (пишет сервер)
   → client/docs/InOutBox/outbox/<server>/REQ-001_feature.response.md       ← copied back
```

Legacy-пути той же пары (заморожены): `client/docs/backend-requirements/` ↔ `server/docs/client-requirements/<client>/`.

### Server → Microservice (F-012)

Микросервис — отдельный проект; требования идут в его **server-репо**. `<peer>` на обеих сторонах — canonical контрагента, **nested per parent** сохранён (ms с несколькими родителями имеет `inbox/<parent>` на каждого).

```
Отправитель: parent-server
Получатель: ms-server-repo (role=server внутри microservice-project)
├─ parent/docs/InOutBox/outbox/<ms>/REQ-002_api.md                    (пишет parent)
├─ ms-server/docs/InOutBox/inbox/<parent>/REQ-002_api.md              ← mirror by sync
└─ ms-server/docs/InOutBox/inbox/<parent>/REQ-002_api.response.md     (пишет microservice)
   → parent/docs/InOutBox/outbox/<ms>/REQ-002_api.response.md         ← copied back
```

Legacy-пути: `parent/docs/microservice-requirements/<ms>/` ↔ `ms-server/docs/server-requirements/<parent>/`.

### Server → Client (api/handlers)

Не REQ-цикл, а односторонняя передача контракта: `docs/api.md` + `docs/handlers.md` + `docs/my_api/*.md` сервера копируются в `docs/server-api/` клиента. Контракт — опубликованный артефакт, а не переписка: **единственное** осознанное исключение из InOutBox-раскладки. Подробно — [api-handlers-sync.md](api-handlers-sync.md).

### Rename handling (F-033 + T-000190)

При переименовании репы запись пишется в `repo_renames`. На следующем sync `replay_rename_in_dir` переименовывает counterparty-side папку со старого canonical на новый — **сначала в legacy-родителях** (чтобы миграция подхватила уже переименованные файлы), **затем в InOutBox-родителях** `outbox/` `inbox/` `pact/` (`replay_inoutbox_renames` / `replay_peer_renames`). Registry-файлы агентские — приложение их не переименовывает. History — Settings → Rename Log.

## Закрытие пары

Закрытие = **удаление всей тройки на обеих сторонах**. Два пути, оба выполняет **приложение**:

1. **Ручной ✓** (`confirm_requirement` → `confirm_pair`). Жмётся на `responded`-паре. Сносит `.md` + `.response.md` + `.impl.md` **во всех кандидатных каталогах** — InOutBox-полосе и legacy-остатке на обеих сторонах (полумигрированная пара закрывается чисто). Пути выводятся из `Repository` source/target — работает симметрично с SyncScreen любого проекта; неподдерживаемая роль источника — ошибка, не молчаливый no-op.
2. **`.impl.md` (LLM-authorable, F-000039).** Отправитель кладёт `REQ-XXX_<slug>.impl.md` рядом с REQ в **своём outbox**. На следующем sync kind-aware teardown сносит `.md` + `.response.md` + `.impl.md` на обеих сторонах. Маркер **родо-заперт**: `.impl.md` в outbox/inbox закрывает только `REQ-*`-тройку (NOTE/HANDOFF/ANNOUNCE не трогает), в `pact/` — только `PACT-*`; `.done.md` — только NOTE/HANDOFF. Залетевший «не свой» маркер просто зеркалится как обычный файл.

**`.impl.md` — только отправитель, только в своём outbox.** Существование файла — сигнал; содержимое игнорируется. Получатель `.impl.md` не пишет — его инбокс зеркалится с чужого outbox, а `.impl.md` в инбоксе не является сигналом закрытия.

## Failure mode — воскрешение base-REQ

Симптом прежний: «закрытый» REQ возвращается получателю снова и снова.

Причина прежняя: **base `REQ-XXX.md` остался в outbox отправителя.** Sender = source of truth: каждый sync заново зеркалит его в инбокс. `.impl.md`, удалённый до прохода авто-close, сигналом уже не станет.

Что поменялось с зеркалом (T-000190):

- Удаление **получателем** копии из инбокса — безразлично: зеркало восстановит её, пока тело живо в outbox. «Убрать у себя» REQ нельзя — закрывает отправитель.
- Удаление **отправителем** тела из outbox теперь **прунит** реплику в инбоксе получателя (раньше оставался сирота). Сиротой остаётся только `.response.md` получателя — он recipient-authored и прунингу не подлежит.
- `.response.md` в outbox отправителя — реплика ответа; если получатель удалит ответ у себя, отправительская копия не исчезает сама (ответы назад не прунятся — как в legacy).

Профилактика (закреплено в скиллах `sdh-cross-repo-req-send` / `-answer`):
- Положил `.impl.md` — **остановись**, дай пройти одному синку. Руками ничего не удалять.
- Получатель не удаляет свой `.response.md` «на уборку» — закрывает отправитель.

`scan_bodies` исключает `.response.md` / `.impl.md` / `.done.md` / `.ack.md` — квитанция и маркеры не считаются открытыми телами в экране синка.

## Автоматизация (v1.7.0)

- **Авто-синк** (`src/lib/stores/autosync.ts`, T-000136): фоновый таймер тикает раз в 60 с; если включён и прошёл интервал — последовательно вызывает `sync_project` по проектам. Ручной Sync в UI — тот же путь.
- **Авто-коммит** (T-000137, T-000190): если у репо настроена auto-commit-ветка и репо сейчас на ней — после sync приложение коммитит **только tracked** cross-repo doc-папки (pathspec-scoped, автором `Solo Dev Hub`, префикс `chore(sdh-sync):`), включая `docs/InOutBox` и **удаления** tracked-но-исчезнувших каталогов (мигрированный legacy-каталог удаляется тем же коммитом, что и появление файлов в InOutBox, — иначе архив воскреснет на pull). Проверка «tracked» — по индексу (`git ls-files`): каталог, который пользователь сам удалил через `git rm`, остаётся его собственным стейджом — приложение не присваивает чужое удаление и не падает на пустом pathspec. Gitignore’нутые inbox-папки пропускаются, `-f` не форсит. Репо на другой ветке — коммит пропускается молча: это состояние репо, а не ошибка синка (T-000198). `RepoDetail` показывает рядом с полем ветки «Авто-коммит на паузе: репо на ветке X, настроена Y» — ветка читается живьём при открытии репо (`get_repo_current_branch`, async, в БД не хранится).
- **Ошибки синка** (T-000198): `SyncResult.errors` — список `SyncIssue { code, subject, target?, detail? }` (`models/sync.rs` ↔ `types/sync.ts`), не строки. `code` — одно из 18 семейств (`copy_failed`, `root_missing`, `rule_held`, `migration_left_in_place`…); у `lookup_failed` в `target` — фиксированный токен того, что читалось (`renames`, `repos`, `home_dir`…); фронт переводит каждое через `syncIssue.<code>` (`formatSyncIssue` в `utils/sync-issues.ts`) и показывает по строке на замечание. `detail` — сырой текст ОС/git, не переводится. Одинаковое замечание за проход пишется один раз (`SyncCounters::push`).
- **Тред-модель** (T-000154): `sync_project` — `#[tauri::command(async)]`, тяжёлая работа (fs + git-субпроцессы) уходит с UI-потока, окно не морозится.

## Статусная модель (SyncScreen)

`list_project_requirements` анализирует файлы на обеих сторонах **во всех кандидатных каталогах пары** — новая InOutBox-полоса + legacy-остаток (полумигрированная пара отображается корректно):

| Статус | Условие | Значение |
|--------|---------|----------|
| `new` | файл у отправителя, нет у получателя | Нажми Sync — отправится |
| `sent` | файл у обоих, response нигде нет | Ждём ответа получателя |
| `responded` | есть `.response.md` на одной из сторон | Можно Confirm (✓) |

Reject-цикла нет (убран в 0.13.27): не устраивает ответ — новый `REQ-N+1`. Получатель может сам отредактировать свой `.response.md` — edit пропагейтится к отправителю при следующем sync.

## Где это реализовано

- **Скан**: `sync::scan_requirements` (`REQ-*.md`, кроме `.response.md` и `.impl.md`), `sync::scan_responses` (`*.response.md`), `sync::scan_impl_files` (`REQ-*.impl.md`) — все в `sync/fs.rs`; `sync::scan_bodies` (открытые тела канала) — в `sync/peer_channels.rs`.
- **Миграция**: `sync::migrate_req_pair` (`sync/requirements.rs`) — plan→apply с откатом, dedup, конфликт-сохранение, уборка опустевших legacy-каталогов.
- **Копирование**: `sync::sync_peer_pair` (T-000190) — kind-aware зеркало `outbox→inbox`, обратный поток `.response.md`, union `pact/`; `sync::copy_file_if_changed` внутри.
- **Закрытие**: `sync::confirm_pair` (ручной ✓, новая + legacy раскладки) и kind-aware teardown в `sync_peer_pair`; legacy-фолбэк `sync::auto_close_impl_pairs` живёт только для пары, чья миграция откатилась на этом проходе.
- **Оркестрация**: `run_project_sync` в `sync/project_sync.rs`; порядок в каждом направлении — rename-replay (legacy, потом InOutBox) → `migrate_req_pair` → унифицированный канал (teardown → обратный поток ответов → зеркало) → autocommit.
- **Guard**: `sync::ensure_root_exists` (B-001) — не создавать папку, если корня репо нет.

## Частые ошибки

- **Удалять файлы пары руками** → воскрешение base-REQ (см. failure mode). Закрывать только через ✓ или `.impl.md`, дальше не трогать.
- **Редактировать копию на чужой стороне**: non-owner копия молча перезаписывается при next sync. Свою сторону: sender → REQ, recipient → response.
- **Корневая папка репо не существует** (B-001): guard пишет ошибку в `SyncResult.errors`, sync продолжается с остальными.

## Что НЕ входит

- git push/pull файлов между репо (только локальное копирование). Коммит — auto-commit (v1.7.0) либо пользователь сам.
- Конфликты (не бывает по дизайну — один owner на файл).
- Автоматическое Confirm пользователем. Авто-close существует только через sender-подписанный `.impl.md`, не как молчаливое решение приложения.
