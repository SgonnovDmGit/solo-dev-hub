import type { TranslationKey } from '$lib/i18n/translations';

export interface HelpNavItem {
  anchor: string;
  labelKey: TranslationKey;
  /**
   * Body i18n keys that make up this section's searchable text. Search matches
   * the group title + the item label + these keys, so a query hits section
   * CONTENT, not just the table of contents.
   */
  searchKeys: TranslationKey[];
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
      { anchor: 'qs-what', labelKey: 'help.nav.qs.what', searchKeys: ['help.qs.what.h', 'help.qs.what.p'] },
      {
        anchor: 'qs-steps',
        labelKey: 'help.nav.qs.steps',
        searchKeys: ['help.qs.steps.h', 'help.qs.steps.s1', 'help.qs.steps.s2', 'help.qs.steps.s3'],
      },
      {
        anchor: 'qs-map',
        labelKey: 'help.nav.qs.map',
        searchKeys: ['help.qs.map.h', 'help.qs.map.p', 'help.qs.map.callout'],
      },
    ],
  },
  {
    icon: '📁',
    titleKey: 'help.group.project',
    items: [
      {
        anchor: 'pr-structure',
        labelKey: 'help.nav.pr.structure',
        searchKeys: ['help.pr.structure.h', 'help.pr.structure.p'],
      },
      {
        anchor: 'pr-roles',
        labelKey: 'help.nav.pr.roles',
        searchKeys: [
          'help.pr.roles.h',
          'help.pr.roles.client',
          'help.pr.roles.server',
          'help.pr.roles.ms',
          'help.pr.roles.tool',
        ],
      },
      {
        anchor: 'pr-workspace',
        labelKey: 'help.nav.pr.workspace',
        searchKeys: ['help.pr.workspace.h', 'help.pr.workspace.p', 'help.pr.workspace.callout'],
      },
    ],
  },
  {
    icon: '🐞',
    titleKey: 'help.group.bugs',
    items: [
      { anchor: 'bg-flow', labelKey: 'help.nav.bg.flow', searchKeys: ['help.bg.flow.h', 'help.bg.flow.p'] },
      {
        anchor: 'bg-edit',
        labelKey: 'help.nav.bg.edit',
        searchKeys: [
          'help.bg.edit.h',
          'help.bg.edit.r1field',
          'help.bg.edit.r1who',
          'help.bg.edit.r2field',
          'help.bg.edit.r2who',
          'help.bg.edit.r3field',
          'help.bg.edit.r3who',
          'help.bg.edit.note',
        ],
      },
      {
        anchor: 'bg-confirm',
        labelKey: 'help.nav.bg.confirm',
        searchKeys: ['help.bg.confirm.h', 'help.bg.confirm.p', 'help.bg.confirm.callout'],
      },
    ],
  },
];
