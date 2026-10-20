export const ru = {
  'hiddenRepos.title': '{0} репо не видны с текущим токеном — убрать из SDH?',
  'hiddenRepos.description': 'Эти репозитории есть в базе SDH, но GitHub не отдаёт их по вашему токену. Удаляется только запись в SDH: файлы на диске и GitHub не затрагиваются.',
  'hiddenRepos.noLocal': 'нет локальной папки',
  'hiddenRepos.nSelected': 'Выбрано: {0}',
  'hiddenRepos.selectAll': 'Выбрать все',
  'hiddenRepos.deselectAll': 'Снять все',
  'hiddenRepos.remove': 'Убрать выбранные',
  'hiddenRepos.notNow': 'Не сейчас',
  'toast.hiddenReposRemoved': 'Убрано из SDH репозиториев: {0}',
  'toast.hiddenReposPartial': 'Убрано репозиториев: {0}, не удалось: {1}',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'hiddenRepos.title': '{0} repos are not visible with the current token — remove from SDH?',
  'hiddenRepos.description': 'These repositories are in the SDH database, but GitHub does not return them for your token. Only the SDH record is removed: files on disk and GitHub are not touched.',
  'hiddenRepos.noLocal': 'no local folder',
  'hiddenRepos.nSelected': '{0} selected',
  'hiddenRepos.selectAll': 'Select all',
  'hiddenRepos.deselectAll': 'Deselect all',
  'hiddenRepos.remove': 'Remove selected',
  'hiddenRepos.notNow': 'Not now',
  'toast.hiddenReposRemoved': '{0} repositories removed from SDH',
  'toast.hiddenReposPartial': '{0} repositories removed, {1} failed',
};
