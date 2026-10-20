import { invoke } from '@tauri-apps/api/core';
import type { SyncResult, RequirementInfo, RepoRename, CodexEdgeInfo, PeerChannelInfo, InitDocsOutcome, SyncGlobalClaudeResult, SharedPreflight, SharedMergeResult, SharedDisableResult, JoinRequest, ProjectStructure } from '$lib/types';


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

/** F-000044: probe the repos that are in no shared project yet for the owner's
 *  shared structure and sync every project they land in (the contributor's
 *  join: project created, role assigned, join request sent). */
export async function probeSharedRepos(): Promise<SyncResult> {
  return invoke<SyncResult>('probe_shared_repos');
}

/** T-000198: the branch the repo's checkout is on now (live, not stored);
 *  null when there is no local git checkout or HEAD is detached / unborn. */
export async function getRepoCurrentBranch(repoId: number): Promise<string | null> {
  return invoke<string | null>('get_repo_current_branch', { repoId });
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

// ── F-000044 / T-000220: shared (contributor-mode) repos ─────────────────────

export async function sharedModePreflight(repoId: number): Promise<SharedPreflight> {
  return invoke<SharedPreflight>('shared_mode_preflight', { repoId });
}

/** Owner: switch shared mode on. Rejects when blocking .gitignore lines remain
 *  and `removeLines` is false; with it, exactly those lines are removed. */
export async function enableSharedMode(repoId: number, removeLines: boolean): Promise<void> {
  return invoke<void>('enable_shared_mode', { repoId, removeLines });
}

export async function disableSharedMode(repoId: number): Promise<SharedDisableResult> {
  return invoke<SharedDisableResult>('disable_shared_mode', { repoId });
}

/** Owner: approve a helper's join request. The next sync commits it. */
export async function approveJoinRequest(repoId: number, login: string): Promise<ProjectStructure> {
  return invoke<ProjectStructure>('approve_join_request', { repoId, login });
}

/** Contributor: trust the owner's changed dev branch again. */
export async function acceptNewMainBranch(repoId: number): Promise<void> {
  return invoke<void>('accept_new_main_branch', { repoId });
}

/** The "merge" button of a diverged shared branch. */
export async function mergeSharedUpstream(repoId: number): Promise<SharedMergeResult> {
  return invoke<SharedMergeResult>('merge_shared_upstream', { repoId });
}

export async function listJoinRequests(repoId: number): Promise<JoinRequest[]> {
  return invoke<JoinRequest[]>('list_join_requests', { repoId });
}

export async function getRepoStructure(repoId: number): Promise<ProjectStructure | null> {
  return invoke<ProjectStructure | null>('get_repo_structure', { repoId });
}
