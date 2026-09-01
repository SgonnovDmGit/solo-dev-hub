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
  {
    icon: '🔗',
    titleKey: 'help.group.crossrepo',
    items: [
      {
        anchor: 'cr-what',
        labelKey: 'help.nav.cr.what',
        searchKeys: ['help.cr.what.h', 'help.cr.what.p', 'help.cr.what.callout'],
      },
      {
        anchor: 'cr-req',
        labelKey: 'help.nav.cr.req',
        searchKeys: ['help.cr.req.h', 'help.cr.req.p', 'help.cr.req.pair', 'help.cr.req.close'],
      },
      {
        anchor: 'cr-peers',
        labelKey: 'help.nav.cr.peers',
        searchKeys: [
          'help.cr.peers.h',
          'help.cr.peers.p',
          'help.cr.peers.kinds',
          'help.cr.peers.explicit',
        ],
      },
      {
        anchor: 'cr-broadcast',
        labelKey: 'help.nav.cr.broadcast',
        searchKeys: [
          'help.cr.broadcast.h',
          'help.cr.broadcast.announce',
          'help.cr.broadcast.codex',
        ],
      },
      {
        anchor: 'cr-registry',
        labelKey: 'help.nav.cr.registry',
        searchKeys: ['help.cr.registry.h', 'help.cr.registry.p', 'help.cr.registry.callout'],
      },
    ],
  },
  {
    icon: '🐳',
    titleKey: 'help.group.deploy',
    items: [
      {
        anchor: 'dp-envs',
        labelKey: 'help.nav.dp.envs',
        searchKeys: ['help.dp.envs.h', 'help.dp.envs.p', 'help.dp.envs.files'],
      },
      {
        anchor: 'dp-secrets',
        labelKey: 'help.nav.dp.secrets',
        searchKeys: ['help.dp.secrets.h', 'help.dp.secrets.p', 'help.dp.secrets.flags'],
      },
      {
        anchor: 'dp-bundles',
        labelKey: 'help.nav.dp.bundles',
        searchKeys: ['help.dp.bundles.h', 'help.dp.bundles.p', 'help.dp.bundles.crypto'],
      },
      {
        anchor: 'dp-report',
        labelKey: 'help.nav.dp.report',
        searchKeys: ['help.dp.report.h', 'help.dp.report.p'],
      },
    ],
  },
  {
    icon: '🤖',
    titleKey: 'help.group.llm',
    items: [
      {
        anchor: 'llm-files',
        labelKey: 'help.nav.llm.files',
        searchKeys: [
          'help.llm.files.h',
          'help.llm.files.p',
          'help.llm.files.r1',
          'help.llm.files.r2',
          'help.llm.files.r3',
          'help.llm.files.r4',
          'help.llm.files.r5',
        ],
      },
      {
        anchor: 'llm-bugs',
        labelKey: 'help.nav.llm.bugs',
        searchKeys: ['help.llm.bugs.h', 'help.llm.bugs.p', 'help.llm.bugs.callout'],
      },
      {
        anchor: 'llm-rules',
        labelKey: 'help.nav.llm.rules',
        searchKeys: ['help.llm.rules.h', 'help.llm.rules.p', 'help.llm.rules.callout'],
      },
      {
        anchor: 'llm-optin',
        labelKey: 'help.nav.llm.optin',
        searchKeys: ['help.llm.optin.h', 'help.llm.optin.p', 'help.llm.optin.callout'],
      },
    ],
  },
];
