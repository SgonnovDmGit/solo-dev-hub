export const ru = {
  'bugs.showConfirmed': 'Показать закрытые ({0})',
  'bugs.showConfirmedHint': 'Подмешать закрытые баги в список (серым, с ✓).',
  'bugs.confirmedAt': 'закрыт {0}',
  'bugs.migrationToast': 'Импортировано {0} багов, из них {1} в архив',
  'bugs.migrationError': 'Не удалось импортировать баги: {0}',
  'bugs.duplicateIdError': 'В MD найдены дубликаты ID. Проверь docs/bug-reports.md',
  // F-000044 / T-000223: a shared repo refused a new bug number (codes: band_refused:*)
  'bugs.refused.pending': 'Баг не создан: главный ещё не подтвердил вашу заявку на участие. Номера выдаются после подтверждения.',
  'bugs.refused.noLogin': 'Баг не создан: логин GitHub этой установки неизвестен — сохраните токен ещё раз в Настройках.',
  'bugs.refused.untrusted': 'Баг не создан: структура общего проекта не прошла проверку ({0}).',
  'bugs.refused.noStructure': 'Баг не создан: структура общего проекта ещё не получена — запустите синк проекта.',
  'bugs.refused.noRange': 'Баг не создан: в структуре проекта для этой установки нет диапазона номеров.',
  'bugs.refused.full': 'Баг не создан: диапазон номеров багов B-{0}..B-{1} этой установки исчерпан.',
  'bugs.refused.unknown': 'Баг не создан: общий проект отказал ({0}).',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'bugs.showConfirmed': 'Show confirmed ({0})',
  'bugs.showConfirmedHint': 'Mix confirmed bugs into the list (greyed, with ✓).',
  'bugs.confirmedAt': 'closed {0}',
  'bugs.migrationToast': 'Imported {0} bugs, {1} archived',
  'bugs.migrationError': 'Bug import failed: {0}',
  'bugs.duplicateIdError': 'Duplicate bug IDs found in MD. Check docs/bug-reports.md',
  // F-000044 / T-000223: a shared repo refused a new bug number (codes: band_refused:*)
  'bugs.refused.pending': 'Bug not created: the owner has not approved your join request yet. Numbers are issued after approval.',
  'bugs.refused.noLogin': 'Bug not created: the GitHub login of this install is unknown — save the token again in Settings.',
  'bugs.refused.untrusted': 'Bug not created: the shared project structure failed validation ({0}).',
  'bugs.refused.noStructure': 'Bug not created: the shared project structure has not arrived yet — sync the project.',
  'bugs.refused.noRange': 'Bug not created: the project structure has no number range for this install.',
  'bugs.refused.full': 'Bug not created: the bug number range B-{0}..B-{1} of this install is full.',
  'bugs.refused.unknown': 'Bug not created: the shared project refused ({0}).',
};
