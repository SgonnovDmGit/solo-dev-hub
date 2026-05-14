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
