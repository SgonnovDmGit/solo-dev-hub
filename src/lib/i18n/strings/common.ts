export const ru = {
  'common.cancel': 'Отмена',
  'common.loading': 'Загрузка...',
  'common.selectAll': 'Все',
  'common.clearAll': 'Сбросить',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'common.cancel': 'Cancel',
  'common.loading': 'Loading...',
  'common.selectAll': 'All',
  'common.clearAll': 'Clear',
};
