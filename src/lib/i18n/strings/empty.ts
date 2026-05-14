export const ru = {
  'empty.noProjects': 'Проектов пока нет',
  'empty.noProjectsHint': 'Нажмите +, чтобы создать проект',
  'empty.noRepos': 'Нет репозиториев',
  'empty.noReposHint': 'Нажмите «Синхр.» на боковой панели, чтобы загрузить репозитории с GitHub.',
  'empty.noMatches': 'Ничего не найдено',
  'empty.noMatchesHint': 'Ни один репозиторий не соответствует запросу. Попробуйте другой запрос.',
  'empty.repoNotFound': 'Репозиторий не найден',
  'empty.repoNotFoundHint': 'Выберите репозиторий на боковой панели или вернитесь к списку.',
  'empty.noBugs': 'Ошибок нет',
  'empty.noBugsHint': 'Нажмите «+ Добавить ошибку», чтобы начать отслеживание.',
} as const;

export const en: Record<keyof typeof ru, string> = {
  'empty.noProjects': 'No projects yet',
  'empty.noProjectsHint': 'Click + to create one',
  'empty.noRepos': 'No repositories',
  'empty.noReposHint': 'Click Sync in the sidebar to fetch repositories from GitHub.',
  'empty.noMatches': 'No matches',
  'empty.noMatchesHint': 'No repositories match your search. Try a different query.',
  'empty.repoNotFound': 'Repository not found',
  'empty.repoNotFoundHint': 'Select a repository from the sidebar or go back to the list.',
  'empty.noBugs': 'No bugs recorded',
  'empty.noBugsHint': 'Click + Add Bug to start tracking issues for this repository.',
};
