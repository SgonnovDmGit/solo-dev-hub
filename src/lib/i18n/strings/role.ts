export const ru = {
  'role.server': 'Сервер',
  'role.client': 'Клиент',
  'role.test_client': 'Тест-клиент',
  'role.admin_client': 'Админ-клиент',
  'role.microservice': 'Микросервис',
  'role.landing': 'Лендинг',
  'role.tool': 'Утилита',
  'role.other': 'Другое',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'role.server': 'Server',
  'role.client': 'Client',
  'role.test_client': 'Test Client',
  'role.admin_client': 'Admin Client',
  'role.microservice': 'Microservice',
  'role.landing': 'Landing',
  'role.tool': 'Tool',
  'role.other': 'Other',
};
