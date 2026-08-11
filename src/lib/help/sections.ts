import type { TranslationKey } from '$lib/i18n/translations';

export interface HelpNavItem {
  anchor: string;
  labelKey: TranslationKey;
}

export interface HelpNavGroup {
  icon: string;
  titleKey: TranslationKey;
  items: HelpNavItem[];
}

export const helpNav: HelpNavGroup[] = [
  {
    icon: '🚀',
    titleKey: 'help.group.quickstart',
    items: [
      { anchor: 'qs-what', labelKey: 'help.nav.qs.what' },
      { anchor: 'qs-steps', labelKey: 'help.nav.qs.steps' },
      { anchor: 'qs-map', labelKey: 'help.nav.qs.map' },
    ],
  },
  {
    icon: '📁',
    titleKey: 'help.group.project',
    items: [
      { anchor: 'pr-structure', labelKey: 'help.nav.pr.structure' },
      { anchor: 'pr-roles', labelKey: 'help.nav.pr.roles' },
      { anchor: 'pr-workspace', labelKey: 'help.nav.pr.workspace' },
    ],
  },
  {
    icon: '🐞',
    titleKey: 'help.group.bugs',
    items: [
      { anchor: 'bg-flow', labelKey: 'help.nav.bg.flow' },
      { anchor: 'bg-edit', labelKey: 'help.nav.bg.edit' },
      { anchor: 'bg-confirm', labelKey: 'help.nav.bg.confirm' },
    ],
  },
];
