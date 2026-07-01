export interface SyncResult {
  copied: number;
  responses: number;
  migrated: number;
  errors: string[];
  /** F-000039: base REQ filenames whose pair was auto-closed via a sender `.impl.md` drop. */
  auto_closed: string[];
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
