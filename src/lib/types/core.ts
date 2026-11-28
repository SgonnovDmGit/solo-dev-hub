import { t } from '$lib/i18n';

export interface Project {
  id: number;
  name: string;
  description: string | null;
  created_at: string;
  project_type: string; // "standard" | "microservice" | "codex"
  auto_sync_enabled: boolean; // T-000136: per-project background auto-sync opt-in
}

// T-000168: 'codex' — a publishing project type whose product is a practices
// svod broadcast read-only to connected consumers (see docs/flows/codex-broadcast.md).
export type ProjectType = 'standard' | 'microservice' | 'codex';

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

/** T-000216: contributor-mode state of one repository (F-000044). */
export interface RepoSharedState {
  repo_id: number;
  shared_mode: boolean;
  /** Derived role of this install; null when the repo is not shared. */
  shared_role: 'main' | 'contributor' | 'pending' | null;
  /** Status code, e.g. 'needs_manual_merge'. */
  shared_status: string | null;
  /**
   * What the status code is about (T-000223). `needs_manual_merge`: JSON
   * `{"branch":"dev","current":true}`; `push_rejected`: JSON
   * `{"branch":"dev","message":"..."}`; `remote_file_stale` (T-000260): JSON
   * `{"branch":"dev","paths":["docs/todo.md"]}`; the others: a plain reason
   * text.
   */
  shared_status_detail: string | null;
  /**
   * T-000243: whether `docs/superpowers/` (specs, plans, mockups) travels in
   * shared mode. Default true; settable before shared mode is enabled, so it
   * is independent of `shared_mode`.
   */
  share_specs: boolean;
  /** T-000264: who decides rule proposals — 'auto' (the post-pass machine
   *  accepts conflict-free ones) or 'manual' (every one waits for the
   *  owner's click). Column default is 'auto'; the string is raw because
   *  the CHECK constraint is the DB's business. */
  rule_proposals_mode: string;
}

/** T-000264: the header chip's numbers — all derived from live state. */
export interface RuleProposalsSummary {
  /** Open proposals waiting for the owner's click (manual repos: all;
   *  auto repos: the machine-held ones). */
  open_manual: number;
  /** Held-by-conflict proposals or a blocked receipt publication exist. */
  attention: boolean;
  /** Machine decisions of the last 72 hours. */
  auto_decided_recent: number;
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
  codex: '\u{1F4DA}', // 📚 — codex project node in the graph
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
