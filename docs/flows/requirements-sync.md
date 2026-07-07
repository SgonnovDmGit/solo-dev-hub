# Flow: Requirements Sync (REQ ↔ Response ↔ Impl)

**Введено в:** v0.2.0 (F-012 в v0.8.0; nested-per-parent + rename-replay в v0.14.0/F-033; `.impl.md` авто-close в v0.26.x/F-000039; авто-синк/авто-коммит в v1.7.0)
**Смежный flow**: api.md/handlers.md one-way copy — [api-handlers-sync.md](api-handlers-sync.md).
**Связанные файлы (после распила монолитов):**
- `src-tauri/src/commands/sync.rs` — Tauri-команды `sync_project` (async, T-000154), `list_project_requirements`, `confirm_requirement`
- `src-tauri/src/sync/project_sync.rs` — `run_project_sync`: оркестрация одного прохода (все три направления + авто-close + авто-коммит)
- `src-tauri/src/sync/requirements.rs` — `confirm_pair`, `auto_close_impl_pairs`, rename-replay хелперы
- `src-tauri/src/sync/fs.rs` — `scan_requirements` / `scan_responses` / `scan_impl_files` / `copy_file_if_changed`
- [src/lib/components/SyncScreen.svelte](../../src/lib/components/SyncScreen.svelte) — UI; `src/lib/stores/autosync.ts` — фоновый таймер

## Суть

Требования между репо передаются через **локальные MD-файлы**, а не через git push. Приложение копирует `REQ-*.md` файлы между папками репо по паттерну «сообщение-квитанция». Merge-конфликтов нет по дизайну: у каждого файла один owner, вторая сторона read-only через протокол.

## Паттерн «сообщение-квитанция»

1. **Отправитель** создаёт `REQ-XXX_topic.md` в своей папке (`docs/<role>-requirements/`).
2. **Sync** копирует файл на сторону получателя в соответствующую subfolder. Если отправитель редактирует REQ позже — next sync перезапишет копию у получателя (**sender = source of truth** для `.md`).
3. **Получатель** реализует и создаёт `.response.md` рядом с копией запроса (**не редактирует исходный файл**).
4. **Sync** копирует `.response.md` обратно отправителю (**recipient = source of truth** для `.response.md`).
5. **Отправитель** смотрит ответ и **закрывает пару** — ручным ✓ или файлом `.impl.md` (см. «Закрытие пары»). Если ответ не устраивает — создаёт новый `REQ-N+1_<slug>.md` с уточнённым запросом (reject-flow убран в 0.13.27).

Тройка файлов одной пары: `REQ-XXX.md` (base, sender) + `REQ-XXX.response.md` (receipt, recipient) + опционально `REQ-XXX.impl.md` (close-signal, sender).

## Три направления

### Client → Server

```
Отправитель: client
Получатель: server
├─ client/docs/backend-requirements/REQ-001_feature.md                              (пишет клиент)
├─ server/docs/client-requirements/<client-canonical>/REQ-001_feature.md            ← copied by sync
└─ server/docs/client-requirements/<client-canonical>/REQ-001_feature.response.md   (пишет сервер)
   → client/docs/backend-requirements/REQ-001_feature.response.md                   ← copied back
```

**Subfolder на сервере**: canonical repo name клиента (last segment of github_name, e.g. `owner/foo-bar` → `foo-bar`) — `Repository::canonical_folder_name()`.

### Server → Microservice (F-012)

Микросервис — отдельный проект; требования идут в его **server-репо**.

```
Отправитель: parent-server
Получатель: ms-server-repo (role=server внутри microservice-project)
├─ parent-server/docs/microservice-requirements/<ms-canonical>/REQ-002_api.md            (пишет parent)
├─ ms-server-repo/docs/server-requirements/<parent-canonical>/REQ-002_api.md             ← copied by sync
└─ ms-server-repo/docs/server-requirements/<parent-canonical>/REQ-002_api.response.md    (пишет microservice)
   → parent-server/docs/microservice-requirements/<ms-canonical>/REQ-002_api.response.md ← copied back
```

**Subfolder на стороне микросервиса**: `parent-server-repo.canonical_folder_name()` — **nested per parent** (F-033) для multi-parent микросервисов. One-time миграция flat → nested при первом sync после 0.14.0 (attribution по byte-equal копиям на parent-стороне).

### Server → Client (api/handlers)

Не REQ-цикл, а односторонняя передача контракта: `docs/api.md` + `docs/handlers.md` + `docs/my_api/*.md` сервера копируются в `docs/server-api/` клиента. Подробно — [api-handlers-sync.md](api-handlers-sync.md).

### Rename handling (F-033)

При переименовании репы (detect в `upsert_repository_with_outcome` по github_id-match + разном github_name) запись пишется в `repo_renames`. На следующем sync preamble `replay_rename_in_dir` переименовывает counterparty-side папку со старого canonical на новый (идемпотентно — state держит сама fs, поля «applied» нет). History — Settings → Rename Log.

## Закрытие пары

Закрытие = **удаление всей тройки на обеих сторонах**. Два пути, оба выполняет **приложение**, не человек и не LLM руками:

