import { get } from 'svelte/store';
import { locale, type Locale } from '$lib/i18n';
import { translations, type TranslationKey } from '$lib/i18n/translations';
import type { SyncIssue } from '$lib/types';

/**
 * T-000198: one translated line for one sync issue. The template comes from
 * `syncIssue.<code>`; `{subject}` / `{target}` / `{detail}` are substituted.
 * A non-empty target the template does not place is appended in parentheses,
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
  // `lookup_failed` names what was being read by a fixed token; an unknown
  // token (a newer backend) is shown as is.
  if (issue.code === 'lookup_failed' && target) {
    target = table[`syncIssueLookup.${target}`] ?? target;
  }
  // An empty subject or target leaves a stray space behind; tidy the template
  // part only, so the raw detail stays exactly as it came.
  let line = fill(fill(template, '{subject}', issue.subject), '{target}', target)
    .replace(/ {2,}/g, ' ')
    .trim();
  // A target the template does not place still names something (e.g. the two
  // sides of a peer pair), so it is appended in parentheses — never as "()".
  if (!template.includes('{target}') && target) {
    line += ` (${target})`;
  }
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
