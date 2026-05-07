# Flow: Secrets Management

**Введено в:** v0.4.0 (multi-line support в v0.6.0)
**Связанные файлы:** [src/lib/components/SecretsPanel.svelte](../../src/lib/components/SecretsPanel.svelte), [src/lib/api/secrets-crypto.ts](../../src/lib/api/secrets-crypto.ts), [src/lib/api/secrets-parser.ts](../../src/lib/api/secrets-parser.ts), [src/lib/api/github.ts](../../src/lib/api/github.ts)

## Суть

Управление GitHub Actions secrets через приложение — без ручного входа в настройки репо на GitHub. Работает **только на стороне фронтенда** через Octokit, Rust backend не участвует (не проксирует GitHub API).

## Архитектура

```
UI (SecretsPanel.svelte)
├─ List of existing secrets (names only, value is write-only in GitHub)
│  ├─ Checkbox + name + inline textarea for new value
│  └─ Bulk actions: "Update selected" / "Delete selected"
└─ Textarea for bulk env-format input
   └─ Push → parse → encrypt each → createOrUpdateRepoSecret

Parser (secrets-parser.ts, pure TS)
└─ parseEnvText(text) → { secrets: [...], errors: [...] }

Crypto (secrets-crypto.ts, pure TS)
└─ encryptSecret(publicKey, value) via libsodium sealed_box

GitHub API (github.ts via Octokit)
├─ listRepoSecrets → имена secrets
├─ getRepoPublicKey → для шифрования
├─ createOrUpdateRepoSecret → PUT с зашифрованным значением
└─ deleteRepoSecret
```

## Шифрование

GitHub API требует значения секретов зашифровать **на клиенте** через libsodium sealed box (public-key encryption). GitHub НЕ видит plaintext value.

```
1. octokit.actions.getRepoPublicKey(owner, repo) → { key (base64), key_id }
2. sodium.crypto_box_seal(valueBytes, publicKey) → encrypted bytes
3. base64(encrypted) + key_id → PUT /repos/:owner/:repo/actions/secrets/:name
```

Зависимость: `libsodium-wrappers` (WASM ~180KB).

## Парсер env-формата

`parseEnvText` принимает multi-line текст, возвращает список `{ name, value }` + ошибки парсинга с номером строки.

### Синтаксис

**Single-line**:
```
API_KEY=abc123
DOMAIN=example.com
```

**Multi-line через triple-quoted** (v0.6.0, для SSH-ключей и других PEM-блоков):
```
SSH_KEY="""
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmU...
abcdef...
-----END OPENSSH PRIVATE KEY-----
"""
```

**Inline triple-quoted** (редко нужен, но поддерживается):
```
SHORT_MULTI="""a
b"""
```

**Правила**:
- Имя секрета: `^[A-Z_][A-Z0-9_]*$`
- Префикс `GITHUB_*` запрещён (GitHub не позволяет)
- Комментарии с `#` в начале строки — игнорируются
- Пустое значение → ошибка
- Незакрытый triple-quote → ошибка

## Multi-line в existing-secrets list

Ранее (v0.4.0) в списке существующих секретов поле ввода было `<input type="password">` — не принимает `\n`. В v0.6.0 заменено на `<textarea rows="1">` с CSS-маскировкой через `-webkit-text-security: disc` (работает в Chromium/WebView2).

Auto-resize при фокусе (становится высотой 80px). При потере фокуса сворачивается обратно в одну строку — но значение хранится целиком с переводами строк.

## UI режимы

### Repo mode (в RepoDetail)

- Показывает список текущих секретов с GitHub + поле для нового значения у каждого
- Массовое "Update selected" / "Delete selected"
- Отдельный textarea для bulk-push новых секретов в env-формате
- Упрощённый паттерн: большинство случаев — не массовая загрузка

### Project mode (в ProjectDetail)

- Тот же textarea для env-текста
- Не показывает existing — у разных репо проекта могут быть разные секреты
- При Push → диалог с чекбоксами: выбрать репо проекта, куда заливать
- Удобно когда проект = несколько клиентов с общими секретами

## PAT scope

Для работы с Actions secrets нужен PAT с:
- Classic: `repo` scope
- Fine-grained: `Secrets` (read + write) + `Actions` (если требуется)

Ошибки 403/401 → toast `secrets.permissionError` с подсказкой.

## Интеграция с Deploy

В DeployScreen кнопка "Check secrets" вызывает `listRepoSecrets` и сравнивает с `required_secrets` из meta.json шаблона. Показывает ✓/✗ для каждого.

Multiline flag из meta.json используется как подсказка пользователю: "этот секрет надо вводить через triple-quoted" (но сейчас подсказка рядом с галочкой — можно расширить до выделения в UI).

## Безопасность

- **PAT хранится в Windows Credential Manager** (через Rust keyring crate), не в SQLite и не в plaintext файлах
- **Значения секретов не хранятся локально** — вводятся пользователем, шифруются и сразу идут на GitHub. SecretsPanel не персистентен.
- **Шифрование происходит в браузерном контексте** (WebView2), plaintext value никогда не покидает локальный процесс в открытом виде

## Частые ошибки

- **Забыть triple-quotes для SSH**: parser вернёт ошибку "missing '=' separator" на первой строке PEM без `=`. Обернуть в `"""..."""`.
- **403 Forbidden**: PAT без `repo` scope. Поменять токен в Settings.
- **GITHUB_X имя**: rejected парсером. Использовать другое имя.

## Что НЕ входит

- Dependabot secrets, Codespaces secrets — только Actions.
- Переносы секретов между репо (копипаста вручную).
- Автоматический rotation SSH-ключей.
