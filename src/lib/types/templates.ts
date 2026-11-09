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
  /** T-000213: a fork whose canon moved since the owner last accepted it (a new build changed the bundled text). */
  canon_moved: boolean;
  /** An open hand-back conflict. */
  conflict: boolean;
  /** T-000224: login of the owner whose repo this rule's text is taken from; the rule is read-only here. */
  follows_main: string | null;
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

/** T-000241: one wire proposal as the repo's listing shows it — the owner's
 *  read comes off the local dev tip, the contributor's off its own rows.
 *  Mirrors `RuleProposalView` in `models/rule_proposals.rs`. */
export interface RuleProposalView {
  /** `<rule>.<login>.json`. */
  file_name: string;
  rule: string;
  login: string;
  seq: number;
  /** Wire status: `open` | `withdrawn`. */
  status: string;
  /** The proposed rule text. */
  text: string;
  /** RFC 3339 timestamp of this version. */
  at: string;
  /** Blob sha of THIS version — the CAS token a decision is taken against. */
  rev: string | null;
  /** The local dev tip carries this exact version. */
  on_dev: boolean;
  /** The last-seen origin snapshot carries this exact version. */
  on_origin: boolean;
  /** Contributor only: `pending_apply` | `applied` — the local canon write. */
  local_state: string | null;
  /** Resolution overlay: the receipt matching THIS version, if any. */
  resolved_action: string | null;
  resolved_by: string | null;
  resolved_at: string | null;
  /** Owner only: the in-flight acceptance for this version — a decision
   *  already taken whose receipt still owes dev. */
  acceptance_status: string | null;
  acceptance_pub_state: string | null;
  acceptance_action: string | null;
}

/** T-000241: one immutable owner decision plus its local work pipeline and
 *  publication state. Mirrors `RuleAcceptance` in `models/rule_proposals.rs`. */
export interface RuleAcceptance {
  repo_id: number;
  rule: string;
  proposer_login: string;
  /** Full blob sha of the open proposal version decided on. */
  decided_rev: string;
  /** `accept` | `reject`. */
  action: string;
  receipt_json: string;
  /** The proposal text the receipt was taken against — immutable snapshot. */
  proposal_text: string;
  /** prepared | decided | applied | rendered | conflicted | expired | superseded. */
  status: string;
  /** pending | blocked | published | overtaken. */
  pub_state: string;
  base_canon_hash: string;
  canon_hash: string | null;
  created_at: string;
  updated_at: string;
}
