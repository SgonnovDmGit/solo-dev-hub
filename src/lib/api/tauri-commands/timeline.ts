import { invoke } from '@tauri-apps/api/core';
import type { ActivityEvent, Task, SyncTasksReport, TimelineFilter } from '$lib/types';


// ── v0.20.0: Tasks + Timeline ─────────────────────────────────────────────────

export async function syncTasksForRepo(repoId: number): Promise<SyncTasksReport> {
  return invoke<SyncTasksReport>('sync_tasks_for_repo_cmd', { repoId });
}

export async function readTasksFromDb(repoId: number): Promise<Task[]> {
  return invoke<Task[]>('read_tasks_from_db', { repoId });
}

export async function readDoneFromDb(repoId: number): Promise<Task[]> {
  return invoke<Task[]>('read_done_from_db', { repoId });
}

export async function readTimeline(filter: TimelineFilter, offset: number, limit: number): Promise<ActivityEvent[]> {
  return invoke<ActivityEvent[]>('read_timeline', { filter, offset, limit });
}

export async function recordSecretEvent(repoId: number, action: 'set' | 'delete', secretName: string): Promise<void> {
  return invoke('record_secret_event', { repoId, action, secretName });
}

export async function recordDeploySecretEvent(deployEnvId: number, repoId: number, action: 'env_secret_set' | 'env_secret_delete', secretName: string): Promise<void> {
  return invoke('record_deploy_secret_event', { deployEnvId, repoId, action, secretName });
}
