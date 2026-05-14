export const ru = {
  'changelog.emptyState': 'Файл Changelog.md не найден в корне репозитория',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'changelog.emptyState': 'No Changelog.md in the repo root',
};
