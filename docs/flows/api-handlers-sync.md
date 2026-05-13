# Flow: API / Handlers sync между репозиториями

**Введено в:** v0.9.0
**Связанные файлы:** [src-tauri/src/sync.rs](../../src-tauri/src/sync.rs), [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs), [src/lib/components/SyncScreen.svelte](../../src/lib/components/SyncScreen.svelte)

## Зачем

`api.md` и `handlers.md` фиксируют контракт сервера/микросервиса. Они должны автоматически доезжать до потребителей: клиенты читают контракт своего сервера, parent-сервер — контракты подключённых микросервисов. Поэтому вдобавок к REQ-*.md синхронизируется ещё и эта пара файлов — в одну сторону, без квитанций.

## 4 направления и пути

| Направление | У отправителя | У получателя |
|-------------|---------------|--------------|
| Server → Client (api) | `docs/api.md` | `docs/server-api/api.md` |
| Server → Client (handlers) | `docs/handlers.md` | `docs/server-api/handlers.md` |
| Microservice → Parent server (api) | `docs/api.md` в server-репо микросервиса | `docs/microservice-api/<ms-project-name>/api.md` у parent-сервера |
| Microservice → Parent server (handlers) | `docs/handlers.md` в server-репо микросервиса | `docs/microservice-api/<ms-project-name>/handlers.md` у parent-сервера |

**Правило папочного именования**: у получателя имя подпапки = роль отправителя (`server-api`) или имя microservice-проекта (`<ms-project-name>`). Свои документы у отправителя лежат в корне `docs/` — они не дублируются в подпапку.

**Источник истины для `<ms-project-name>`** — `projects.name` (не github_name). Тот же паттерн, что для REQ-*.md в 0.8.0.

## Rename-replay (T-000092, v0.29.0)

Когда microservice-проект переименован в приложении (`update_project` → `projects.name`), parent-сервер'у нужно переименовать у себя папку `docs/microservice-api/<old>/` → `<new>/`, иначе на следующем sync'е API/handlers скопируются в новую папку, а старая останется как мусор.

Механика симметрична `repo_renames` (v16):

- `update_project` сравнивает старое и новое имя, при отличии пишет строку в `project_renames` (id, project_id, old_name, new_name, renamed_at).
- На каждом sync'е, в цикле по микросервисам проекта, вызывается `db.list_renames_for_project(ms_project_id)` и для каждой записи — `sync::replay_rename_in_dir(<srv>/docs/microservice-api/, old, new)`.
- Идемпотентно по fs: replay = no-op, если `old/` уже нет (переименована раньше). Если и `old/`, и `new/` существуют одновременно — `Collision`, ошибка в `SyncResult.errors`, ручное вмешательство.
- Папка `microservice-requirements/<ms_canonical>/` rename'ится по `repo_renames` для server-репо микросервиса (это отдельная сущность с github_name → canonical_folder_name).

## Автомиграция клиента (одноразовая)

До 0.9.0 `api.md` на клиенте лежал в `docs/api.md`. В 0.9.0 target переехал в `docs/server-api/api.md`. При первом sync после обновления:

1. Если `docs/api.md` существует и `docs/server-api/api.md` — нет,
2. Приложение вызывает `sync::migrate_file(old, new)`: сначала `fs::copy(old, new)`, потом `fs::remove_file(old)`.
3. Если copy упал — `old` остаётся нетронутым (атомарность per-файл).
4. Счётчик `SyncResult.migrated` инкрементируется → в toast после sync'а добавляется строка "перенесено: N".

**handlers.md миграция не нужна**: до 0.9.0 он не синкался, поэтому на клиентах его быть не должно. Если пользователь положил руками — `scan` не происходит, trained copy просто перезапишет новым при sync'е (если сервер теперь им обладает).

## Поведение при отсутствии файла у отправителя

`api.md` / `handlers.md` считаются **опциональными**. Если у отправителя файла нет — sync молча пропускает этот файл для данной стороны:

- Не добавляет запись в `SyncResult.errors`.
- Не создаёт строку в UI-списке `list_project_requirements`.

Исключение: если у сервера нет `docs/api.md` и при этом есть клиенты — `sync_project` до 0.9.0 писал в errors "api.md not found". Это поведение сохранено как подсказка: сервер без api.md обычно неожиданность, стоит предупредить.

## Ошибки на стороне микросервиса

Для microservice → parent sync нужен server-репо микросервиса. Если `server_repo_of_microservice(ms_id)` возвращает Err (0 или ≥2 server-репо) — sync записывает ошибку в `errors` и пропускает этот микросервис. Остальные микросервисы продолжают синхронизироваться. Поведение идентично REQ-*.md sync'у с 0.8.0.

## Пример файловой структуры

Для стандартного проекта "web-app" (server + client) с подключённым microservice-project "auth-service" (у которого свой server-репо "auth-backend"):

```
web-app-backend/docs/         (свой server)
├─ api.md                      ← первоисточник (server → client)
├─ handlers.md                 ← первоисточник (server → client)
├─ microservice-api/
│  └─ auth-service/            ← имя microservice-проекта
│     ├─ api.md                ← пришёл от auth-backend
│     └─ handlers.md           ← пришёл от auth-backend
├─ client-requirements/        (см. requirements-sync.md)
└─ microservice-requirements/  (см. requirements-sync.md)

web-app-frontend/docs/         (client)
├─ server-api/
│  ├─ api.md                   ← пришёл от web-app-backend
│  └─ handlers.md              ← пришёл от web-app-backend
└─ backend-requirements/       (см. requirements-sync.md)

auth-backend/docs/             (server-репо микросервиса auth-service)
├─ api.md                      ← первоисточник (microservice → parent)
├─ handlers.md                 ← первоисточник (microservice → parent)
└─ server-requirements/        (см. requirements-sync.md)
```

## UI (SyncScreen)

`RequirementInfo.direction` расширен двумя значениями:

| direction | Секция в SyncScreen |
|-----------|---------------------|
| `client_to_server` | Клиент → Сервер (REQ-*.md) |
| `server_to_client` | Сервер → Клиент (api.md, handlers.md, REQ-*.response.md) |
| `server_to_microservice` | Сервер → Микросервис (REQ-*.md) |
| **`microservice_to_server_api`** | **Микросервис → Сервер (API)** |
| **`microservice_to_server_handlers`** | **Микросервис → Сервер (Handlers)** |

У двух новых направлений нет confirm/reject (не требование-квитанция, а one-way copy). Статусы: `new` (файл появился или изменился) / `sent` (в назначении контент идентичный).

Результат sync-операции в toast'е: если `migrated > 0` — используется ключ `sync.syncCompleteFull` ("Скопировано: {0}, квитанций: {1}, перенесено: {2}"), иначе обычный `sync.syncComplete`.
