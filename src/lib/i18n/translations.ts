export type Locale = 'ru' | 'en';

import * as common from './strings/common';
import * as grid from './strings/grid';
import * as app from './strings/app';
import * as ctx from './strings/ctx';
import * as about from './strings/about';
import * as settings from './strings/settings';
import * as sidebar from './strings/sidebar';
import * as repoDetail from './strings/repoDetail';
import * as bugNotes from './strings/bugNotes';
import * as bugItem from './strings/bugItem';
import * as bugs from './strings/bugs';
import * as dialog from './strings/dialog';
import * as toast from './strings/toast';
import * as merge from './strings/merge';
import * as repo from './strings/repo';
import * as changelog from './strings/changelog';
import * as tasks from './strings/tasks';
import * as done from './strings/done';
import * as bug from './strings/bug';
import * as role from './strings/role';
import * as priority from './strings/priority';
import * as severity from './strings/severity';
import * as status from './strings/status';
import * as category from './strings/category';
import * as empty from './strings/empty';
import * as project from './strings/project';
import * as stats from './strings/stats';
import * as dashboard from './strings/dashboard';
import * as timeline from './strings/timeline';
import * as sync from './strings/sync';
import * as appDefaults from './strings/appDefaults';
import * as templateEditor from './strings/templateEditor';
import * as secrets from './strings/secrets';
import * as templates from './strings/templates';
import * as deploy from './strings/deploy';
import * as untrack from './strings/untrack';
import * as bundles from './strings/bundles';
import * as reports from './strings/reports';
import * as secretAudit from './strings/secretAudit';

const ru = {
  ...common.ru,
  ...grid.ru,
  ...app.ru,
  ...ctx.ru,
  ...about.ru,
  ...settings.ru,
  ...sidebar.ru,
  ...repoDetail.ru,
  ...bugNotes.ru,
  ...bugItem.ru,
  ...bugs.ru,
  ...dialog.ru,
  ...toast.ru,
  ...merge.ru,
  ...repo.ru,
  ...changelog.ru,
  ...tasks.ru,
  ...done.ru,
  ...bug.ru,
  ...role.ru,
  ...priority.ru,
  ...severity.ru,
  ...status.ru,
  ...category.ru,
  ...empty.ru,
  ...project.ru,
  ...stats.ru,
  ...dashboard.ru,
  ...timeline.ru,
  ...sync.ru,
  ...appDefaults.ru,
  ...templateEditor.ru,
  ...secrets.ru,
  ...templates.ru,
  ...deploy.ru,
  ...untrack.ru,
  ...bundles.ru,
  ...reports.ru,
  ...secretAudit.ru,
} as const;

const en: Record<keyof typeof ru, string> = {
  ...common.en,
  ...grid.en,
  ...app.en,
  ...ctx.en,
  ...about.en,
  ...settings.en,
  ...sidebar.en,
  ...repoDetail.en,
  ...bugNotes.en,
  ...bugItem.en,
  ...bugs.en,
  ...dialog.en,
  ...toast.en,
  ...merge.en,
  ...repo.en,
  ...changelog.en,
  ...tasks.en,
  ...done.en,
  ...bug.en,
  ...role.en,
  ...priority.en,
  ...severity.en,
  ...status.en,
  ...category.en,
  ...empty.en,
  ...project.en,
  ...stats.en,
  ...dashboard.en,
  ...timeline.en,
  ...sync.en,
  ...appDefaults.en,
  ...templateEditor.en,
  ...secrets.en,
  ...templates.en,
  ...deploy.en,
  ...untrack.en,
  ...bundles.en,
  ...reports.en,
  ...secretAudit.en,
};

export const translations: Record<Locale, Record<keyof typeof ru, string>> = { ru, en };
export type TranslationKey = keyof typeof ru;
