# Flow: Repository Deletion

**Введено в:** v0.5.0 (B-003)
**Связанные файлы:** [src-tauri/src/db.rs](../../src-tauri/src/db.rs) (`delete_repository`), [src-tauri/src/lib.rs](../../src-tauri/src/lib.rs), [src-tauri/src/sync.rs](../../src-tauri/src/sync.rs) (`remove_git_dir`), [src/lib/api/github.ts](../../src/lib/api/github.ts) (`deleteRepoOnGitHub`), [src/lib/components/RepoDetail.svelte](../../src/lib/components/RepoDetail.svelte)

## Суть

Кнопка "Удалить репозиторий" в RepoDetail meta-row. Три независимые опции в диалоге, каждая с чекбоксом:

1. **Удалить из программы** (всегда checked, disabled — обязательно если кнопка нажата)
2. **Удалить с GitHub** (по желанию, требует PAT scope `delete_repo`)
3. **Очистить `.git` локально** (по желанию, если есть `local_path`)

Плюс поле подтверждения — ввести точное имя репо (`getDisplayName`).

## UI-флоу

1. RepoDetail → кнопка "Удалить репозиторий" (справа в meta-row, компактная)
2. Открывается ConfirmDialog с 3 чекбоксами + полем ввода имени
3. Кнопка Confirm активна только когда введённое имя == displayName
4. Действия выполняются последовательно, каждое независимо — ошибка одного не блокирует другие
5. Toast per action (success или error с деталями)
6. В конце — `loadAllRepos()` + переход на repo-list

## Три действия

### 1. GitHub delete (опционально)

`deleteRepoOnGitHub(token, owner, repo)` через `octokit.repos.delete`.

- **Scope**: нужен `delete_repo` в PAT (не входит в стандартный `repo`)
- **Ошибка 403**: показывается в toast `toast.githubDeleteFailed`, флоу продолжается
- **Ошибка 404**: репо уже удалён с GitHub — ок, молчим или warning
- **PAT отсутствует**: чекбокс disabled

### 2. DB + .git cleanup (всегда)

`delete_repository(id, clear_git_local, local_path)` — Rust команда:

```rust
db.delete_repository(id)?;  // каскадно удаляет bugs, bug_events (через bugs), tasks, task_events (через tasks),
                            // deploy_environments, deploy_secrets/deploy_events (через deploy_environments),
                            // sync_events, repo_renames

if clear_git_local && local_path.is_some() {
    sync::remove_git_dir(&local_path)?;  // rm -rf local_path/.git, only .git
}
```

`remove_git_dir`:
- Если `.git` нет — no-op (Ok)
- Если есть — `fs::remove_dir_all`
- **Остальные файлы в папке не трогаются** — только `.git`

Cascade при удалении записи из `repositories` (по состоянию на v0.24.0):
- `bugs.repository_id` — ON DELETE CASCADE (v0.16.0+, SoT для багов)
- `bug_events.bug_id` — ON DELETE CASCADE (v0.17.0+, удаляется через каскад из `bugs`)
- `tasks.repository_id` — ON DELETE CASCADE (v0.20.0+, SoT для задач)
- `task_events.task_id` — ON DELETE CASCADE (v0.20.0+, через `tasks`)
- `deploy_environments.repository_id` — ON DELETE CASCADE (v0.18.0+, ранее `deploy_manifests`)
- `deploy_secrets.deploy_env_id` — ON DELETE CASCADE (v0.18.0+, через `deploy_environments`)
- `deploy_events.deploy_env_id` — ON DELETE CASCADE (v0.20.0+, через `deploy_environments`)
- `sync_events.repository_id` — ON DELETE CASCADE (v0.20.0+; nullable для portfolio-wide events)
- `repo_renames.repository_id` — ON DELETE CASCADE (F-033)
- `project_microservices`: в F-012 (v0.8.0) эта таблица больше не содержит `repository_id`, каскада нет
- `bug_stats` (VIEW, v0.16.0–v0.23.x): VIEW не имеет ON DELETE CASCADE — он пересчитывался live из `bugs`. Удалён в v23 (T-000058, v0.24.0); упоминается здесь только для исторической справки.

## Что произойдёт с каждой опцией

| Опция | Результат на диске | Результат в БД | Результат на GitHub |
|-------|-------------------|----------------|---------------------|
| Только DB | Файлы репо как были | Запись удалена; bugs / bug_events / tasks / task_events / deploy_environments / deploy_secrets / deploy_events / sync_events / repo_renames — каскад | Репо на месте |
| DB + .git | Файлы репо как были, `.git` удалён | Запись удалена | Репо на месте |
| DB + GitHub | Файлы репо как были | Запись удалена | Репо удалён |
| Все три | Файлы как были, `.git` удалён | Запись удалена | Репо удалён |

**Никогда не удаляется сама папка репо на диске** — только `.git` внутри. Пользователь решает что делать с рабочими файлами.

## PAT scopes — где нужно что

- **Create/Update/Delete secrets**: `repo`
- **Read secrets list**: `repo`
- **List branches**: `repo`
- **Delete repository**: `delete_repo` (отдельный!)

Если пользователь пробует удалить с GitHub без `delete_repo` scope — 403. В диалоге рядом с чекбоксом есть подсказка про scope.

## Guards и безопасность

- **Имя-подтверждение**: кнопка заблокирована пока введённое имя не равно displayName. Защита от случайного клика.
- **ConfirmDialog с `children`**: использует существующий паттерн из ConfirmDialog (виден в v0.4.0 секретах и в v0.5.0 delete).
- **Независимость действий**: если GitHub-запрос упал — .git очистка и DB-удаление всё равно выполнятся. Частичный результат лучше чем блокирующая ошибка.

## Тесты (в db.rs)

- `test_delete_repository_cascades` — удаление репо каскадно сносит `bugs` (и через них `bug_events`)
- `test_remove_git_dir_missing_is_noop` — отсутствующая .git ок
- `test_remove_git_dir_removes_only_git` — .git удалён, README остался

## Частые сценарии

### Сценарий A: "тестовый репо, хочу всё снести"
Checked: DB + GitHub + .git. Typed: имя. → Всё исчезает.

### Сценарий B: "из программы убрать, на GitHub оставить"
Checked: только DB. → Репо на GitHub и на диске останется, в приложении пропадёт. Можно снова подтянуть через Sync repos.

### Сценарий C: "хочу начать заново с чистым git"
Checked: только .git. → Папка остаётся, `.git` удалён. `git init` руками и заново.

## Что НЕ входит

- Удаление папки репо целиком с файлами. Только `.git`.
- Удаление forks/связанных репо на GitHub.
- Undo — после подтверждения возврата нет.
