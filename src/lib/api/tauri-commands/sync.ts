import { invoke } from '@tauri-apps/api/core';
import type { SyncResult, RequirementInfo, RepoRename } from '$lib/types';


// ── Requirements sync commands ──────────────────────────────────────────────

export interface SyncGlobalClaudeResult {
  path: string;
  synced_at: string;
}

export async function syncGlobalClaudeMd(): Promise<SyncGlobalClaudeResult> {
  return await invoke<SyncGlobalClaudeResult>('sync_global_claude_md');
}

export async function initDocsForRepo(repoId: number): Promise<string[]> {
  return invoke<string[]>('init_docs_for_repo', { repoId });
}

export async function syncProject(projectId: number): Promise<SyncResult> {
  return invoke<SyncResult>('sync_project', { projectId });
}

export async function listProjectRequirements(projectId: number): Promise<RequirementInfo[]> {
  return invoke<RequirementInfo[]>('list_project_requirements', { projectId });
}

export async function confirmRequirement(
  projectId: number,
  filename: string,
  sourceRepoId: number,
  targetRepoId: number,
): Promise<void> {
  return invoke<void>('confirm_requirement', { projectId, filename, sourceRepoId, targetRepoId });
}

// ── Rename log (F-033) ────────────────────────────────────────────────────────

export async function listRepoRenames(): Promise<RepoRename[]> {
  return invoke<RepoRename[]>('list_rename_history');
}

export async function listRenamesForRepo(repoId: number): Promise<RepoRename[]> {
  return invoke<RepoRename[]>('list_renames_for_repo', { repoId });
}
