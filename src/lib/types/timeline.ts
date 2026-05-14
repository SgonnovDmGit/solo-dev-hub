/** v0.20.0: Filter params for read_timeline. */
export interface TimelineFilter {
  start_date: string;
  end_date: string;
  event_kinds?: string[];
  project_ids?: number[];
  repo_ids?: number[];
  search?: string;
}
