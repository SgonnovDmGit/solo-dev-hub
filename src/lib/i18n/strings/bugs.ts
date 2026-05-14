export const ru = {
  'bugs.showConfirmed': 'Показать закрытые ({0})',
  'bugs.showConfirmedHint': 'Подмешать закрытые баги в список (серым, с ✓).',
  'bugs.confirmedAt': 'закрыт {0}',
  'bugs.migrationToast': 'Импортировано {0} багов, из них {1} в архив',
  'bugs.migrationError': 'Не удалось импортировать баги: {0}',
  'bugs.duplicateIdError': 'В MD найдены дубликаты ID. Проверь docs/bug-reports.md',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'bugs.showConfirmed': 'Show confirmed ({0})',
  'bugs.showConfirmedHint': 'Mix confirmed bugs into the list (greyed, with ✓).',
  'bugs.confirmedAt': 'closed {0}',
  'bugs.migrationToast': 'Imported {0} bugs, {1} archived',
  'bugs.migrationError': 'Bug import failed: {0}',
  'bugs.duplicateIdError': 'Duplicate bug IDs found in MD. Check docs/bug-reports.md',
};
