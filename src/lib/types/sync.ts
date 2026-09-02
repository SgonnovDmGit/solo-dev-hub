import type { Repository } from './core';

export interface SyncResult {
  copied: number;
  responses: number;
  migrated: number;
  errors: string[];
  /** F-000039: base REQ filenames whose pair was auto-closed via a sender `.impl.md` drop. */
  auto_closed: string[];
  /** T-000137: display names of repos that received an SDH auto-commit of synced cross-repo files. */
  committed: string[];
  /** T-000205: skills whose text was taken back out of a repo's own flagged copy. */
  rules_taken: string[];
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
