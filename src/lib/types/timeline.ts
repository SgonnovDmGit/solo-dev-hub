/** v0.20.0: Filter params for read_timeline. */
export interface TimelineFilter {
  start_date: string;
  end_date: string;
  event_kinds?: string[];
  project_ids?: number[];
  repo_ids?: number[];
  search?: string;
  /** F-000044: only the events of this install (no author). */
  author_is_me?: boolean;
  /** F-000044: only the events of this GitHub login; ignored when `author_is_me`. */
  author?: string;
}
