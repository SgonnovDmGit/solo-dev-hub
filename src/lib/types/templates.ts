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
  quiet_conflicts: string[];
}

/** T-000174: one entry of a rule's history. Mirrors `RuleEvent` in `models/templates.rs`.
 *  `kind`: taken | saved | reset | superseded | lost | conflict | conflict_resolved. */
export interface RuleEvent {
  id: number;
  rule: string;
  kind: string;
  /** Repo copy involved; null for an edit made in the app. */
  repo: string | null;
  note: string | null;
  created_at: string;
}

/** T-000174: one workflow rule (a skill, or `claude.md.global`) as listed on the Rules screen. */
export interface RuleInfo {
  rule: string;
  file_name: string;
  content: string;
  /** The text bundled with this build; null when the bundle has no such file. */
  canon: string | null;
  /** Text differs from `canon` (not `is_custom`). */
  changed: boolean;
  /** An open hand-back conflict. */
  conflict: boolean;
  /** Where the current text came from; null when it predates the history. */
  origin: RuleEvent | null;
}

export interface RulesOverview {
  /** Global CLAUDE.md section first, then skills alphabetically. */
  rules: RuleInfo[];
  /** Repos a rule is delivered to: local checkout present on this machine. */
  repos: number;
}

/** T-000206: one side of a hand-back conflict, read live from a repo's copy. */
export interface RuleConflictSide {
  /** Repo label — also the key to accept this side by. */
  repo: string;
  /** Marker mtime, RFC 3339; null when the filesystem would not say. */
  marked_at: string | null;
  note: string | null;
  content: string;
}

export interface TemplateLanguage {
  language_key: string;
  display_name: string;
  file_count: number;
}
