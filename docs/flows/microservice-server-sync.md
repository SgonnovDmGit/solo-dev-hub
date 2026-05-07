# Flow: Микросервис ↔ Сервер

**Введено в:** v0.8.0 (F-012)  
**Связанные файлы:** [src-tauri/src/db.rs](../../src-tauri/src/db.rs), [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs), [src/lib/components/ProjectDetail.svelte](../../src/lib/components/ProjectDetail.svelte)

## Модель

Микросервис — это **тип проекта**, не роль репозитория.

- `projects.project_type ∈ {'standard', 'microservice'}`
- Microservice-проект содержит свои репо (обычно: один server + опционально клиенты для разработки/админки)
- **Правило**: microservice-проект обязан содержать **ровно один репо с role='server'**. Sync проверяет при обращении, не при сохранении
- Parent-проекты "подключают" microservice-проекты через таблицу `project_microservices (project_id, microservice_project_id)`

## Ограничения

| Правило | Где проверяется | Что происходит при нарушении |
|---------|-----------------|------------------------------|
| `connect(parent, ms)` требует ms.project_type='microservice' | `db.connect_microservice` | Err "Target project is not of type 'microservice'" |
| Нет self-loop (A → A) | SQL `CHECK (project_id != microservice_project_id)` | Err от SQLite |
| Нет транзитивных циклов (A→B→C→A) | DFS в `db.is_reachable` | Err "Cycle detected: target already references this project transitively" |
| Microservice-проект с parent'ами нельзя удалить | `db.delete_project` | Err "Microservice project has N parent(s) — disconnect them first" |
| **Смена `project_type` блокируется только если проект — microservice, подключённый к parent'ам** | `db.update_project_type` | Err "Project is connected to parents as a microservice — disconnect first" |

**Цепочки разрешены**: microservice-проект может подключать свои микросервисы (если нет цикла).

### Про смену типа — nuance

Репо внутри проекта и свои подключённые микросервисы **не мешают** смене типа. Только `parentsOfMicroservice.length > 0` — иначе parent-проекты ссылались бы на standard как на microservice, что ломает семантику.

Примеры:
- Standard с 5 репо → microservice: ✅ разрешено (user отвечает за то, чтобы в microservice был ровно один server-репо)
- Microservice без parent'ов → standard: ✅ разрешено
- Microservice, подключённый к web-app → standard: ❌ Err (сначала disconnect в ProjectDetail web-app)

## UI-флоу создания и подключения

### 1. Создание microservice-проекта
1. Sidebar → "+ Создать проект"
2. Ввести имя + описание
3. **Выбрать тип: Микросервис**
4. В созданном проекте добавить репо с role='server' (обычно бэкенд микросервиса)

### 2. Подключение к parent-проекту
1. Открыть standard-проект (parent)
2. В секции "Microservices" — список всех microservice-проектов в системе
3. Toggle "Подключить" возле нужного микросервиса
4. Если backend вернёт ошибку cycle — toast с переводом `toast.cycleDetected`

### 3. Проверка подключений микросервиса
1. Открыть microservice-проект
2. Секция "Подключён к проектам" показывает список parent'ов
3. Клик по parent'у — переход на его ProjectDetail

## Sync-флоу (server → microservice)

Триггер: кнопка Sync на экране проекта или SyncScreen.

```
Parent standard project "web-app"
├─ server-repo "web-app-backend" (role=server)
└─ подключённые microservice-проекты:
   ├─ "auth-service" (microservice)
   │  └─ server-repo "auth-backend" (role=server)
   └─ "billing-service" (microservice)
      └─ server-repo "billing-backend" (role=server)
```

**Что делает `sync_project` для каждого подключённого microservice-проекта**:

1. `server_repo_of_microservice(ms_project_id)` → получает server-репо (или Err)
2. Если Err (`0 servers` или `2+ servers`) — запись в errors, пропуск микросервиса
3. Копирует REQ-*.md из `web-app-backend/docs/microservice-requirements/<ms-project-name>/` в `ms-server-repo/docs/server-requirements/`
4. Копирует .response.md обратно из `ms-server-repo/docs/server-requirements/` в `web-app-backend/docs/microservice-requirements/<ms-project-name>/`
5. Имя папки subfolder — **`ms-project.name`** (не github_name!), т.к. микросервис теперь проект, а не репо

### Пример файловой структуры

У сервера parent-проекта:
```
web-app-backend/docs/
├─ microservice-requirements/
│  ├─ auth-service/                      ← имя microservice-проекта
│  │  ├─ REQ-001_login-api.md
│  │  └─ REQ-001_login-api.response.md  ← пришёл от auth-backend
│  └─ billing-service/
│     └─ REQ-002_payment-api.md
```

У server-репо микросервиса (`auth-backend`):
```
auth-backend/docs/
└─ server-requirements/
   ├─ REQ-001_login-api.md          ← пришёл от web-app-backend
   └─ REQ-001_login-api.response.md ← ответ перед возвратом
```

**Flat на стороне микросервиса**: у микросервиса обычно один parent-сервер (а если несколько — приходят в одну папку, по именам REQ не конфликтуют т.к. источник один).

## Миграция v12

У существующих пользователей:
- `role='microservice'` репо → `role=NULL` (освобождаются для других ролей)
- Таблица `project_microservices` пересоздаётся пустой (старая семантика `(parent_project_id, microservice_repo_id)` несовместима)
- **Связи теряются**. В Changelog — пошаговая инструкция пересоздания

## API / Handlers в обратную сторону (microservice → parent server)

С v0.9.0 microservice-проект, подключённый к parent'у, отдаёт ему свои `docs/api.md` и `docs/handlers.md`. На parent-сервере они попадают в `docs/microservice-api/<ms-project-name>/{api,handlers}.md`. Подробности, включая поведение при отсутствии файлов и при 0/>1 server-репо в микросервисе — [api-handlers-sync.md](api-handlers-sync.md).
