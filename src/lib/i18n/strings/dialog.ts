export const ru = {
  'dialog.cancel': 'Отмена',
  'dialog.confirm': 'ОК',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'dialog.cancel': 'Cancel',
  'dialog.confirm': 'OK',
};
