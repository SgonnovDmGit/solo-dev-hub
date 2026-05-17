// ── v0.17.0 Dashboard types ─────────────────────────────────────────────────

export interface Period {
  start: string;  // YYYY-MM-DD
  end: string;    // YYYY-MM-DD
}

export type PeriodPreset = 'week' | 'month' | 'quarter' | 'custom';

export interface DashboardFilter {
  period: Period;
  compare_period: Period | null;
  project_ids: number[] | null;  // null => all repos
}

export interface KpiCard {
  value: number | null;       // null => render '—'
  prev_value: number | null;  // null => no compare arrow
  critical_count: number | null;  // for active-bugs subtitle only
}

export interface TopHotProject {
  project_id: number;
  name: string;
  critical: number;
  major: number;
  active: number;
  bugs_closed: number;
  tasks_done: number;
}

export interface DailyFlowDay {
  date: string;        // YYYY-MM-DD
  opened: number | null;
  closed: number | null;
  done: number | null;
  is_future: boolean;
}

export interface CategoryEfficiencyRow {
  category: string;
  touched_in_period: number;
  closed_in_period: number;
  attempts_in_period: number;
  resolution_rate: number | null;  // null => touched=0, hide row in UI
}

export interface DashboardData {
  active_bugs: KpiCard;
  closed_in_period: KpiCard;
  tasks_done: KpiCard;
  solve_rate: KpiCard;
  attempts_per_closed: KpiCard;
  top_hot: TopHotProject[];
  bugs_daily: DailyFlowDay[];
  tasks_daily: DailyFlowDay[];
  categories: CategoryEfficiencyRow[];
}

/** v0.20.0: Dashboard activity feed event row. */
export interface ActivityEvent {
  kind: 'bug_event' | 'repo_rename' | 'sync_event' | 'deploy_event' | 'task_event';
  event_type: string;
  ts: string;          // ISO8601
  repo_id: number | null;
  repo_display_name: string | null;
  bug_display_id: string | null;
  task_display_id: string | null;
  old_canonical: string | null;
  new_canonical: string | null;
  sync_type: string | null;
  deploy_action: string | null;
  deploy_env_name: string | null;
  change_count: number | null;
  /**
   * v0.31.0 (T-000103 Task 6): structured JSON payload for sync_events.
   * Currently used by `sync_type='migration'` to surface v25 deploy-config
   * placeholder conflict info. Shape (for migration events):
   *   {"conflicts":[{"key":"GO_VERSION","kept_env":"prod","kept_value":"...","discarded":[{"env":"...","value":"..."}]}]}
   * `null` for non-sync events, and for older sync_events with NULL details.
   */
  details: string | null;
}
