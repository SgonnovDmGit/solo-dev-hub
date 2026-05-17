export const ru = {
  'untrackDialog.title': 'Очистка gitignore-файлов',
  'untrackDialog.description': 'Выбери файлы для удаления из git index. Working tree не меняется. После — `git commit` для фиксации.',
  'untrackDialog.selectAll': 'Выбрать все',
  'untrackDialog.deselectAll': 'Снять все',
  'untrackDialog.nSelected': 'Выбрано: {0}',
  'untrackDialog.emptyState': 'В этом репо нет .gitignore-файлов в индексе.',
  'untrackDialog.loading': 'Чтение индекса...',
  'untrackDialog.errorRead': 'Не удалось прочитать индекс: {0}',
  'untrackDialog.midMergeError': 'Невозможно во время merge или rebase. Заверши или прерви текущую операцию.',
  'untrackDialog.otherStagedWarning': '⚠ В индексе ещё {0} других изменений — они останутся staged после очистки.',
  'untrackDialog.confirmAction': 'Очистить ({0})',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'untrackDialog.title': 'Untrack gitignored files',
  'untrackDialog.description': 'Select files to remove from git index. Working tree files remain. Run `git commit` to record after.',
  'untrackDialog.selectAll': 'Select all',
  'untrackDialog.deselectAll': 'Deselect all',
  'untrackDialog.nSelected': '{0} selected',
  'untrackDialog.emptyState': 'No gitignored files are tracked in this repository.',
  'untrackDialog.loading': 'Reading git index...',
  'untrackDialog.errorRead': 'Failed to read git index: {0}',
  'untrackDialog.midMergeError': 'Cannot untrack during merge or rebase. Finish or abort the current operation first.',
  'untrackDialog.otherStagedWarning': '⚠ {0} other staged changes detected — they remain staged after untrack.',
  'untrackDialog.confirmAction': 'Untrack {0} selected',
};
