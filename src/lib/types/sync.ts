import type { Repository } from './core';
import type { RulesRefresh } from './templates';

/** T-000198: family of a non-fatal sync issue. Mirrors `SyncIssueCode` in
 *  `models/sync.rs` (serde snake_case; a Rust test checks every wire name is
 *  listed here). Translated by `formatSyncIssue` via `syncIssue.<code>`; the
 *  i18n table fails to compile when a code has no key. */
export const SYNC_ISSUE_CODES = [
  'copy_failed',
  'no_local_path',
  'root_missing',
  'rename_collision',
  'rename_replay_failed',
  'lookup_failed',
  'missing_counterpart',
  'managed_file_failed',
  'peer_channel_failed',
  'version_conflict',
  'rule_conflict',
  'req_auto_close_failed',
  'migration_failed',
  'migration_left_in_place',
  'migration_diverged',
  'autocommit_failed',
  'rule_unknown_skill',
  'rule_held',
  'rule_failed',
  'shared_untrusted',
  'shared_fetch_failed',
  'shared_push_failed',
  'shared_push_rejected',
  'shared_needs_manual_merge',
  'shared_no_own_branch',
  'shared_pending_approval',
  'shared_no_login',
] as const;
export type SyncIssueCode = (typeof SYNC_ISSUE_CODES)[number];

/** T-000198: what a `lookup_failed` issue was reading — its `target` token.
 *  Mirrors `models::sync::lookup` in Rust; translated via `syncIssueLookup.<token>`. */
export const SYNC_ISSUE_LOOKUPS = [
  'renames',
  'repos',
  'project',
  'skills',
  'seeding',
  'peers',
  'codex_source',
  'home_dir',
  'autocommit_repos',
] as const;
export type SyncIssueLookup = (typeof SYNC_ISSUE_LOOKUPS)[number];

/** T-000198: one non-fatal issue of a sync-like pass. Mirrors `SyncIssue` in
 *  `models/sync.rs`; absent `target` / `detail` are omitted keys, not null.
 *  `detail` is raw OS / git / DB text and is never translated. */
export interface SyncIssue {
  code: SyncIssueCode;
  subject: string;
  target?: string;
  detail?: string;
}

export interface SyncResult {
  copied: number;
  responses: number;
  migrated: number;
  errors: SyncIssue[];
  /** F-000039: base REQ filenames whose pair was auto-closed via a sender `.impl.md` drop. */
  auto_closed: string[];
  /** T-000137: display names of repos that received an SDH auto-commit of synced cross-repo files. */
  committed: string[];
  /** T-000205: skills whose text was taken back out of a repo's own flagged copy. */
  rules_taken: string[];
  /** T-000207: `<skill> (<repo>)` — marker beside a copy already equal to the rule; the edit was lost. */
  rules_lost: string[];
}

/** Result of `sync_global_claude_md` (the global-rules button). Mirrors
 *  `SyncGlobalClaudeResult` in `commands/sync.rs`. */
export interface SyncGlobalClaudeResult {
  path: string;
  synced_at: string;
  /** T-000207: the button collects pending hand-backs before rendering `~/.claude/skills/`. */
  rules: RulesRefresh;
}

/** T-000207: result of `init_docs_for_repo` — files written plus the rule refresh report. */
export interface InitDocsOutcome {
  updated: string[];
  rules: RulesRefresh;
}

export interface RequirementInfo {
  filename: string;
  direction: string;
  status: string;
  source_repo: string;
  target_repo: string;
  /** B-000018: true для reverse-lookup строк со стороны ms-проекта (sender — parent server,
   * recipient — текущий ms). UI скрывает ✓-кнопку для них. */
  is_reverse_lookup?: boolean;
  /** F-000039: true когда sender положил рядом `REQ-NNN_slug.impl.md` (пара будет
   * авто-закрыта на следующем sync). */
  has_impl?: boolean;
}

