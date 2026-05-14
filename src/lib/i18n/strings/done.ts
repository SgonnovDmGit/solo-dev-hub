export const ru = {
  'done.tabTitle': 'Сделано',
  'done.empty': 'Завершённых задач пока нет',
  'done.col.id': 'ID',
  'done.col.description': 'Описание',
  'done.col.date': 'Дата',
  'done.col.version': 'Версия',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'done.tabTitle': 'Done',
  'done.empty': 'No completed tasks yet',
  'done.col.id': 'ID',
  'done.col.description': 'Description',
  'done.col.date': 'Date',
  'done.col.version': 'Version',
};
