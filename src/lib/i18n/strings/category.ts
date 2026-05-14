export const ru = {
  'category.ui_ux': 'UI/UX',
  'category.backend': 'Бэкенд',
  'category.network': 'Сеть',
  'category.database': 'База данных',
  'category.security': 'Безопасность',
  'category.performance': 'Производительность',
  'category.logic': 'Логика',
  'category.integration': 'Интеграция',
  'category.ux_flow': 'UX-флоу',
  'category.other': 'Другое',
  'category.unknown': 'Не определено',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'category.ui_ux': 'UI/UX',
  'category.backend': 'Backend',
  'category.network': 'Network',
  'category.database': 'Database',
  'category.security': 'Security',
  'category.performance': 'Performance',
  'category.logic': 'Logic',
  'category.integration': 'Integration',
  'category.ux_flow': 'UX Flow',
  'category.other': 'Other',
  'category.unknown': 'Unknown',
};
