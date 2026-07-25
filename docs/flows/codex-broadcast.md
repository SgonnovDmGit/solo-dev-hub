# Flow: Codex broadcast (свод практик → консумеры)

**Введено в:** v1.11.0 (T-000162…171)
**Связанные файлы:**
- [src-tauri/src/sync/codex.rs](../../src-tauri/src/sync/codex.rs) — broadcast-движок (`broadcast_codex_to_consumer`)
- [src-tauri/src/sync/fs.rs](../../src-tauri/src/sync/fs.rs) — `mirror_dir` (полное зеркало + прунинг)
- [src-tauri/src/sync/project_sync.rs](../../src-tauri/src/sync/project_sync.rs) — `run_codex_broadcast` (врезка в оркестратор)
- [src-tauri/src/db/projects.rs](../../src-tauri/src/db/projects.rs) — `repo_of_project_with_role`, `list_connected_codices`, гейт состава, смена типа
- [src-tauri/src/sync/project_md.rs](../../src-tauri/src/sync/project_md.rs) — секция «Connected Codices»
- Дизайн-док (12 решений): `docs/superpowers/specs/2026-07-25-codex-informational-service-design.md`

## Модель

**Codex — это тип проекта** (`projects.project_type = 'codex'`), «издательский» микросервис: его продукт не эндпоинты, а **свод инженерных практик** (`rules/`, `cases/`, `lessons/`), который зеркалится read-only подключённым консумерам. Структурно это тот же механизм, что api-контракт (продюсер держит каноническую папку, приложение зеркалит её в консумеров), но с другой полезной нагрузкой: консумер-LLM трактует свод как «уроки, которые применяю», а не «поверхность, которую вызываю».

**Роль репо внутри Codex-проекта = «полоса аудитории» (lane).** Codex переиспользует существующие роли `server` и `admin_client` — новых ролей нет. `role=server` в Codex-проекте значит «серверная полоса свода», а не запущенный сервер; тип проекта снимает двусмысленность (в managed CLAUDE.md читается как «Type: 📚 Codex · Role: server»).

## Полосы и role-matched доставка

Репо полосы R вещает свой свод в **одноимённую** полосу каждого подключённого консумера. Доставка **role-matched, per-lane, best-effort**:

| Codex-полоса (источник) | Консумер (приёмник) |
|---|---|
| `server` → `<codex-server-repo>/practices/` | `<consumer-server-repo>/docs/codex/<codex-name>/` |
| `admin_client` → `<codex-admin-repo>/practices/` | `<consumer-admin-repo>/docs/codex/<codex-name>/` |

- Путь-источник берётся из `repositories.codex_source_path` (миграция v29, nullable → дефолт `practices`).
- Нет у Codex репо полосы R → эта полоса просто не публикуется (**silent skip**).
- Есть полоса R у Codex, но у консумера нет репо роли R (или нет `local_path`) → **skip + error** (ожидалась role-matched доставка).
- Пропал сам источник (папка практик отсутствует) → **error** (`mirror_dir` возвращает Err, зеркало никогда не «стирает» приёмник впустую).

## Приёмник — gitignored-инбокс

`docs/codex/<имя-свода>/` в консумере — **полное read-only зеркало** практик-папки: `mirror_dir` копирует изменённые файлы (`copy_file_if_changed`) и **прунит** исчезнувшие (файл убран из свода → удаляется у консумера). Папка `docs/codex/` в managed `.gitignore` → это локальный инбокс, авто-коммит её пропускает, `git status` чист. Консумер-LLM свод **не правит** (любая правка перетрётся на следующем синке); добавления/изменения — через REQ в сторону Codex (скилл `sdh-codex-consume`).

## Направления синка (обе идемпотентны)

`run_codex_broadcast` вызывается в оркестраторе после микросервисных блоков, до авто-коммита:

- **Consumer-driven:** при синке любого проекта он тянет свод каждого подключённого Codex (`list_connected_codices(project_id)`) в свои одноимённые репо.
- **Codex-driven:** если синкаемый проект сам Codex — он пушит свод в каждого консумера (`list_parents_of_microservice(codex_id)` — рёбра лежат в той же таблице `project_microservices`).

Счётчики: записи зеркала идут в `copied`, прунинг — в `migrated`, ошибки полос — в `errors`.

## Ветвление по (type, role)

- **api-контракт пропускает Codex автоматически:** `list_project_microservices` фильтрует по `project_type='microservice'`, поэтому codex-проекты не попадают в микросервисный путь синка (у свода нет `docs/my_api/`).
- **project.md:** тип рендерится как `📚 Codex`; секция «Connected microservices» исключает codex; новая секция «**Connected Codices**» перечисляет подключённые своды построчно по полосам (server / admin_client источники) + инбокс `docs/codex/<name>/`. Admin-полоса идёт как полноценный Codex-канал, симметрично серверной.

## Гейт состава и смена типа

- **Гейт (#11):** Codex-проект держит репо только ролей `server`/`admin_client`, ≤1 каждой. Проверяется при assign (`enforce_codex_repo_gate`), смене роли и смене типа `→ codex`; защитный дубль при синке (репо вне гейта → skip + error).
- **Смена типа (#12):** подключённый к parent'ам проект может переключаться **между** `microservice` и `codex` (оба потребляются parent'ами через ту же `project_microservices`), блокируется только `→ standard`. Ретипизация реального Swan_Practices (Microservice → Codex) — **штатная операция**, без disconnect/reconnect parent'ов.

## Скиллы

REQ- и announce-каналы за Codex сохранены. +2 role-split скилла:

| Роль | Скилл |
|---|---|
| консумер: применяет свод из `docs/codex/`, добавления — REQ'ом | `sdh-codex-consume` |
| издатель: курирует свод (case→lesson→rule, граница CLAUDE.md↔свод, критичный урок анонсом) | `sdh-codex-maintain` |

## Не-цели (v1.11.0)

- Client-полоса (`role=client`) у Codex — механизм обобщается, но не строится сейчас (клиенты самобытнее — вероятно отдельным Codex позже).
- Портфельный broadcast независимо от графа подключений.
- Редактор свода внутри SDH — контрибьюция через REQ/PR, не через UI.
