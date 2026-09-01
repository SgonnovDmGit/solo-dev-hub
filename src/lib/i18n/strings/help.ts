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
  'help.nav.pr.workspace': 'Рабочая папка и токен',
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
    'Укажите рабочую папку — где лежат локальные клоны репозиториев. Приложение находит их по имени папки.',
  'help.qs.steps.s3': 'Добавьте проект и подтяните в него репозитории кнопкой «+».',
  'help.qs.map.h': 'Что где находится',
  'help.qs.map.p':
    'Верхняя панель: Обзор портфеля, Хронология событий, Отчёты, Секреты, Настройки, Справка, О программе. Слева — дерево проектов и репозиториев.',
  'help.qs.map.callout':
    'Дальше начните с раздела «Настройка проекта» — там подробно про роли репозиториев, рабочую папку и токен.',
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
  'help.pr.workspace.h': 'Рабочая папка и токен',
  'help.pr.workspace.p':
    'Рабочая папка — корень с локальными клонами репозиториев. PAT — персональный токен GitHub со scope repo.',
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
  'help.group.crossrepo': 'Кросс-репо связь',
  'help.group.deploy': 'Деплой',
  'help.group.llm': 'Контракт с ИИ-агентом',
  'help.nav.cr.what': 'Как это работает',
  'help.nav.cr.req': 'Требования по вертикали',
  'help.nav.cr.peers': 'Каналы равных',
  'help.nav.cr.broadcast': 'Анонсы и своды',
  'help.nav.cr.registry': 'Реестр номеров',
  'help.nav.dp.envs': 'Окружения',
  'help.nav.dp.secrets': 'Роли секретов',
  'help.nav.dp.bundles': 'Наборы секретов',
  'help.nav.dp.report': 'Отчёт по деплоям',
  'help.nav.llm.files': 'Какие файлы читает агент',
  'help.nav.llm.bugs': 'Что агенту можно в багах',
  'help.nav.llm.rules': 'Откуда агент берёт правила',
  'help.nav.llm.optin': 'Контракт добровольный',

  'help.cr.h1': 'Кросс-репо связь',
  'help.cr.lead':
    'Репозитории портфеля обмениваются между собой файлами, а разносит эти файлы приложение. Ни GitHub Issues, ни переписки: всё, что один репозиторий сказал другому, лежит текстом в обоих и доступно ИИ-агенту с обеих сторон.',
  'help.cr.what.h': 'Как это работает',
  'help.cr.what.p':
    'Отправитель пишет файл только у себя. На ближайшей синхронизации приложение копирует его в нужную папку репозитория-получателя — локально, без push и pull. Получатель отвечает тоже у себя, и ответ приезжает обратно тем же способом.',
  'help.cr.what.callout':
    'Правило одно и оно жёсткое: писать можно только в своём репозитории. Всё, что появилось в чужом, положило туда приложение.',
  'help.cr.req.h': 'Требования по вертикали',
  'help.cr.req.p':
    'Требование — это просьба, на которую вторая сторона обязана ответить. Поэтому оно ходит только вниз по иерархии: клиент просит свой сервер, сервер просит свой микросервис. Между равными долга нет, для них отдельный канал.',
  'help.cr.req.pair':
    'Обмен состоит из пары: сообщение от отправителя и квитанция от получателя рядом с ним. Пока квитанции нет, требование видно в экране синхронизации как ожидающее.',
  'help.cr.req.close':
    'Закрывается требование либо кнопкой ✓ в приложении — тогда пара удаляется с обеих сторон, — либо маркером от отправителя, если он подтвердил внедрение сам.',
  'help.cr.peers.h': 'Каналы равных',
  'help.cr.peers.p':
    'Два репозитория одного уровня — клиент с клиентом, сервер с сервером, серверы двух разных продуктов — обмениваются через папку InOutBox: исходящая, которую пишете вы, входящая, которую наполняет приложение, и общая для двусторонних договорённостей.',
  'help.cr.peers.kinds':
    'Три рода обмена: пакт — договорённость, требующая ответа второй стороны; передача — вы отдаёте область ответственности; заметка — наблюдение или вопрос. Поскольку долга здесь нет, «отклонено» — законный ответ на любой из них.',
  'help.cr.peers.explicit':
    'Связи равных заводятся вручную во вкладке «Связи» проекта, ровно как подключается микросервис. Совпадение роли само по себе канала не открывает — иначе в проекте с шестью клиентами все начали бы писать всем.',
  'help.cr.broadcast.h': 'Анонсы и своды',
  'help.cr.broadcast.announce':
    'Анонс — односторонний толчок вниз по иерархии, когда вы хотите что-то сообщить, а ответа не ждёте. Получатель удаляет файл после того, как учёл его у себя; квитанции здесь нет.',
  'help.cr.broadcast.codex':
    'Свод практик рассылает проект-кодекс: его папка практик зеркалится в репозитории потребителей по совпадающим ролям. Входящий свод — только для чтения, правки в нём затираются следующей синхронизацией; предложить изменение можно требованием к кодексу.',
  'help.cr.registry.h': 'Реестр номеров',
  'help.cr.registry.p':
    'В каждом исходящем канале лежит registry.md — список номеров обменов, который только пополняется. Ведёт его агент, приложение туда не пишет и оттуда не удаляет.',
  'help.cr.registry.callout':
    'Зачем: номер обмена живёт в имени файла, поэтому закрытие пары его стирает. Без реестра «максимум плюс один» рано или поздно выдаст номер, который у второй стороны уже был.',

  'help.dp.h1': 'Деплой',
  'help.dp.lead':
    'Приложение генерирует Dockerfile и GitHub Actions workflow из шаблона под язык проекта и держит секреты этих пайплайнов. Один репозиторий может иметь сколько угодно окружений.',
  'help.dp.envs.h': 'Окружения',
  'help.dp.envs.p':
    'Окружение — это один экземпляр деплоя: имя, домен, ветка, Docker tag, имя workflow. Их у репозитория может быть несколько — prod, staging, test или любые ваши. Общие для образа настройки задаются один раз на репозиторий, остальное — на каждое окружение.',
  'help.dp.envs.files':
    'По кнопке генерации приложение показывает, какие файлы будут записаны и чем они отличаются от текущих, и записывает только выбранные.',
  'help.dp.secrets.h': 'Роли секретов',
  'help.dp.secrets.p':
    'У каждого секрета в деплое есть роль, определяющая, куда он попадёт: build — запекается в образ при сборке, deploy — доступен в workflow, runtime — передаётся в docker run.',
  'help.dp.secrets.flags':
    'На каждое окружение секрет можно включить или исключить, а значение — переопределить своим. Значения хранятся зашифрованными; в открытом виде их в базе нет нигде.',
  'help.dp.bundles.h': 'Наборы секретов',
  'help.dp.bundles.p':
    'Набор — это переиспользуемая пачка значений: ключи SSH, доступы к базе, токены реестра пакетов, общие для нескольких серверов. Вводите один раз, применяете к секретам репозитория или конкретного окружения.',
  'help.dp.bundles.crypto':
    'Значения шифруются на диске, а ключ шифрования лежит в хранилище учётных данных операционной системы, рядом с токеном GitHub. Мастер-пароля нет и вводить его не нужно.',
  'help.dp.report.h': 'Отчёт по деплоям',
  'help.dp.report.p':
    'В разделе «Отчёты» есть инвентарь всех окружений портфеля одним экраном — домены, ветки, образы, число секретов, сгруппированные по проектам. Клик по строке ведёт к настройке этого окружения, содержимое выгружается в CSV.',

  'help.llm.h1': 'Контракт с ИИ-агентом',
  'help.llm.lead':
    'Приложение рассчитано на то, что рядом с вами работает ИИ-агент. Оно раскладывает по репозиториям файлы, которые агент читает и правит, и проверяет результат там, где ошибка дорога. Этот раздел — о том, что именно от агента ожидается.',
  'help.llm.files.h': 'Какие файлы читает агент',
  'help.llm.files.p':
    'Всё, чем обменивается с агентом приложение, — обычный markdown внутри самих репозиториев. Отдельной интеграции не нужно: агент видит портфель, потому что видит файлы.',
  'help.llm.files.col1': 'Файл',
  'help.llm.files.col2': 'Что это',
  'help.llm.files.r1': 'Ваши задачи и выполненное. Правит и агент, и вы.',
  'help.llm.files.r2': 'Баги. Источник истины — база приложения, файл её представление.',
  'help.llm.files.r3': 'Карточка проекта: состав, роли, с кем разрешён обмен. Пишет приложение.',
  'help.llm.files.r4': 'Правила работы, синхронизируемые во все репозитории. Пишет приложение.',
  'help.llm.files.r5': 'Инструкции для агента в этом репозитории. Пишете вы, часть блока — приложение.',
  'help.llm.bugs.h': 'Что агенту можно в багах',
  'help.llm.bugs.p':
    'В файле багов агенту разрешены только статус и комментарий, плюс удаление строк, которые вы уже подтвердили. Описание, серьёзность, категория, дата и счётчик попыток — не его: если он их изменит, приложение молча восстановит.',
  'help.llm.bugs.callout':
    'Счётчик попыток ведёт приложение, увеличивая его на каждом переходе в «Тестируется». Поэтому «сколько попыток ушло на этот баг» — честная история, а не то, что агент про себя написал.',
  'help.llm.rules.h': 'Откуда агент берёт правила',
  'help.llm.rules.p':
    'Правила приезжают файлами: в каждый репозиторий кладётся папка со скиллами, а для Claude Code они дополнительно раскладываются в пользовательскую папку скиллов. Оба места — копии одного источника, и приложение перезаписывает их при синхронизации.',
  'help.llm.rules.callout':
    'Поэтому править правила на месте бессмысленно — правка исчезнет. Ваши изменения вносятся через редактор шаблонов в настройках.',
  'help.llm.optin.h': 'Контракт добровольный',
  'help.llm.optin.p':
    'Заставить агента соблюдать эти правила приложение не может — оно раскладывает файлы и проверяет результат там, где это возможно. Агент, который правил не читает, просто не пользуется кросс-репо частью: задачи и баги у него работать будут, обмен между репозиториями — нет.',
  'help.llm.optin.callout':
    'Проверяется машиной ровно то, что можно проверить: защищённые поля багов, счётчик попыток, содержимое синкаемых папок. Остальное держится на том, что агент правила прочитал.',
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
  'help.group.crossrepo': 'Cross-repo links',
  'help.group.deploy': 'Deploy',
  'help.group.llm': 'The AI agent contract',
  'help.nav.cr.what': 'How it works',
  'help.nav.cr.req': 'Requirements down the hierarchy',
  'help.nav.cr.peers': 'Channels between equals',
  'help.nav.cr.broadcast': 'Announcements and svods',
  'help.nav.cr.registry': 'The number registry',
  'help.nav.dp.envs': 'Environments',
  'help.nav.dp.secrets': 'Secret roles',
  'help.nav.dp.bundles': 'Secret bundles',
  'help.nav.dp.report': 'The deploy report',
  'help.nav.llm.files': 'Which files the agent reads',
  'help.nav.llm.bugs': 'What the agent may touch in bugs',
  'help.nav.llm.rules': 'Where the agent gets its rules',
  'help.nav.llm.optin': 'The contract is opt-in',

  'help.cr.h1': 'Cross-repo links',
  'help.cr.lead':
    'Repositories in your portfolio exchange files, and the app is what carries those files. No GitHub Issues, no email threads: everything one repository said to another sits as text in both, readable by an AI agent from either side.',
  'help.cr.what.h': 'How it works',
  'help.cr.what.p':
    'The sender writes a file only in its own repository. On the next sync the app copies it into the right folder of the recipient — locally, with no push or pull. The recipient answers in its own repository too, and the answer travels back the same way.',
  'help.cr.what.callout':
    'There is one rule and it is strict: you only ever write inside your own repository. Anything that appeared in someone else’s was put there by the app.',
  'help.cr.req.h': 'Requirements down the hierarchy',
  'help.cr.req.p':
    'A requirement is a request the other side is obliged to answer, so it only travels downwards: a client asks its server, a server asks its microservice. Between equals nobody owes anybody, and they get a separate channel.',
  'help.cr.req.pair':
    'An exchange is a pair: the sender’s message and the recipient’s receipt beside it. Until the receipt exists, the requirement shows as pending on the sync screen.',
  'help.cr.req.close':
    'A requirement closes either through the ✓ button in the app — which deletes the pair on both sides — or through a marker from the sender, if they confirmed the implementation themselves.',
  'help.cr.peers.h': 'Channels between equals',
  'help.cr.peers.p':
    'Two repositories on the same level — client with client, server with server, servers of two different products — exchange through the InOutBox folder: an outgoing one you write, an incoming one the app fills, and a shared one for two-sided agreements.',
  'help.cr.peers.kinds':
    'Three kinds of exchange: a pact — an agreement that needs the other side’s answer; a handoff — you give away an area of responsibility; a note — an observation or a question. Since nobody owes anybody here, “declined” is a legitimate answer to any of them.',
  'help.cr.peers.explicit':
    'Peer links are created by hand on the project’s Links tab, exactly the way a microservice is connected. A matching role alone opens nothing — otherwise in a project with six clients everyone would start writing to everyone.',
  'help.cr.broadcast.h': 'Announcements and svods',
  'help.cr.broadcast.announce':
    'An announcement is a one-way push down the hierarchy, for when you want to tell someone something and expect no answer. The recipient deletes the file once they have taken it into account; there is no receipt here.',
  'help.cr.broadcast.codex':
    'A svod of practices is broadcast by a codex project: its practices folder is mirrored into consumers’ repositories along matching roles. An incoming svod is read-only — edits in it are overwritten on the next sync; to propose a change, file a requirement to the codex.',
  'help.cr.registry.h': 'The number registry',
  'help.cr.registry.p':
    'Every outgoing channel holds a registry.md — an append-only list of exchange numbers. The agent maintains it; the app neither writes to it nor deletes from it.',
  'help.cr.registry.callout':
    'Why: an exchange number lives in the file name, so closing a pair erases it. Without the registry, "the maximum plus one" would eventually hand out a number the other side has already seen.',

  'help.dp.h1': 'Deploy',
  'help.dp.lead':
    'The app generates a Dockerfile and a GitHub Actions workflow from a template for your project’s language, and holds the secrets those pipelines need. One repository can have any number of environments.',
  'help.dp.envs.h': 'Environments',
  'help.dp.envs.p':
    'An environment is one deploy instance: a name, a domain, a branch, a Docker tag, a workflow name. A repository can have several — prod, staging, test or whatever you call them. Image-wide settings are entered once per repository, everything else per environment.',
  'help.dp.envs.files':
    'When you generate, the app shows which files it is about to write and how they differ from the current ones, and writes only the ones you pick.',
  'help.dp.secrets.h': 'Secret roles',
  'help.dp.secrets.p':
    'Every secret in a deploy carries a role that decides where it lands: build — baked into the image at build time, deploy — available inside the workflow, runtime — passed to docker run.',
  'help.dp.secrets.flags':
    'Per environment a secret can be included or left out, and its value overridden with your own. Values are stored encrypted; there is no plaintext anywhere in the database.',
  'help.dp.bundles.h': 'Secret bundles',
  'help.dp.bundles.p':
    'A bundle is a reusable set of values — SSH keys, database credentials, registry tokens shared across several servers. Enter them once, apply them to a repository’s secrets or to a specific environment.',
  'help.dp.bundles.crypto':
    'Values are encrypted at rest and the encryption key lives in the operating system’s credential store, next to your GitHub token. There is no master password to type.',
  'help.dp.report.h': 'The deploy report',
  'help.dp.report.p':
    'The Reports section carries an inventory of every environment in the portfolio on one screen — domains, branches, images, secret counts, grouped by project. Clicking a row jumps to that environment’s settings, and the contents export to CSV.',

  'help.llm.h1': 'The AI agent contract',
  'help.llm.lead':
    'The app is built on the assumption that an AI agent works alongside you. It lays out files across your repositories for the agent to read and edit, and verifies the result wherever a mistake would be costly. This section is about what is expected from the agent.',
  'help.llm.files.h': 'Which files the agent reads',
  'help.llm.files.p':
    'Everything the app exchanges with the agent is ordinary markdown inside the repositories themselves. No separate integration is needed: the agent sees the portfolio because it sees the files.',
  'help.llm.files.col1': 'File',
  'help.llm.files.col2': 'What it is',
  'help.llm.files.r1': 'Your tasks and what is done. Both you and the agent edit them.',
  'help.llm.files.r2': 'Bugs. The app’s database is the source of truth; the file is its view.',
  'help.llm.files.r3': 'The project card: composition, roles, who you may exchange with. Written by the app.',
  'help.llm.files.r4': 'Working rules, synced into every repository. Written by the app.',
  'help.llm.files.r5': 'Instructions for the agent in this repository. Yours, with one managed block from the app.',
  'help.llm.bugs.h': 'What the agent may touch in bugs',
  'help.llm.bugs.p':
    'In the bugs file the agent may change only the status and the comment, plus delete rows you have already confirmed. Description, severity, category, date and the attempt counter are not its business — if it changes them, the app silently restores them.',
  'help.llm.bugs.callout':
    'The attempt counter is kept by the app, incremented on every transition into “Testing”. That is why “how many attempts this bug took” is an honest record rather than something the agent wrote about itself.',
  'help.llm.rules.h': 'Where the agent gets its rules',
  'help.llm.rules.p':
    'Rules arrive as files: a skills folder is placed into every repository, and for Claude Code they are additionally laid out in the user-level skills folder. Both places are copies of one source, and the app overwrites them on sync.',
  'help.llm.rules.callout':
    'So editing the rules in place is pointless — the edit disappears. Your changes go through the template editor in Settings.',
  'help.llm.optin.h': 'The contract is opt-in',
  'help.llm.optin.p':
    'The app cannot force an agent to follow these rules — it lays out the files and verifies the result where verification is possible. An agent that does not read the rules simply does not use the cross-repo half: tasks and bugs will still work for it, exchanges between repositories will not.',
  'help.llm.optin.callout':
    'What a machine can check, it checks: the protected bug fields, the attempt counter, the contents of the synced folders. The rest rests on the agent having read the rules.',
};
