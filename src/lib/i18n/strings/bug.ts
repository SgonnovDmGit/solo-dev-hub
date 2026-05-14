export const ru = {
  'bug.pipeHint': 'Символ `|` будет экранирован как `\\|` при сохранении (так устроен формат bug-reports.md).',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'bug.pipeHint': 'Character `|` will be escaped as `\\|` on save (this is how bug-reports.md format works).',
};
