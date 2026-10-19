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
