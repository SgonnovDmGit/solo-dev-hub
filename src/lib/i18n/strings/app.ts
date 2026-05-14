export const ru = {
  'app.title': 'Solo Dev Hub',
  'app.settings': 'Настройки',
  'app.openSettings': 'Открыть настройки',
  'app.minimize': 'Свернуть',
  'app.maximize': 'Развернуть',
  'app.restore': 'В окно',
  'app.close': 'Закрыть',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'app.title': 'Solo Dev Hub',
  'app.settings': 'Settings',
  'app.openSettings': 'Open Settings',
  'app.minimize': 'Minimize',
  'app.maximize': 'Maximize',
  'app.restore': 'Restore',
  'app.close': 'Close',
};
