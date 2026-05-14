export const ru = {
  'merge.title': 'Найдено несколько совпадений',
  'merge.description': 'Репозиторий {0} с GitHub совпадает по имени с несколькими локальными записями. Выбери действие:',
  'merge.chooseExisting': 'Объединить с {0}',
  'merge.createNew': 'Создать новую запись',
  'merge.apply': 'Применить',
  'merge.skip': 'Пропустить',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'merge.title': 'Multiple matches found',
  'merge.description': 'GitHub repo {0} matches multiple local-only records by folder name. Choose:',
  'merge.chooseExisting': 'Merge with {0}',
  'merge.createNew': 'Create a new entry',
  'merge.apply': 'Apply',
  'merge.skip': 'Skip',
};
