// v0.22.0 (T-000054): mirrors models/stats.rs StatsSummary / StatsKpi / CategoryBar / HotRepo

export interface StatsKpi {
  active: number;
  active_critical: number;
  closed_total: number;
  avg_attempts: number;
  median_attempts: number;
  fix_rate: number;       // 0..1
  created_total: number;
}

export interface CategoryBar {
  category: string;
  total: number;
  closed: number;
  percent: number;        // 0..100
}

export interface HotRepo {
  repo_id: number;
  github_name: string | null;
  description: string | null;
  critical: number;
  major: number;
  active: number;
  bugs_closed: number;
  tasks_done: number;
}

export interface StatsSummary {
  kpi: StatsKpi;
  categories: CategoryBar[];
  top_hot_repos: HotRepo[] | null;     // null on repo-scope
  lifetime_since: string | null;        // YYYY-MM-DD
  days_history: number;
  repo_count: number | null;            // null on repo-scope
}
