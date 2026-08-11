export const ru = {
  'help.title': 'Справка',
  'help.tooltip': 'Справка и онбординг',
  'help.searchPlaceholder': 'Поиск по справке…',
  'help.searchEmpty': 'Ничего не найдено',
  'help.group.quickstart': 'Быстрый старт',
  'help.group.project': 'Настройка проекта',
  'help.group.bugs': 'Работа с багами',
  'help.nav.qs.what': 'Что это такое',
  'help.nav.qs.steps': 'Три шага',
  'help.nav.qs.map': 'Карта интерфейса',
  'help.nav.pr.structure': 'Проект и репозитории',
  'help.nav.pr.roles': 'Роли репозиториев',
  'help.nav.pr.workspace': 'Workspace root и токен',
  'help.nav.bg.flow': 'Жизненный цикл',
  'help.nav.bg.edit': 'Что правит агент',
  'help.nav.bg.confirm': 'Подтверждение ✓',
  'help.qs.h1': 'Быстрый старт',
  'help.qs.lead':
    'Solo Dev Hub — единый пульт для портфеля проектов на GitHub: задачи, баги, деплой, секреты и кросс-репо связи в одном окне. Эта справка проведёт по первому запуску.',
  'help.qs.what.h': 'Что это такое',
  'help.qs.what.p':
    'Приложение управляет вашими репозиториями как портфелем: проекты слева, детали справа. Баги и задачи живут в docs/ ваших репозиториев и синхронизируются с локальной базой — вы правите их в удобном интерфейсе, а AI-агент видит те же файлы.',
  'help.qs.steps.h': 'Три шага до первого проекта',
  'help.qs.steps.s1':
    'Вставьте GitHub-токен: Настройки → PAT. Хранится в Windows Credential Manager, не в файлах и не в базе.',
  'help.qs.steps.s2':
    'Укажите Workspace root — папку, где лежат локальные клоны репозиториев. Приложение находит их по имени папки.',
  'help.qs.steps.s3': 'Добавьте проект и подтяните в него репозитории кнопкой «+».',
  'help.qs.map.h': 'Что где находится',
  'help.qs.map.p':
    'Верхняя панель: Обзор портфеля, Хронология событий, Отчёты, Секреты, Настройки, Справка, О программе. Слева — дерево проектов и репозиториев.',
  'help.qs.map.callout':
    'Дальше начните с раздела «Настройка проекта» — там подробно про роли репозиториев, workspace root и токен.',
  'help.pr.h1': 'Настройка проекта',
  'help.pr.lead':
    'Проект — это контейнер для репозиториев одного продукта. Разберёмся, как их связать и какие роли бывают.',
  'help.pr.structure.h': 'Проект и репозитории',
  'help.pr.structure.p':
    'Репозиторий принадлежит одному проекту. Добавьте репозиторий в проект и назначьте ему роль — она определяет, какие кросс-репо связи станут доступны.',
  'help.pr.roles.h': 'Роли репозиториев',
  'help.pr.roles.client': 'Клиент — знает только один сервер, к микросервисам напрямую не обращается.',
  'help.pr.roles.server': 'Сервер — оркеструет микросервисы, у него может быть много клиентов.',
  'help.pr.roles.ms': 'Микросервис — это тип проекта; подключается к проектам many-to-many.',
  'help.pr.roles.tool': 'Инструмент / Лендинг — самостоятельные репозитории без кросс-репо контрактов.',
  'help.pr.workspace.h': 'Workspace root и токен',
  'help.pr.workspace.p':
    'Workspace root — корневая папка с локальными клонами. PAT — персональный токен GitHub со scope repo.',
  'help.pr.workspace.callout':
    'Секреты и токен никогда не попадают в код или базу: PAT и ключ шифрования лежат в хранилище учётных данных ОС.',
  'help.bg.h1': 'Работа с багами',
  'help.bg.lead':
    'Баги — ядро приложения: понятный статус-воркфлоу и чёткая граница того, что правит AI-агент, а что только вы.',
  'help.bg.flow.h': 'Жизненный цикл',
  'help.bg.flow.p':
    'Вы создаёте баг («Создан»). Агент берёт его в работу («В работе»), чинит и переводит в «Тестирование» с комментарием. Вы проверяете и подтверждаете («Подтверждён») или отклоняете («Отклонён»).',
  'help.bg.edit.h': 'Что правит агент, а что нет',
  'help.bg.edit.col1': 'Поле',
  'help.bg.edit.col2': 'Кто меняет',
  'help.bg.edit.r1field': 'Статус, комментарий',
  'help.bg.edit.r1who': 'агент и вы',
  'help.bg.edit.r2field': 'Описание, важность, категория',
  'help.bg.edit.r2who': 'только вы (в приложении)',
  'help.bg.edit.r3field': 'Число попыток исправления',
  'help.bg.edit.r3who': 'только приложение (авто при переходе в «Тестирование»)',
  'help.bg.edit.note':
    'Защищённые поля агент не может переписать — при любой его правке они восстанавливаются.',
  'help.bg.confirm.h': 'Подтверждение ✓',
  'help.bg.confirm.p':
    'После проверки нажмите ✓ — строка помечается как «Подтверждён», и агент вычистит её при следующем проходе.',
  'help.bg.confirm.callout':
    'Создавать баги — только через «+ Добавить баг», чтобы id, дата и статистика связались корректно.',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'help.title': 'Help',
  'help.tooltip': 'Help & onboarding',
  'help.searchPlaceholder': 'Search help…',
  'help.searchEmpty': 'No matches',
  'help.group.quickstart': 'Quick start',
  'help.group.project': 'Project setup',
  'help.group.bugs': 'Bug workflow',
  'help.nav.qs.what': 'What it is',
  'help.nav.qs.steps': 'Three steps',
  'help.nav.qs.map': 'Interface map',
  'help.nav.pr.structure': 'Project & repositories',
  'help.nav.pr.roles': 'Repository roles',
  'help.nav.pr.workspace': 'Workspace root & token',
  'help.nav.bg.flow': 'Lifecycle',
  'help.nav.bg.edit': 'What the agent edits',
  'help.nav.bg.confirm': 'Confirmation ✓',
  'help.qs.h1': 'Quick start',
  'help.qs.lead':
    'Solo Dev Hub is a single control panel for your GitHub project portfolio: tasks, bugs, deploys, secrets and cross-repo links in one window. This guide walks you through the first run.',
  'help.qs.what.h': 'What it is',
  'help.qs.what.p':
    'The app manages your repositories as a portfolio: projects on the left, details on the right. Bugs and tasks live in the docs/ folder of your repositories and sync with a local database — you edit them in a convenient UI while the AI agent sees the same files.',
  'help.qs.steps.h': 'Three steps to your first project',
  'help.qs.steps.s1':
    'Paste your GitHub token: Settings → PAT. Stored in Windows Credential Manager, not in files or the database.',
  'help.qs.steps.s2':
    'Set the Workspace root — the folder holding your local repository clones. The app finds them by folder name.',
  'help.qs.steps.s3': 'Add a project and pull repositories into it with the “+” button.',
  'help.qs.map.h': 'Where things are',
  'help.qs.map.p':
    'Top bar: portfolio Overview, event Timeline, Reports, Secrets, Settings, Help, About. On the left — the tree of projects and repositories.',
  'help.qs.map.callout':
    'Next, start with the “Project setup” section — it covers repository roles, the workspace root and the token in detail.',
  'help.pr.h1': 'Project setup',
  'help.pr.lead':
    'A project is a container for one product’s repositories. Let’s see how to link them and what roles exist.',
  'help.pr.structure.h': 'Project & repositories',
  'help.pr.structure.p':
    'A repository belongs to a single project. Add a repository to a project and assign it a role — the role determines which cross-repo links become available.',
  'help.pr.roles.h': 'Repository roles',
  'help.pr.roles.client': 'Client — knows only one server, never talks to microservices directly.',
  'help.pr.roles.server': 'Server — orchestrates microservices, can have many clients.',
  'help.pr.roles.ms': 'Microservice — a project type; connected to projects many-to-many.',
  'help.pr.roles.tool': 'Tool / Landing — standalone repositories with no cross-repo contracts.',
  'help.pr.workspace.h': 'Workspace root & token',
  'help.pr.workspace.p':
    'Workspace root — the root folder with local clones. PAT — a GitHub personal access token with the repo scope.',
  'help.pr.workspace.callout':
    'Secrets and the token never land in code or the database: the PAT and the encryption key live in the OS credential store.',
  'help.bg.h1': 'Bug workflow',
  'help.bg.lead':
    'Bugs are the heart of the app: a clear status workflow and a firm boundary between what the AI agent edits and what only you do.',
  'help.bg.flow.h': 'Lifecycle',
  'help.bg.flow.p':
    'You create a bug (“Created”). The agent takes it (“In Progress”), fixes it and moves it to “Testing” with a comment. You verify and either confirm (“Confirmed”) or reject (“Rejected”).',
  'help.bg.edit.h': 'What the agent edits, and what it doesn’t',
  'help.bg.edit.col1': 'Field',
  'help.bg.edit.col2': 'Who edits',
  'help.bg.edit.r1field': 'Status, comment',
  'help.bg.edit.r1who': 'agent and you',
  'help.bg.edit.r2field': 'Description, severity, category',
  'help.bg.edit.r2who': 'only you (in the app)',
  'help.bg.edit.r3field': 'Fix-attempt count',
  'help.bg.edit.r3who': 'only the app (auto on entering “Testing”)',
  'help.bg.edit.note':
    'The agent cannot overwrite protected fields — they are restored on any edit it makes.',
  'help.bg.confirm.h': 'Confirmation ✓',
  'help.bg.confirm.p':
    'After verifying, press ✓ — the row is marked “Confirmed” and the agent cleans it up on its next pass.',
  'help.bg.confirm.callout':
    'Create bugs only via “+ Add bug”, so the id, date and statistics wire up correctly.',
};