1. **Ручной ✓** (`confirm_requirement` → `confirm_pair`). Пользователь жмёт ✓ в SyncScreen на `responded`-паре. Сносит `.md` + `.response.md` + `.impl.md` на sender- и recipient-стороне.
2. **`.impl.md` (LLM-authorable, F-000039).** Отправитель кладёт `REQ-XXX_<slug>.impl.md` рядом со своим REQ в **своей outgoing**-папке. На следующем sync `auto_close_impl_pairs` сканирует sender-папку на `REQ-*.impl.md` и сносит всю тройку на обеих сторонах. Идемпотентно; вызывается по всем трём направлениям в `run_project_sync`. Это даёт sender-LLM закрывать свои петли без человеческого ✓.

**`.impl.md` — только отправитель, только в своей outgoing-папке.** Существование файла — сигнал; содержимое игнорируется (пустой ок). Микросервис `.impl.md` не пишет — на recipient-стороне incoming-папка перезаписывается синком, файл недоставляем.

> **Порядок удаления в коде:** `auto_close_impl_pairs` (и `confirm_pair`) удаляют **base первым**, затем response, затем impl. То есть если приложение оборвётся на полпути (например, Windows-лок на удалении) — первым исчезнет именно base. Это диагностический признак: см. failure mode ниже.

## Failure mode — воскрешение base-REQ (важно)

Симптом: сервер видит требование, которое уже выполнено, как «открытое» — `REQ-XXX.md` приходит с клиента снова и снова, а `.response.md`/`.impl.md` пропали.

Причина: **кто-то удалил файлы пары руками, но оставил base `REQ-XXX.md` на стороне отправителя.** Так как sender — source of truth для base, **каждый** sync заново копирует его получателю. Если при этом `.impl.md` удалён до того, как sync успел отработать авто-close — авто-close уже не сработает (нет сигнала), и base воскресает бесконечно.

Как отличить от бага приложения: приложение удаляет base **первым**, поэтому «base жив, а response+impl исчезли» приложение произвести не может — это всегда внешнее (ручное) вмешательство.

Профилактика (закреплено в скиллах `sdh-cross-repo-req-send` / `-answer`):
- Положил `.impl.md` — **остановись**, дай пройти одному синку. Не удаляй ни base, ни response, ни `.impl.md` руками — приложение снесёт всё само.
- Получатель не удаляет свой `.response.md` «на уборку» — закрывает отправитель.

Код-сеть (v1.9.5): `scan_requirements` теперь исключает `*.impl.md`, чтобы застрявший (при сбое teardown) impl-файл не был принят за требование и не уехал получателю фантомной парой.

## Автоматизация (v1.7.0)

- **Авто-синк** (`src/lib/stores/autosync.ts`, T-000136): фоновый таймер тикает раз в 60 с; если включён и прошёл интервал — последовательно вызывает `sync_project` по проектам. Ручной Sync в UI — тот же путь.
- **Авто-коммит** (T-000137): если у репо настроена auto-commit-ветка и репо сейчас на ней — после sync приложение коммитит **только tracked** cross-repo doc-папки (pathspec-scoped, автором `Solo Dev Hub`, префикс `chore(sdh-sync):`). Gitignore’нутые inbox-папки пропускаются, `-f` не форсит.
- **Тред-модель** (T-000154): `sync_project` — `#[tauri::command(async)]`, тяжёлая работа (fs + git-субпроцессы) уходит с UI-потока, окно не морозится.

## Статусная модель (SyncScreen)

`list_project_requirements` анализирует файлы на обеих сторонах:

| Статус | Условие | Значение |
|--------|---------|----------|
| `new` | файл у отправителя, нет у получателя | Нажми Sync — отправится |
| `sent` | файл у обоих, response нигде нет | Ждём ответа получателя |
| `responded` | есть `.response.md` на одной из сторон | Можно Confirm (✓) |

Reject-цикла нет (убран в 0.13.27): не устраивает ответ — новый `REQ-N+1`. Получатель может сам отредактировать свой `.response.md` — edit пропагейтится к отправителю при следующем sync.

## Где это реализовано

- **Скан**: `sync::scan_requirements` (`REQ-*.md`, кроме `.response.md` и `.impl.md`), `sync::scan_responses` (`*.response.md`), `sync::scan_impl_files` (`REQ-*.impl.md`) — все в `sync/fs.rs`.
- **Копирование**: `sync::copy_file_if_changed(src, dst)` — перезаписывает только при отличии содержимого.
- **Закрытие**: `sync::confirm_pair` (ручной ✓) и `sync::auto_close_impl_pairs` (авто) — в `sync/requirements.rs`.
- **Оркестрация**: `run_project_sync` в `sync/project_sync.rs`; порядок в каждом направлении — сперва rename-replay, затем `auto_close_impl_pairs`, затем copy REQ (sender→recipient), затем copy-back response.
- **Guard**: `sync::ensure_root_exists` (B-001) — не создавать папку, если корня репо нет.

## Частые ошибки

- **Удалять файлы пары руками** → воскрешение base-REQ (см. failure mode). Закрывать только через ✓ или `.impl.md`, дальше не трогать.
- **Редактировать копию на чужой стороне**: non-owner копия молча перезаписывается при next sync. Свою сторону: sender → REQ, recipient → response.
- **Корневая папка репо не существует** (B-001): guard пишет ошибку в `SyncResult.errors`, sync продолжается с остальными.

## Что НЕ входит

- git push/pull файлов между репо (только локальное копирование). Коммит — auto-commit (v1.7.0) либо пользователь сам.
- Конфликты (не бывает по дизайну — один owner на файл).
- Автоматическое Confirm пользователем. Авто-close существует только через sender-подписанный `.impl.md`, не как молчаливое решение приложения.
