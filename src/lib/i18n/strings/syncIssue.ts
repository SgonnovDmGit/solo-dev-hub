// T-000198: one line per sync issue code (`SyncIssueCode` in types/sync.ts).
// Placeholders: {subject}, {target}, {detail}. When a template has no
// {detail}, `formatSyncIssue` appends the raw detail after a colon.
export const ru = {
  'syncIssue.copy_failed': 'Не удалось скопировать {subject} → {target}',
  'syncIssue.no_local_path': 'У репозитория {subject} не задан локальный путь — пропущен',
  'syncIssue.root_missing': 'Папка репозитория {subject} не найдена — пропущен',
  'syncIssue.rename_collision': 'Переименование {subject}/ → {target}/: обе папки уже есть в {detail} — объедините вручную',
  'syncIssue.rename_replay_failed': 'Не удалось переименовать папку {subject}/ → {target}/',
  'syncIssue.lookup_failed': 'Внутренняя ошибка чтения данных: {subject}',
  'syncIssue.missing_counterpart': 'В проекте {subject} нет репозитория с ролью «{target}»',
  'syncIssue.managed_file_failed': 'Не удалось обновить {target} в {subject}',
  'syncIssue.peer_channel_failed': 'Канал между равными: сбой на {subject}',
  'syncIssue.version_conflict': '{subject}: разные версии у {target} — разрешите вручную и оставьте одну',
  'syncIssue.req_auto_close_failed': 'Не удалось авто-закрыть REQ-пары с {subject}',
  'syncIssue.migration_failed': 'Миграция не выполнена: {subject}',
  'syncIssue.autocommit_failed': 'Авто-коммит в {subject} не выполнен',
  'syncIssue.rule_unknown_skill': '{subject}: {target} помечен, но такого правила нет — в копии правило не создать',
  'syncIssue.rule_held': '{subject}: копия {target}.md оставлена как есть — возврат правила не разрешён',
  'syncIssue.rule_failed': 'Возврат правила {subject}: сбой на {target}',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'syncIssue.copy_failed': 'Could not copy {subject} → {target}',
  'syncIssue.no_local_path': 'Repo {subject} has no local path — skipped',
  'syncIssue.root_missing': 'Repo {subject} folder not found — skipped',
  'syncIssue.rename_collision': 'Rename {subject}/ → {target}/: both folders exist in {detail} — merge by hand',
  'syncIssue.rename_replay_failed': 'Could not rename folder {subject}/ → {target}/',
  'syncIssue.lookup_failed': 'Internal read failed: {subject}',
  'syncIssue.missing_counterpart': 'Project {subject} has no repo with role "{target}"',
  'syncIssue.managed_file_failed': 'Could not update {target} in {subject}',
  'syncIssue.peer_channel_failed': 'Peer channel failed at {subject}',
  'syncIssue.version_conflict': '{subject}: different versions at {target} — resolve by hand and keep one',
  'syncIssue.req_auto_close_failed': 'Could not auto-close REQ pairs with {subject}',
  'syncIssue.migration_failed': 'Migration not done: {subject}',
  'syncIssue.autocommit_failed': 'Auto-commit in {subject} failed',
  'syncIssue.rule_unknown_skill': '{subject}: {target} is flagged but no such rule exists — a rule cannot be invented in a copy',
  'syncIssue.rule_held': '{subject}: {target}.md copy left as is — unresolved hand-back',
  'syncIssue.rule_failed': 'Rule hand-back {subject}: failed at {target}',
};
