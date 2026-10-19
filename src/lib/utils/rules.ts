import type { TranslationKey } from '$lib/i18n';

/**
 * T-000174: which `_global` template files are rules, and what they are called.
 * Mirrors the Rust `sync::rule_name_of` — the frontend lists rules by the same
 * rule the backend keeps history for.
 */

/** The `_global` template of the global CLAUDE.md section. */
export const GLOBAL_SECTION_FILE = 'claude.md.global.tmpl';

const GLOBAL_SECTION_RULE = 'claude.md.global';
const SKILL_PREFIX = 'skill.';
const SKILL_SUFFIX = '.md.tmpl';

/**
 * The rule a `_global` template file stands for: a skill name for
 * `skill.<name>.md.tmpl`, `claude.md.global` for the global section, `null`
 * for every other file (an empty skill name is not a rule).
 */
export function ruleNameOf(fileName: string): string | null {
  if (fileName === GLOBAL_SECTION_FILE) return GLOBAL_SECTION_RULE;
  if (!fileName.startsWith(SKILL_PREFIX)) return null;
  const rest = fileName.slice(SKILL_PREFIX.length);
  if (!rest.endsWith(SKILL_SUFFIX)) return null;
  const name = rest.slice(0, rest.length - SKILL_SUFFIX.length);
  return name.length > 0 ? name : null;
}

export function isRuleFile(fileName: string): boolean {
  return ruleNameOf(fileName) !== null;
}

// ── Rules screen view helpers (T-000174) ─────────────────────────────────────

/** Last path segment of a repo label (`owner/name` or a local-only path). */
export function shortRepoName(label: string): string {
  const parts = label.split(/[\\/]/).filter((p) => p.length > 0);
  return parts.length > 0 ? parts[parts.length - 1] : label;
}

/** Local `DD.MM.YYYY HH:MM`; the raw string when it is not a date. */
export function formatRuleDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = (n: number) => String(n).padStart(2, '0');
  return `${p(d.getDate())}.${p(d.getMonth() + 1)}.${d.getFullYear()} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** Notes the backend writes into `conflict_resolved` events, by exact value. */
const NOTE_KEYS: Record<string, TranslationKey> = {
  'accepted by the owner': 'rules.note.accepted',
  'rejected by the owner': 'rules.note.rejected',
  "superseded by the owner's edit": 'rules.note.superseded',
  'resolved outside the app': 'rules.note.outside',
};

/** i18n key for a known event note; null for free text (show it raw). */
export function noteKeyOf(note: string): TranslationKey | null {
  return NOTE_KEYS[note] ?? null;
}

export interface RuleStats {
  total: number;
  changed: number;
  conflicts: number;
}

export function ruleStats(rules: { changed: boolean; conflict: boolean }[]): RuleStats {
  return {
    total: rules.length,
    changed: rules.filter((r) => r.changed).length,
    conflicts: rules.filter((r) => r.conflict).length,
  };
}
