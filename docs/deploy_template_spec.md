# Спека: шаблон «Flutter Web Client + NPM deploy»

**Статус:** draft
**Дата:** 2026-04-15
**Заказчик:** SwanQu (боевая обкатка — `swan_support_test`, ветка master)

## Роль в проекте

Solo Dev Hub должен уметь выдавать проекту «стартовый комплект» CI/CD: `dockerfile` + `.github/workflows/deploy.yml`, параметризованный под конкретный проект. Артефакт — точная копия рабочего деплоя `swan_support_test` (обкатан, задеплоен, https работает). Второй проект, получивший эти файлы через рендер, должен задеплоиться без ручных правок в сгенерированном коде.

## Что делает шаблон

Три связанных job'а (`build-and-push` → `deploy` → `nginx`):

1. **build-and-push**: собирает Docker-образ Flutter-web приложения (multi-stage: cirruslabs/flutter + nginx:alpine), пушит в GHCR с двумя тегами — `@@IMAGE_TAG@@` и SHA коммита. Перед сборкой генерирует `.env` из GitHub Secrets (`API_BASE_URL`, `APP_API_KEY`) — проект читает его через `flutter_dotenv`.

2. **deploy**: по SSH на VPS — `docker pull`, `docker rm -f`, `docker run` в сеть `goapp01_prod_proxy-network` с двумя compose-label'ами.

3. **nginx**: через API Nginx Proxy Manager (localhost:81 на VPS) идемпотентно управляет proxy-host:
   - Если proxy-host для `@@DOMAIN@@` уже существует, `forward_host` правильный, привязан cert, `ssl_forced=true` → **exit 0** (никаких действий, не плодим LE-серты).
   - Иначе — ищет/создаёт Let's Encrypt сертификат, делает upsert proxy-host с `certificate_id` + `ssl_forced=true` + `http2_support=true`.

## Источник истины

Рабочий файл: `f:\Development\SwanQu\swan_support_test\.github\workflows\deploy.yml` (обкатан). Dockerfile: `f:\Development\SwanQu\swan_support_test\dockerfile`.

Любые сомнения в шаблоне — сверяться с этим файлом.

## Плейсхолдеры

Маркер: `@@VAR@@` (выбран чтобы не конфликтовать с `${{ secrets.X }}` GHA и `${VAR}` bash).

| Плейсхолдер | Тип | Пример | Где используется |
|---|---|---|---|
| `@@WORKFLOW_NAME@@` | string | `SwanQu Support — Deploy` | `name:` workflow |
| `@@IMAGE_TAG@@` | string | `prod` | Docker tag в `ghcr.io/.../X` |
| `@@COMPOSE_SERVICE@@` | string | `swan-support-prod-frontend` | label `com.docker.compose.service` |
| `@@DOMAIN@@` | string (FQDN) | `support.swanqu.tech` | NPM proxy-host |

Всё. Остальное — константы (общие для всех Flutter-web проектов SwanQu) или ссылки на GitHub Secrets.

## Константы (НЕ шаблонизировать)

- Docker-сеть: `goapp01_prod_proxy-network`
- Label: `com.docker.compose.project=goapp01_prod`
- Registry: `ghcr.io`
- NPM URL (внутри VPS): `http://localhost:81`
- Версии action'ов: `actions/checkout@v4`, `docker/login-action@v3`, `docker/build-push-action@v5`, `appleboy/ssh-action@v1.0.3`

## GitHub Secrets (обязательные для проекта, получающего шаблон)

Должны быть заведены **до первого пуша**. Solo Dev Hub в UI формы может показать чеклист.

| Secret | Описание | Примечание |
|---|---|---|
| `API_BASE_URL` | Публичный API backend'а | Кладётся в `.env` приложения |
| `APP_API_KEY` | Ключ аутентификации приложения к API | Кладётся в `.env` приложения |
| `SSH_HOST` | Публичный адрес VPS | Домен или IP |
| `SSH_USER` | Пользователь SSH | |
| `SSH_KEY` | Приватный ключ SSH, **многострочный OpenSSH PEM** | Через `gh secret set SSH_KEY < keyfile` — веб-форма GHA также работает; Solo Dev Hub должен поддерживать ввод многострочного значения |
| `SSH_PORT` | Порт SSH | Обычно `22` |
| `CONTAINER_NAME_PROD` | Имя docker-контейнера на VPS | Используется в `--name` И как NPM `forward_host` — docker DNS резолвит имя внутри `goapp01_prod_proxy-network` |
| `NPM_EMAIL` | Email пользователя NPM с правами Manage Proxy Hosts + Manage Certificates | **НЕ админ** |
| `NPM_PASSWORD` | Пароль этого пользователя | Пользователь должен хотя бы раз войти в NPM UI и принять смену пароля, иначе API даёт 400 |

