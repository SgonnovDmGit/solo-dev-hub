import { invoke } from '@tauri-apps/api/core';
import type { StatsSummary, DashboardFilter, DashboardData, ActivityEvent, ProjectGraph } from '$lib/types';


// ── Stats / Graph summaries (v0.22.0 lifetime-only API; live-computed) ───────

export async function getRepoStatsSummary(repositoryId: number): Promise<StatsSummary> {
  return invoke<StatsSummary>('get_repo_stats_summary', { repositoryId });
}

export async function getProjectStatsSummary(projectId: number): Promise<StatsSummary> {
  return invoke<StatsSummary>('get_project_stats_summary', { projectId });
}

export async function getProjectGraph(projectId: number): Promise<ProjectGraph> {
  return invoke<ProjectGraph>('get_project_graph', { projectId });
}

// ── v0.17.0 Dashboard ──────────────────────────────────────────────────────

export async function readDashboard(filter: DashboardFilter): Promise<DashboardData> {
  return invoke<DashboardData>('read_dashboard', { filter });
}

export async function parseDoneEntriesInPeriod(
  repoId: number,
  start: string,
  end: string,
): Promise<Array<[string, number]>> {
  return invoke<Array<[string, number]>>('parse_done_entries_in_period_cmd', {
    repoId, start, end,
  });
}

// ── v0.19.0: Activity feed ────────────────────────────────────────────────────

export async function readRecentActivity(limit: number): Promise<ActivityEvent[]> {
  return invoke<ActivityEvent[]>('read_recent_activity', { limit });
}
