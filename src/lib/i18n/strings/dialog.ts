export const ru = {
  'dialog.cancel': 'Отмена',
  'dialog.confirm': 'Подтвердить',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'dialog.cancel': 'Cancel',
  'dialog.confirm': 'Confirm',
};