`GITHUB_TOKEN` — проставляется GHA автоматически, заводить не надо.

## Прекондиции на VPS (разовые, вне шаблона)

1. Установлен `jq` на хосте (`apt install jq`) — нужен для SSH-скрипта в `nginx` job.
2. Установлен и работает **Nginx Proxy Manager**, слушает `localhost:81`.
3. NPM и наш контейнер в одной сети `goapp01_prod_proxy-network`, docker DNS резолвит имя по `--name`.
4. В NPM создан выделенный API-пользователь с правами на Proxy Hosts + Certificates, **не админ**.
5. Порты 80 и 443 открыты наружу (LE challenge + HTTPS).
6. A-запись `@@DOMAIN@@` на публичный IP VPS.

## Функция рендера

Чистая функция, никаких сторонних зависимостей:

```ts
function renderTemplate(tmpl: string, vars: Record<string, string>): string {
  return tmpl.replace(/@@(\w+)@@/g, (_, key) => {
    if (!(key in vars)) throw new Error(`Missing manifest key: ${key}`);
    return vars[key];
  });
}
```

Семантика:
- Отсутствующий ключ в манифесте → **ошибка**, не тихая пустая подстановка.
- Лишний ключ в манифесте → игнорируется (терпимо).
- Повторный вызов с теми же аргументами — идемпотентен.

Место в коде (предложение): `src/lib/template-render.ts` или `src-tauri/src/template_render.rs`. Оба варианта равноценны — выбирайте по тому, где удобнее вызывать из UI.

## Выходные файлы

На выход рендер пишет в репо проекта **ровно два файла**:
- `dockerfile` — идентичен текущему `swan_support_test/dockerfile`, ноль плейсхолдеров, просто копия.
- `.github/workflows/deploy.yml` — заполненный по манифесту.

Перед записью — проверить, что оба файла либо отсутствуют, либо Solo Dev Hub показал пользователю diff и получил подтверждение на перезапись.

## UI в Solo Dev Hub

Новая вкладка «Deploy» на экране проекта (или расширение существующего). Поля:

- **Workflow name** → `WORKFLOW_NAME`
- **Image tag** → `IMAGE_TAG` (можно сделать выпадашку: `test`, `prod`, `latest`, или свободный ввод)
- **Compose service** → `COMPOSE_SERVICE`
- **Domain** → `DOMAIN`

Кнопки:
- «Check secrets» — пройтись по списку обязательных Secrets (GH API `GET /repos/:owner/:repo/actions/secrets`), подсветить отсутствующие.
- «Generate files» — вызвать рендер, показать diff с существующими файлами, по подтверждению записать в рабочую копию репо.

## Верификация в Solo Dev Hub

1. Ренденринг-юнит-тест: `renderTemplate(fixture, {WORKFLOW_NAME:"X", IMAGE_TAG:"prod", COMPOSE_SERVICE:"y", DOMAIN:"z.tech"})` → точный ожидаемый текст.
2. Ошибка на missing key: удалить `DOMAIN` из vars → throws `Missing manifest key: DOMAIN`.
3. Регрессия на эталоне: загрузить шаблон, подставить манифест от `swan_support_test` → diff с текущим `.github/workflows/deploy.yml` в этом проекте **пустой**.
4. E2E на втором проекте: перевести `swan_info_test_app` на этот шаблон. Заменить его текущий workflow (формат из эталона, но чуть другой) на сгенерированный → пройти push → деплой успешен без ручных правок.

## Что НЕ входит

- Автоматическое создание NPM API-пользователя — руками.
- Ротация SSH-ключей — руками.
- TLS-certs management отдельно от proxy-host — вся логика сертификатов уже внутри шаблона.
- Бэкенд/микросервисные шаблоны (Go, Node и т.д.) — отдельная спека.
- Backward-compat с разными версиями NPM API — фиксируем актуальную версию (конец 2025), шаблон переделываем если NPM сломает API.

## Известные ограничения / грабли

- **Первый пуш после настройки** — может упасть на `nginx` job из-за отсутствия LE-серта и ограничений схемы `POST /api/nginx/certificates` в некоторых версиях NPM. Фоллбэк: создать сертификат руками через UI один раз (форма → Let's Encrypt → домен + email), CI на следующем прогоне найдёт по домену и привяжет. UI в Solo Dev Hub может предупредить об этом на странице шаблона.
- **LE rate-limit**: 5 дубль-сертов на домен в неделю. Текущий ранний-выход в `nginx` job защищает, но если придётся много ретрайнуть на отладке — возможен `Too Many Requests`. Использовать LE staging-среду для отладки в таком случае.
- **NPM user без прав** → API возвращает пустой список сертификатов → CI думает что cert'а нет → шлёт POST → дубликат. UI Solo Dev Hub должен подсказать выставить Full Access API-пользователю.
