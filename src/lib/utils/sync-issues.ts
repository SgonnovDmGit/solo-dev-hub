import { get } from 'svelte/store';
import { locale, type Locale } from '$lib/i18n';
import { translations, type TranslationKey } from '$lib/i18n/translations';
import type { SyncIssue } from '$lib/types';

/**
 * T-000198: one translated line for one sync issue. The template comes from
 * `syncIssue.<code>`; `{subject}` / `{target}` / `{detail}` are substituted,
 * and a detail the template does not place is appended after a colon — it is
 * raw OS / git text and is never translated. A code this build does not know
 * (a newer backend) falls back to English built from the raw fields.
 */
export function formatSyncIssue(issue: SyncIssue, loc: Locale = get(locale)): string {
  const table = translations[loc] as Record<string, string>;
  const template = table[`syncIssue.${issue.code}`];
  if (!template) {
    let line = `${issue.code}: ${issue.subject}`;
    if (issue.target) line += ` → ${issue.target}`;
    if (issue.detail) line += `: ${issue.detail}`;
    return line;
  }
  let target = issue.target ?? '';
  // A missing role reads better as the role's own label.
  if (issue.code === 'missing_counterpart' && target) {
    target = table[`role.${target}` as TranslationKey] ?? target;
  }
  let line = fill(fill(template, '{subject}', issue.subject), '{target}', target);
  if (template.includes('{detail}')) {
    line = fill(line, '{detail}', issue.detail ?? '');
  } else if (issue.detail) {
    line += `: ${issue.detail}`;
  }
  return line;
}

function fill(text: string, placeholder: string, value: string): string {
  return text.split(placeholder).join(value);
}

/** T-000198: the issues as toast text — one translated line per issue. */
export function formatSyncIssues(issues: SyncIssue[], loc: Locale = get(locale)): string {
  return issues.map((i) => formatSyncIssue(i, loc)).join('\n');
}