/** T-000173: one codex broadcast edge (svod → consumer repo, per lane) surfaced
 * in the sync screen. snake_case mirrors the Rust `CodexEdgeInfo` (no rename_all). */
export interface CodexEdgeInfo {
  codex_name: string;
  consumer_name: string;
  lane: string;
  source_repo: string;
  target_repo: string;
  inbox: string;
  file_count: number;
  /** "incoming" (this project consumes the codex) | "outgoing" (this project IS the codex). */
  direction: string;
  /** "synced" (inbox already mirrors the source) | "pending" (not yet delivered / stale). */
  status: string;
}

/** F-033: rename-log entry. */
export interface RepoRename {
  id: number;
  repository_id: number;
  old_canonical: string;
  new_canonical: string;
  renamed_at: string;
}

/** F-000041: result of `untrack_files`. `untracked` aggregates successful chunks;
 * `errors` carries one entry per failed chunk (chunk-level granularity, not per-file). */
export interface UntrackReport {
  untracked: number;
  errors: string[];
}

/** F-000041: payload returned by `list_gitignored_tracked`. `repo_state` is one
 * of `"clean" | "mid_merge" | "mid_rebase"` — UI gates the Untrack button on `"clean"`. */
export interface GitignoredListing {
  files: string[];
  repo_state: 'clean' | 'mid_merge' | 'mid_rebase' | string;
  other_staged_count: number;
}

// T-000191: explicit peer link between two repositories — the InOutBox channel
// between equals. Every pair is explicit; nothing is inferred from a shared
// role. Stored normalised (repo_a_id < repo_b_id).
export interface PeerLink {
  id: number;
  repo_a_id: number;
  repo_b_id: number;
  created_at: string;
}

// T-000193: one peer channel as shown in the sync screen — a pair of repos and
// the state of the three folders between them.
export interface PeerChannelInfo {
  self_repo_id: number;
  peer_repo_id: number;
  self_repo: string;
  peer_repo: string;
  peer_project: string;
  role: string;
  folder: string;
  outbox_count: number;
  inbox_count: number;
  pact_count: number;
  status: 'synced' | 'pending' | 'unresolved';
  registry_present: boolean;
}

// ── F-000044 / T-000220: shared (contributor-mode) repos ─────────────────────

/** Stored `shared_status` codes of a shared repo (`SharedStatus::as_str` in Rust). */
export type SharedStatusCode =
  | 'shared_untrusted'
  | 'fetch_failed'
  | 'push_failed'
  | 'push_rejected'
  | 'needs_manual_merge'
  | 'no_own_branch'
  | 'pending_approval'
  | 'shared_no_login';

/** One `.gitignore` line (outside the managed block) that still ignores a shared path. */
export interface IgnoreLine {
  line_no: number;
  text: string;
}

/** What must be settled before shared mode can be switched on (`SharedPreflight`). */
export interface SharedPreflight {
  blocking_lines: IgnoreLine[];
  missing_project: boolean;
  missing_dev_branch: boolean;
}

/** Result of the "merge" button (`SharedMergeResult`). */
export interface SharedMergeResult {
  outcome: 'up_to_date' | 'merged' | 'conflict';
  files: string[];
}

/** Result of switching shared mode off (`SharedDisableResult`). */
export interface SharedDisableResult {
  pushed: boolean;
  note: string | null;
}

/** A helper's request to join a shared project (`.sdh/requests/<login>.json`). */
export interface JoinRequest {
  login: string;
  branch: string;
}

export interface StructureParticipant {
  login: string;
  branch: string;
  band_start: number;
}

/** The shared project structure (`.sdh/project.json`, format 1). */
export interface ProjectStructure {
  format: number;
  project: { name: string; type: string };
  main: { login: string; dev_branch: string };
  repos: { github: string; role: string }[];
  microservices: string[];
  participants: StructureParticipant[];
}
