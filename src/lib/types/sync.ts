export interface SyncResult {
  copied: number;
  responses: number;
  migrated: number;
  errors: string[];
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
}

/** F-033: rename-log entry. */
export interface RepoRename {
  id: number;
  repository_id: number;
  old_canonical: string;
  new_canonical: string;
  renamed_at: string;
}
