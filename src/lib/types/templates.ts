import type { SyncIssue } from './sync';

export interface TemplateFile {
  language_key: string;
  file_name: string;
  content: string;
  is_custom: boolean;
  updated_at: string;
}

/** T-000203: saving a `skill.*.md.tmpl` also delivers the rule to every place an
 *  agent reads it from, so the save reports what actually reached the portfolio. */
export interface TemplateSaveOutcome {
  rendered: boolean;
  skills: number;
  repos: number;
  errors: SyncIssue[];
  /** T-000207: hand-backs of other skills taken up before the render. */
  taken: string[];
  /** T-000207: `<skill> (<repo>)` — marker beside a copy already equal to the rule. */
  lost: string[];
  /** T-000207: `<skill> (<repo>)` — pending hand-back of the saved skill, set aside. */
  superseded: string[];
}

/** T-000207: one `refresh_rules` pass — collect hand-backs across the portfolio,
 *  then render. Mirrors `RulesRefresh` in `models/templates.rs`. */
export interface RulesRefresh {
  taken: string[];
  lost: string[];
  superseded: string[];
  errors: SyncIssue[];
  skills: number;
  repos: number;
  /** T-000206: skills whose conflict was already open before this pass —
   *  reported once when it opened, silent on every repeat. */
  quiet_conflicts?: string[];
}

export interface TemplateLanguage {
  language_key: string;
  display_name: string;
  file_count: number;
}
