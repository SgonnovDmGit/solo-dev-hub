export const ru = {
  'reports.title': 'Отчёты',
  'reports.tooltip': 'Отчёты по портфелю',
  'reports.deployCardTitle': 'Деплои',
  'reports.deployCardDesc': 'Инвентарь окружений: домены, ветки, образы, секреты',
  'reports.auditCardTitle': 'Аудит секретов',
  'reports.auditCardDesc': 'Журнал пушей секретов + сверка с GitHub',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'reports.title': 'Reports',
  'reports.tooltip': 'Portfolio reports',
  'reports.deployCardTitle': 'Deployments',
  'reports.deployCardDesc': 'Environment inventory: domains, branches, images, secrets',
  'reports.auditCardTitle': 'Secrets audit',
  'reports.auditCardDesc': 'Secret-push journal + reconcile with GitHub',
};
