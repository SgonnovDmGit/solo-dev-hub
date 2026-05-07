import { t } from '$lib/i18n';

export interface Project {
  id: number;
  name: string;
  description: string | null;
  created_at: string;
  project_type: string; // "standard" | "microservice"
}

export type ProjectType = 'standard' | 'microservice';

export interface Repository {
  id: number;
  project_id: number | null;
  github_name: string | null;
  github_url: string | null;
  role: string | null;
  description: string | null;
  language: string | null;
  last_pushed_at: string | null;
  local_path: string | null;
  added_at: string;
  updated_at: string;
  github_id: number | null;
  deploy_target: string | null;
}

export interface DeployManifest {
  repository_id: number;
  workflow_name: string;
  image_tag: string;
  compose_service: string;
  domain: string;
  deploy_branch: string;
  updated_at: string;
  /** Non-core placeholder values (ENV_FILE_PATH, ENTRY_POINT, GO_VERSION, …). */
  extras: Record<string, string>;
}

export interface RenderedFile {
  path: string;
  content: string;
}

export interface WriteError {
  path: string;
  error: string;
}

export interface WriteResult {
  written: string[];
  errors: WriteError[];
}

export interface BranchInfo {
  name: string;
  isDefault: boolean;
}

export function getDisplayName(repo: { github_name: string | null; description?: string | null }): string {
  if (repo.github_name) {
    const parts = repo.github_name.split('/');
    return parts[parts.length - 1] || repo.github_name;
  }
  return repo.description ?? '—';
}

export interface FileBugNote {
  id: string;
  date: string;
  description: string;
  severity: string;
  category: string;
  status: string;
  fix_attempts: number;
  comment: string | null;
}

export interface ReadBugsResult {
  bugs: FileBugNote[];
  warnings: string[];
}

/// v0.16.0: UI-facing bug view (SQLite SoT). Includes `confirmed_at` which is
/// not in MD-format `FileBugNote`. All date fields are YYYY-MM-DD (date portion
/// of the underlying ISO timestamps).
export interface BugView {
  id: string;             // display_id, e.g. "B-000042"
  date: string;           // YYYY-MM-DD (created)
  description: string;
  severity: string;
  category: string;
  status: string;
  fix_attempts: number;
  comment: string | null;
  confirmed_at: string | null;  // YYYY-MM-DD when status=='confirmed'
}

/// v0.16.0: result of lazy MD→DB bug migration for a repo.
export interface MigrationReport {
  imported: number;
  confirmed_archived: number;
  already: boolean;  // true = already migrated, no-op
}


// v0.22.0 (T-000054): mirrors models.rs StatsSummary / StatsKpi / CategoryBar / HotRepo
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
}

export interface StatsSummary {
  kpi: StatsKpi;
  categories: CategoryBar[];
  top_hot_repos: HotRepo[] | null;     // null on repo-scope
  lifetime_since: string | null;        // YYYY-MM-DD
  days_history: number;
  repo_count: number | null;            // null on repo-scope
}

export interface SyncResult {
  copied: number;
  responses: number;
  migrated: number;
  errors: string[];
}

export interface TemplateFile {
  language_key: string;
  file_name: string;
  content: string;
  is_custom: boolean;
  updated_at: string;
}

export interface TemplateLanguage {
  language_key: string;
  display_name: string;
  file_count: number;
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

/** Must match DB CHECK constraint on `bugs.category` (see migration v18 in src-tauri/src/db.rs) */
export const BUG_CATEGORIES = [
  'ui_ux', 'ux_flow', 'logic', 'auth', 'database',
  'performance', 'security', 'integration', 'other',
] as const;
export type BugCategory = typeof BUG_CATEGORIES[number];

export const CATEGORY_COLORS: Record<BugCategory, string> = {
  ui_ux: '#8b5cf6',
  ux_flow: '#a855f7',
  logic: '#3b82f6',
  auth: '#06b6d4',
  database: '#14b8a6',
  security: '#f43f5e',
  performance: '#f59e0b',
  integration: '#0ea5e9',
  other: '#6b7280',
};

export function getCategoryLabel(category: string): string {
  return t(`category.${category}` as any);
}

export type Role = 'server' | 'client' | 'test_client' | 'admin_client' | 'landing' | 'tool' | 'other';

export function getRoleLabel(role: Role | string): string {
  return t(`role.${role}` as any);
}

export function getPriorityLabel(priority: string): string {
  return t(`priority.${priority}` as any);
}

// F-012: 'microservice' removed from Role union (microservice is now a project type).
// Kept in ROLE_ICONS/i18n for graceful degradation of legacy DB values.
export const ROLE_ICONS: Record<string, string> = {
  server: '\u{1F5A5}',
  client: '\u{1F4F1}',
  test_client: '\u{1F9EA}',
  admin_client: '\u{1F6E1}',
  microservice: '\u{2699}',
  landing: '\u{1F310}',
  tool: '\u{1F527}',
  other: '\u{1F4E6}',
};

export const PRIORITY_COLORS: Record<string, string> = {
  critical: '#ef4444',
  high: '#f97316',
  medium: '#eab308',
  low: '#6b7280',
};

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

// v0.18.0: Multi-environment deploy
export type DeploySecretRole = 'build' | 'deploy' | 'runtime';

export interface DeployEnvironment {
  id: number;
  repository_id: number;
  name: string;
  workflow_name: string;
  image_tag: string;
  compose_service: string;
  domain: string;
  deploy_branch: string;
  sort_order: number;
  extras: Record<string, string>;
  updated_at: string;
}

export interface DeploySecret {
  id: number;
  deploy_env_id: number;
  secret_name: string;
  role: DeploySecretRole | null;
  included: boolean;
  override_enabled: boolean;
  sort_order: number;
}

export interface CreateDeployEnvironmentArgs {
  repository_id: number;
  name: string;
  workflow_name: string;
  image_tag: string;
  compose_service: string;
  domain: string;
  deploy_branch: string;
  extras?: Record<string, string>;
}

export interface UpdateDeployEnvironmentArgs {
  id: number;
  workflow_name: string;
  image_tag: string;
  compose_service: string;
  domain: string;
  deploy_branch: string;
  extras: Record<string, string>;
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
}

/** v0.20.0: DB row for `tasks` table. */
export interface Task {
  id: number;
  repository_id: number;
  task_id: string;
  prefix: 'T' | 'F' | 'D';
  description: string;
  effort: number | null;
  priority: string | null;
  status: string | null;
  version: string | null;
  source: 'todo' | 'done';
  created_at: string;
  updated_at: string;
}

/** v0.20.0: SyncTasks report. */
export interface SyncTasksReport {
  imported: number;
  events_emitted: number;
}

/** v0.20.0: Filter params for read_timeline. */
export interface TimelineFilter {
  start_date: string;
  end_date: string;
  event_kinds?: string[];
  project_ids?: number[];
  repo_ids?: number[];
  search?: string;
}

// F-013 Project graph mirror types (matches src-tauri/src/models.rs)

export type GraphNodeKind = 'repo' | 'project';
export type GraphEdgeKind = 'in_project' | 'cross_project_ms';

export interface GraphNode {
  id: string;
  label: string;
  kind: GraphNodeKind;
  role: string | null;
  repo_id: number | null;
  project_id: number | null;
}

export interface GraphEdge {
  source: string;
  target: string;
  kind: GraphEdgeKind;
}

export interface ProjectGraph {
  center: GraphNode | null;
  ring: GraphNode[];
  edges: GraphEdge[];
}
