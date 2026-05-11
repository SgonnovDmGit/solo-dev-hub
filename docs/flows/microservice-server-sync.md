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

## Triangular flow (client → server → microservice)

С v0.25.0 (T-000078) global CLAUDE.md template содержит секцию `## Forwarding (triangular flow)` с правилами для случая, когда client REQ требует работы подключённого микросервиса. Здесь — file-system lifecycle и где Solo Dev Hub помогает / не помогает.

### Когда триангуляция случается

Client пишет REQ серверу. Server-LLM при процессинге классифицирует scope:
- **server-only** — обычный двусторонний flow (см. [requirements-sync.md](requirements-sync.md)).
- **MS-only** — вся работа лежит на стороне подключённого микросервиса. Server forward'ит REQ вниз.
- **mixed** — часть на сервере, часть на одном или нескольких микросервисах. Server делает свою часть параллельно с forward'ом MS-частей.

Источник truth для классификации — `docs/project.md` (auto-generated by Solo Dev Hub) секции `## Repositories` и `## Connected microservices` плюс текущий handler-surface сервера.

### Lifecycle (MS-only пример)

1. **Client → Server.** Client пишет `REQ-005_avatar-storage.md` в собственный `docs/backend-requirements/`. На sync Solo Dev Hub копирует на server-сторону в `docs/client-requirements/<client-repo>/REQ-005_avatar-storage.md`.

2. **Server forward'ит вниз.** Server-LLM открывает REQ, классифицирует scope как MS-only. Создаёт `docs/microservice-requirements/ms-storage/REQ-001_blob-storage.md` с reformulated body (server↔MS контракт без упоминания "client") и `**Forwarded-from:** <client-repo>/REQ-005` header сразу после H1. **Не пишет client receipt.**

3. **Sync пропагирует downstream.** Solo Dev Hub копирует новый downstream REQ в `ms-storage-server/docs/server-requirements/REQ-001_blob-storage.md`. `Forwarded-from:` header едет вместе с телом REQ.

4. **MS обрабатывает.** MS-LLM открывает REQ, **игнорирует `Forwarded-from:` header** (правило Rule 2.1 — это metadata для server-side chain-tracing). Делает работу, пишет `REQ-001_blob-storage.response.md` со `## Status: implemented`.

5. **Sync пропагирует receipt вверх.** Solo Dev Hub копирует receipt обратно в server'ский `docs/microservice-requirements/ms-storage/REQ-001_blob-storage.response.md`.

6. **Server консолидирует.** Server-LLM (возможно в новой сессии) при scan'е обнаруживает MS receipt. По `Forwarded-from:` header в downstream REQ восстанавливает link к client REQ. Пишет consolidated client receipt `docs/client-requirements/<client-repo>/REQ-005_avatar-storage.response.md` со `## Status: implemented` и `## Comment:` про функциональный исход. **Receipt не упоминает microservice** — server's implementation strategy opaque to client.

7. **Confirm.** Sync пропагирует receipt клиенту. Client видит → confirms через app's ✓ → атомарное удаление пары на обеих сторонах. Downstream REQ-001 / response.md на server↔MS оси confirm'ится отдельно когда server считает нужным.

### Resuming across sessions

Chain может span'ить несколько сессий. При открытии новой сессии server-LLM:

1. Сканирует `docs/client-requirements/*/REQ-*.md` без `.response.md` — открытые client REQ'ы.
2. Для каждого: grep `docs/microservice-requirements/*/REQ-*.md` на `Forwarded-from: <client-repo>/REQ-NNN` header.
3. **Найден downstream с response.md** → шаг 6 lifecycle (consolidated client receipt).
4. **Найден без response.md** → chain in-flight, ждать.
5. **Не найден** → forward'ы ещё не созданы, классифицировать scope и действовать.

Этот scan возможен только благодаря `Forwarded-from:` header'у — без него filesystem не позволил бы восстановить связку.

### Multi-MS forwarding

Если client REQ требует работы нескольких микросервисов:
- Server создаёт **N** downstream REQ — по одному в каждый `docs/microservice-requirements/<ms>/`.
- В каждом — собственный `REQ-NNN` (отдельный per-folder counter, не shared между папками).
- `Forwarded-from:` header — **одинаковое** значение во всех (одна client-REQ-ID, разные MS-REQ файлы).
- Server ждёт **все** receipt'ы перед consolidated client receipt'ом.
- Consolidated receipt описывает исход per microservice ("реализованы оба компонента: ... и ...").

### Clarification-needed loop

Если recipient (server или MS) не понимает scope incoming REQ:

1. Пишет receipt со `## Status: clarification-needed` и `## Comment:` с конкретными вопросами.
2. Sender в следующей сессии видит receipt → обновляет **оригинальный** REQ inline с уточнениями (**не** новый REQ-N+1).
3. Sync пропагирует обновлённый REQ.
4. Recipient re-evaluate scope, **перезаписывает** свой receipt с финальным status'ом.
5. Loop продолжается пока sender не напишет complete REQ.

Receipts мутабельны на recipient-стороне; sender никогда не редактирует receipt recipient'а.

### Где Solo Dev Hub помогает / не помогает

| Шаг | Solo Dev Hub | LLM |
|---|---|---|
| Copy REQ через sync | ✓ | — |
| Copy receipt обратно | ✓ | — |
| Atomic delete pair при confirm ✓ | ✓ | — |
| Классификация scope | — | ✓ (по project.md + handler surface) |
| Reformulation REQ для MS | — | ✓ |
| `Forwarded-from:` header | — | ✓ |
| Scan / grep in-flight chains | — | ✓ (в начале сессии) |
| Consolidation client receipt | — | ✓ |
| Validation receipt format | — | пока ✓ (planned F-000039 будет parse'ить `## Status:`) |

В будущем (F-000039, v2.0.0) приложение начнёт parse'ить `## Status:` line и автоматически confirm'ить `implemented` receipt'ы. Сейчас все правила — convention для LLM/человека, без runtime-enforcement.

### Cross-reference

Нормативный текст правил — в global CLAUDE.md section:
- `## Receipt format` — формат receipt'ов (4 status values, hard-enforce) и `clarification-needed` workflow
- `## Forwarding (triangular flow)` — Server-side responsibility, Linkage header, MS-side responsibility

## Миграция v12

У существующих пользователей:
- `role='microservice'` репо → `role=NULL` (освобождаются для других ролей)
- Таблица `project_microservices` пересоздаётся пустой (старая семантика `(parent_project_id, microservice_repo_id)` несовместима)
- **Связи теряются**. В Changelog — пошаговая инструкция пересоздания

## API / Handlers в обратную сторону (microservice → parent server)

С v0.9.0 microservice-проект, подключённый к parent'у, отдаёт ему свои `docs/api.md` и `docs/handlers.md`. На parent-сервере они попадают в `docs/microservice-api/<ms-project-name>/{api,handlers}.md`. Подробности, включая поведение при отсутствии файлов и при 0/>1 server-репо в микросервисе — [api-handlers-sync.md](api-handlers-sync.md).
