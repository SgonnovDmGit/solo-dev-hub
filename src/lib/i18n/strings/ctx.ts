export const ru = {
  'ctx.cut': 'Вырезать',
  'ctx.copy': 'Копировать',
  'ctx.paste': 'Вставить',
  'ctx.selectAll': 'Выделить всё',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'ctx.cut': 'Cut',
  'ctx.copy': 'Copy',
  'ctx.paste': 'Paste',
  'ctx.selectAll': 'Select All',
};
