import { invoke } from '@tauri-apps/api/core';
import type { Repository, UntrackReport, GitignoredListing } from '$lib/types';


// ── Repository commands ───────────────────────────────────────────────────────

export async function createLocalRepository(
  localPath: string,
  displayName: string,
  projectId: number | null = null,
  role: string | null = null,
): Promise<Repository> {
  return invoke<Repository>('create_local_repository', { localPath, displayName, projectId, role });
}

export type UpsertRepoOutcome =
  | { kind: 'inserted'; repo: Repository }
  | { kind: 'merged'; repo: Repository; merged_with_local_id: number; local_path: string }
  | {
      kind: 'ambiguous';
      github_name: string;
      github_url: string | null;
      description: string | null;
      language: string | null;
      last_pushed_at: string | null;
      github_id: number | null;
      candidates: Repository[];
    };

export async function upsertRepository(
  githubName: string,
  githubUrl: string | null,
  description: string | null,
  language: string | null,
  lastPushedAt: string | null,
  githubId: number | null = null,
): Promise<UpsertRepoOutcome> {
  return invoke<UpsertRepoOutcome>('upsert_repository', {
    githubName,
    githubUrl,
    description,
    language,
    lastPushedAt,
    githubId,
  });
}

export async function resolveMergeWithLocal(
  localId: number,
  githubName: string,
  githubUrl: string | null,
  description: string | null,
  language: string | null,
  lastPushedAt: string | null,
  githubId: number | null = null,
): Promise<Repository> {
  return invoke<Repository>('resolve_merge_with_local', {
    localId,
    githubName,
    githubUrl,
    description,
    language,
    lastPushedAt,
    githubId,
  });
}

export async function forceInsertGithubRepo(
  githubName: string,
  githubUrl: string | null,
  description: string | null,
  language: string | null,
  lastPushedAt: string | null,
  githubId: number | null = null,
): Promise<Repository> {
  return invoke<Repository>('force_insert_github_repo', {
    githubName,
    githubUrl,
    description,
    language,
    lastPushedAt,
    githubId,
  });
}

export async function assignRepository(
  id: number,
  projectId?: number | null,
  role?: string | null
): Promise<Repository> {
  return invoke<Repository>('assign_repository', {
    id,
    projectId: projectId ?? null,
    role: role ?? null,
  });
}

export async function listReposByProject(projectId?: number | null): Promise<Repository[]> {
  return invoke<Repository[]>('list_repos_by_project', { projectId: projectId ?? null });
}

export async function listAllRepos(): Promise<Repository[]> {
  return invoke<Repository[]>('list_all_repos');
}

export async function getRepository(id: number): Promise<Repository> {
  return invoke<Repository>('get_repository', { id });
}

export async function getRepositoryByName(githubName: string): Promise<Repository> {
  return invoke<Repository>('get_repository_by_name', { githubName });
}

export async function setRepoLocalPath(id: number, localPath: string | null): Promise<Repository> {
  return invoke<Repository>('set_repo_local_path', { id, localPath });
}

export async function updateRepoDescription(repoId: number, newDescription: string): Promise<Repository> {
  return invoke<Repository>('update_repo_description', { repoId, newDescription });
}

export async function deleteRepository(id: number, clearGitLocal: boolean, localPath: string | null): Promise<void> {
  return invoke<void>('delete_repository', { id, clearGitLocal, localPath });
}

// ── F-000041: untrack gitignored files ──────────────────────────────────────

export async function checkGitAvailableForRepo(repositoryId: number): Promise<boolean> {
  return invoke<boolean>('check_git_available_for_repo', { repositoryId });
}

export async function listGitignoredTracked(repositoryId: number): Promise<GitignoredListing> {
  return invoke<GitignoredListing>('list_gitignored_tracked', { repositoryId });
}

export async function untrackFiles(repositoryId: number, files: string[]): Promise<UntrackReport> {
  return invoke<UntrackReport>('untrack_files', { repositoryId, files });
}

export async function scanWorkspaceForRepos(workspaceRoot: string, githubNames: string[]): Promise<Record<string, string>> {
  return invoke<Record<string, string>>('scan_workspace_for_repos', { workspaceRoot, githubNames });
}

// ── F-025 Manual ordering ─────────────────────────────────────────────────────

export async function reorderProject(id: number, direction: 'up' | 'down'): Promise<void> {
  return invoke<void>('reorder_project', { id, direction });
}

export async function reorderRepo(repoId: number, direction: 'up' | 'down'): Promise<void> {
  return invoke<void>('reorder_repo', { repoId, direction });
}

export async function rebalanceRepoGroup(orderedIds: number[]): Promise<void> {
  return invoke<void>('rebalance_repo_group', { orderedIds });
}

export async function rebalanceProjects(orderedIds: number[]): Promise<void> {
  return invoke<void>('rebalance_projects', { orderedIds });
}

export async function autoSortAll(): Promise<void> {
  return invoke<void>('auto_sort_all');
}
