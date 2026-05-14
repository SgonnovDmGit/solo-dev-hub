export const ru = {
  'priority.low': 'Низкий',
  'priority.medium': 'Средний',
  'priority.high': 'Высокий',
  'priority.critical': 'Критический',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'priority.low': 'Low',
  'priority.medium': 'Medium',
  'priority.high': 'High',
  'priority.critical': 'Critical',
};
