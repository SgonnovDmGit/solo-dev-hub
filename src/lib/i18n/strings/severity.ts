export const ru = {
  'severity.critical': 'Критический',
  'severity.major': 'Серьёзный',
  'severity.medium': 'Средний',
  'severity.minor': 'Незначительный',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'severity.critical': 'Critical',
  'severity.major': 'Major',
  'severity.medium': 'Medium',
  'severity.minor': 'Minor',
};
