# Flow: Requirements Sync (REQ ↔ Response)

**Введено в:** v0.2.0 (обновлено в v0.8.0 для F-012)
**Смежный flow**: api.md/handlers.md one-way copy — [api-handlers-sync.md](api-handlers-sync.md) (добавлен в v0.9.0).
**Связанные файлы:** [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs) (`sync_project`, `list_project_requirements`, `confirm_requirement`), [src-tauri/src/sync.rs](../../src-tauri/src/sync.rs), [src/lib/components/SyncScreen.svelte](../../src/lib/components/SyncScreen.svelte)
**Исходная спека:** [docs/doc4_sync_spec.md](../doc4_sync_spec.md)

## Суть

Требования между репо передаются через **локальные MD-файлы**, а не через git push. Приложение копирует `REQ-*.md` файлы между папками репо по паттерну "сообщение-квитанция". Никаких merge-конфликтов по дизайну: каждый пишет только в свою папку.

## Паттерн "сообщение-квитанция"

1. **Отправитель** создаёт `REQ-XXX_topic.md` в своей папке (`docs/<role>-requirements/`)
2. **Sync** копирует файл на сторону получателя в соответствующую subfolder. Если отправитель редактирует REQ позже — next sync перезапишет копию у получателя (sender = source of truth).
3. **Получатель** реализует и создаёт `.response.md` рядом с копией запроса (**не редактирует исходный файл**)
4. **Sync** копирует `.response.md` обратно отправителю. Получатель может редактировать свою копию — next sync перезапишет копию у отправителя (recipient = source of truth для response).
5. **Отправитель** смотрит ответ, в UI жмёт ✓ Confirm → обе стороны удаляют файлы. Если ответ не устраивает — создаёт новый `REQ-N+1_<slug>.md` с уточнённым запросом (reject-flow убран в 0.13.27, см. ниже).

## Три направления

### Client → Server

```
Отправитель: client
Получатель: server
├─ client/docs/backend-requirements/REQ-001_feature.md           (пишет клиент)
├─ server/docs/client-requirements/<client-display-name>/REQ-001_feature.md          ← copied by sync
└─ server/docs/client-requirements/<client-display-name>/REQ-001_feature.response.md (пишет сервер)
   → client/docs/backend-requirements/REQ-001_feature.response.md                    ← copied back
```

**Subfolder на сервере**: **canonical repo name** клиента (last segment of github_name, e.g. `owner/foo-bar` → `foo-bar`) — см. `Repository::canonical_folder_name()` (v0.14.0+).

### Server → Microservice (F-012 update)

Раньше (до v0.8.0) микросервис был репо с role='microservice'. С v0.8.0 микросервис — отдельный проект, требования идут в **server-репо микросервиса**.

```
Отправитель: parent-server (standard-project)
Получатель: ms-server-repo (role=server внутри microservice-project)
├─ parent-server/docs/microservice-requirements/<ms-canonical>/REQ-002_api.md           (пишет parent)
├─ ms-server-repo/docs/server-requirements/<parent-canonical>/REQ-002_api.md            ← copied by sync
└─ ms-server-repo/docs/server-requirements/<parent-canonical>/REQ-002_api.response.md   (пишет microservice)
   → parent-server/docs/microservice-requirements/<ms-canonical>/REQ-002_api.response.md ← copied back
```

**Subfolder на parent-сервере**: `ms-server-repo.canonical_folder_name()` (v0.14.0+ — раньше было `ms-project.name`).
**Subfolder на стороне микросервиса**: `parent-server-repo.canonical_folder_name()` — **nested per parent** с v0.14.0 (снимает collision для multi-parent microservice; раньше было flat `server-requirements/`). One-time migration при первом sync после 0.14.0 автоматически раскладывает flat файлы по subfolder'ам через attribution к parent-side копиям (byte-equal match).

### Rename handling (v0.14.0+)

При переименовании репы (detect'ится в `upsert_repository_with_outcome` по github_id-match + разный github_name) запись добавляется в таблицу `repo_renames`. На следующем sync app запускает preamble `replay_rename_in_dir` на counterparty-side parent-директории — если там есть папка со старым canonical name и нет с новым, делает `fs::rename`. Идемпотентно — нет state-поля "applied", fs сама state. History видна в Settings → Rename Log (`SettingsRenameLog.svelte`).

### Server → Client (api/handlers)

Не REQ-цикл, а односторонняя передача контракта: `docs/api.md` + `docs/handlers.md` сервера копируются в `docs/server-api/` клиента. Подробно — [api-handlers-sync.md](api-handlers-sync.md).

## Статусная модель (SyncScreen)

`list_project_requirements` анализирует файлы на обеих сторонах и возвращает список:

| Статус | Условие | Значение для пользователя |
|--------|---------|---------------------------|
| `new` | файл у отправителя, нет у получателя | Нажми Sync — отправится |
| `sent` | файл у обоих, response нигде нет | Ждём ответа получателя |
| `responded` | есть `.response.md` на одной из сторон | Можно Confirm/Reject |

### Confirm

Отправитель подтверждает что всё ок. `confirm_requirement`:
- Удаляет `REQ-XXX.md` и `REQ-XXX.response.md` с **обеих** сторон атомарно в одной операции
- После этого sync не будет их восстанавливать (их нет нигде)

### Нет Reject (удалён в 0.13.27)

Reject-цикл убран. Если ответ не устраивает — создаётся новый `REQ-N+1_<slug>.md` с clarified ask. Receipient может также самостоятельно отредактировать свой `.response.md` — edit propagate'ится к отправителю при следующем sync.

## Где это всё реализовано

- **Сканирование**: `sync::scan_requirements(dir)` (файлы `REQ-*.md` кроме `.response.md`), `sync::scan_responses(dir)` (`*.response.md`)
- **Копирование**: `sync::copy_file_if_changed(src, dst)` — перезаписывает если содержимое отличается (propagates edits от source of truth к копии). REQ — sender owns, response — recipient owns.
- **Guard**: `sync::ensure_root_exists` (B-001) — не создавать папку если корня репо нет

## UI (SyncScreen)

Группировка в SyncScreen (с v0.5.0 B-002):
- Верхний уровень: направление (client→server, server→client, server→microservice)
- Внутри каждого направления: subgroup по `source_repo`

Для responded — кнопка ✓ Confirm.

## Частые ошибки

- **Редактировать копию на другой стороне**: non-owner копия silent'но перезаписывается при next sync. Редактировать только свою сторону (sender → REQ, recipient → response).
- **Несоответствие имени папки**: subfolder формируется из `display_name`/`project.name`. Если переименовать проект/репо в UI, старые subfolders остаются на диске. Ручная уборка (в v0.14.0 — F-033 — app будет переименовывать автоматически через rename-log).
- **Корневая папка репо не существует** (B-001): guard записывает ошибку в `SyncResult.errors`, sync продолжается с остальными.

## Что НЕ входит

- git commit/push файлов. Пользователь сам коммитит.
- Конфликты (не бывает по дизайну — каждый файл имеет одного owner'а, non-owner сторона read-only через протокол).
- Автоматическое Confirm. Только ручное через UI (✓).
