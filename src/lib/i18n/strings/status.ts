export const ru = {
  'status.created': 'Создан',
  'status.in-progress': 'В работе',
  'status.testing': 'Тестирование',
  'status.confirmed': 'Подтверждён',
  'status.rejected': 'Отклонён',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'status.created': 'Created',
  'status.in-progress': 'In Progress',
  'status.testing': 'Testing',
  'status.confirmed': 'Confirmed',
  'status.rejected': 'Rejected',
};
