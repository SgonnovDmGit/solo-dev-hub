# Releasing

Процесс релиза Solo Dev Hub после 0.15.0 — через GitHub Actions pipeline. Локальная `npm run tauri build` больше не нужна для production-релизов.

## Обычный релиз

1. Убедиться что `master` в чистом состоянии, CI зелёный (если PR-workflow есть).
2. **Bump версии** в 3 файлах:
   - `package.json` (`"version": "..."`)
   - `src-tauri/Cargo.toml` (`version = "..."`)
   - `src-tauri/tauri.conf.json` (`"version": "..."`)
3. **Обновить `Changelog.md`** — переместить накопленное из `## [Unreleased]` в новую секцию `## [X.Y.Z] — YYYY-MM-DD`. Не оставлять пустых заголовков.
4. **Пересобрать `Cargo.lock`**: `cd src-tauri && cargo check` (пересинхронизирует с новой версией пакета).
5. **Коммит**: `git commit -m "release: vX.Y.Z"`.
6. **Тег и пуш**:
   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin master
   git push origin vX.Y.Z
   ```
7. GitHub Actions подхватит тег → собёрет → подпишет → опубликует Release.
8. Проверить на GitHub: Release создан, артефакты на месте (`*-setup.exe`, `*-setup.exe.sig`, `latest.json`), body соответствует Changelog секции.
9. (Опционально) на установленном приложении → About → "Проверить обновления" → убедиться что новая версия доступна → установить через in-app updater.

## Prerelease (rc/beta/alpha)

Тег вида `vX.Y.Z-rc1` / `vX.Y.Z-beta2` / `vX.Y.Z-alpha1` автоматически помечается как prerelease в GitHub. Используем для dogfood pipeline'а перед финальным тегом. Не забудь удалить rc-release и rc-tag после теста (чтобы `latest.json` не указывал на prerelease).

## Что делает workflow

`.github/workflows/release.yml`:
1. Checkout, setup Node 20, install deps, setup Rust, cache.
2. Извлечение release notes из `Changelog.md` скриптом `scripts/extract-changelog.mjs <version>`.
3. `tauri-apps/tauri-action@v0` — компиляция, подпись (`TAURI_SIGNING_PRIVATE_KEY` + password), сборка NSIS installer'а, генерация `latest.json`, публикация Release с артефактами.
4. Endpoint у installed apps настроен на `https://github.com/SgonnovDmGit/solo-dev-hub/releases/latest/download/latest.json` — GitHub редиректит на актуальный релиз автоматически. Репозиторий приватный до v1.0.0 public-flip'а — `latest.json` без auth не отдаётся, autoupdate фактически приостановлен на v0.25.x.

## GitHub Actions secrets (один раз настраиваются)

В Repo Settings → Secrets and variables → Actions:

| Secret | Что содержит |
|--------|--------------|
| `TAURI_SIGNING_PRIVATE_KEY` | Содержимое `.tauri/signing-key.pem` (весь текст, включая `untrusted comment:` и base64 блок) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Пароль, которым зашифрован приватник |

`GITHUB_TOKEN` автоматически подставляется Actions — отдельно не нужен.

## Ключи подписи

### Генерация (уже сделана для 0.15.0 — повторять не нужно)

```bash
mkdir -p .tauri
npm run tauri signer generate -- -w .tauri/signing-key.pem
```

Выхлоп:
- `.tauri/signing-key.pem` — **приватный**, зашифрован паролем. В `.gitignore`. **НИКОГДА не коммитить.**
- `.tauri/signing-key.pem.pub` — **публичный**. Его base64-блок копируется в `tauri.conf.json → plugins.updater.pubkey`.

### Backup policy (критично)

Потеря приватника = installed users застряли на последней подписанной версии. Восстановление невозможно без manual reinstall.

**Обязательно:**
1. Сохранить содержимое `.pem` + пароль в password manager (1Password / KeePass / Bitwarden).
2. Дополнительно скопировать `.pem` на отдельный физический носитель (external SSD / флешка), пароль записать отдельно.

### Ротация ключа (когда понадобится — например, при компрометации или переходе на v2.0.0)

**Если старый приватник жив:**
1. Сгенерировать новую пару ключей.
2. В `tauri.conf.json` поменять `pubkey` на **новый**.
3. В GitHub Secrets — `TAURI_SIGNING_PRIVATE_KEY` поменять на **старый** приватник.
4. Выпустить release `vX.Y.Z` — installed users доверяют старому ключу → примут update (в его `tauri.conf.json` уже новый pubkey).
5. Следующий release — секрет `TAURI_SIGNING_PRIVATE_KEY` меняем на **новый** приватник; цепочка доверия переехала.

**Если старый приватник потерян:**
- Installed users на старой версии застряли. Единственный путь вперёд — ручная переустановка `.exe` (скачать новый installer с GitHub Releases). Документировать в Changelog → Migration notes.

## Если CI fail

1. Открыть Actions tab на GitHub → failed workflow run → логи конкретного step.
2. Частые причины:
   - **Changelog section не найдена** — `extract-changelog.mjs` требует точное совпадение `## [X.Y.Z]`. Убедиться что тег и Changelog-версия совпадают.
   - **Signing error** — проверить что `TAURI_SIGNING_PRIVATE_KEY` содержит полный текст `.pem` (не путь), а `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` корректный.
   - **Rust compile error** — воспроизвести локально `cd src-tauri && cargo build --release`.
   - **npm ci fail** — обновить `package-lock.json` и закоммитить.
3. Удалить failed Release (если был создан) и tag (`git push --delete origin vX.Y.Z` + `git tag -d vX.Y.Z`), исправить проблему, перетегнуть.

## 0.14.0 → 0.15.0 (first time, one-off)

0.14.0 и ранее не содержат updater-plugin. Переход с 0.14.0 на 0.15.0 — **ручной**: скачать `github-repo-manager_0.15.0_x64-setup.exe` с GitHub Releases и установить (исторический installer, имя файла на тот момент). С 0.15.0 дальнейшие обновления через in-app updater.
