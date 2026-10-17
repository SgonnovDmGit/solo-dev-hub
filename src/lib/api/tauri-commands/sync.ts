import { invoke } from '@tauri-apps/api/core';
import type { SyncResult, RequirementInfo, RepoRename, CodexEdgeInfo, PeerChannelInfo, InitDocsOutcome, SyncGlobalClaudeResult } from '$lib/types';


// ── Requirements sync commands ──────────────────────────────────────────────

export async function syncGlobalClaudeMd(): Promise<SyncGlobalClaudeResult> {
  return await invoke<SyncGlobalClaudeResult>('sync_global_claude_md');
}

export async function initDocsForRepo(repoId: number): Promise<InitDocsOutcome> {
  return invoke<InitDocsOutcome>('init_docs_for_repo', { repoId });
}

export async function syncProject(projectId: number): Promise<SyncResult> {
  return invoke<SyncResult>('sync_project', { projectId });
}

export async function listProjectRequirements(projectId: number): Promise<RequirementInfo[]> {
  return invoke<RequirementInfo[]>('list_project_requirements', { projectId });
}

// T-000173: codex broadcast edges (svod → consumer repo, per lane) for the sync screen.
export async function listCodexEdges(projectId: number): Promise<CodexEdgeInfo[]> {
  return invoke<CodexEdgeInfo[]>('list_codex_edges', { projectId });
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

// ── T-000137: per-repo auto-commit branch selector ─────────────────────────────

export async function getAutocommitBranch(repoId: number): Promise<string | null> {
  return invoke<string | null>('get_autocommit_branch', { repoId });
}

export async function setAutocommitBranch(repoId: number, branch: string | null): Promise<void> {
  return invoke<void>('set_autocommit_branch', { repoId, branch });
}

// T-000193: peer channels (InOutBox) touching a project.
export async function listPeerChannels(projectId: number): Promise<PeerChannelInfo[]> {
  return invoke<PeerChannelInfo[]>('list_peer_channels', { projectId });
}
