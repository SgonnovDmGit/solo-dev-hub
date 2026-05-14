export const ru = {
  'appDefaults.title': 'Шаблоны приложения',
  'appDefaults.gitignoreTab': '.gitignore',
  'appDefaults.claudeMdTab': 'CLAUDE.md секция',
  'appDefaults.syncGlobal': 'Синхронизировать в ~/.claude/CLAUDE.md',
  'appDefaults.syncGlobalConfirm': 'Записать секцию Solo Dev Hub в глобальный ~/.claude/CLAUDE.md? Существующий контент вне маркеров не будет затронут.',
  'appDefaults.syncGlobalDone': 'Обновлено: {0}',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'appDefaults.title': 'App Templates',
  'appDefaults.gitignoreTab': '.gitignore',
  'appDefaults.claudeMdTab': 'CLAUDE.md section',
  'appDefaults.syncGlobal': 'Sync to ~/.claude/CLAUDE.md',
  'appDefaults.syncGlobalConfirm': 'Write Solo Dev Hub section to global ~/.claude/CLAUDE.md? Existing content outside markers will not be affected.',
  'appDefaults.syncGlobalDone': 'Updated: {0}',
};
