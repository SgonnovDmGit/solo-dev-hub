import { writable, get } from 'svelte/store';
import type { Repository } from '$lib/types';
import {
  listAllRepos as tauriListAllRepos,
  upsertRepository,
  assignRepository as tauriAssignRepository,
} from '$lib/api/tauri-commands';
import { fetchAllRepos } from '$lib/api/github';
import { addToast } from './ui';
import { t, tf } from '$lib/i18n';

export const allRepos = writable<Repository[]>([]);
export const isSyncing = writable<boolean>(false);

// Ambiguous merge cases queued from B-007 sync: 2+ local-only records matched by basename.
// Consumed by <MergeChoiceDialog> in +page.svelte.
export type AmbiguousMergeCase = {
  github_name: string;
  github_url: string | null;
  description: string | null;
  language: string | null;
  last_pushed_at: string | null;
  github_id: number | null;
  candidates: Repository[];
};
export const pendingMergeCases = writable<AmbiguousMergeCase[]>([]);

export async function loadAllRepos(): Promise<void> {
  try {
    const data = await tauriListAllRepos();
    allRepos.set(data);
  } catch (err) {
    addToast(tf('toast.failedToLoadRepos', String(err)), 'error');
  }
}

export async function syncFromGitHub(token: string): Promise<void> {
  isSyncing.set(true);
  try {
    const githubRepos = await fetchAllRepos(token);

    let upsertedCount = 0;
    const ambiguousCases: AmbiguousMergeCase[] = [];
    for (const repo of githubRepos) {
      const outcome = await upsertRepository(
        repo.full_name,
        repo.html_url ?? null,
        repo.description ?? null,
        repo.language ?? null,
        repo.pushed_at ?? null,
        repo.id
      );
      upsertedCount++;
      if (outcome.kind === 'merged') {
        addToast(tf('toast.repoMerged', outcome.repo.github_name ?? '', outcome.local_path), 'success');
      } else if (outcome.kind === 'ambiguous') {
        ambiguousCases.push({
          github_name: outcome.github_name,
          github_url: outcome.github_url,
          description: outcome.description,
          language: outcome.language,
          last_pushed_at: outcome.last_pushed_at,
          github_id: outcome.github_id,
          candidates: outcome.candidates,
        });
      }
    }

    const updated = await tauriListAllRepos();
    allRepos.set(updated);

    if (ambiguousCases.length > 0) {
      pendingMergeCases.update((q) => [...q, ...ambiguousCases]);
    }

    addToast(tf('toast.syncedRepos', upsertedCount), 'success');
  } catch (err) {
    const message = String(err);
    if (message.includes('401') || message.includes('Unauthorized')) {
      addToast(t('toast.syncFailed401'), 'error');
    } else if (message.includes('403') || message.includes('Forbidden')) {
      addToast(t('toast.syncFailed403'), 'error');
    } else {
      addToast(tf('toast.syncFailed', String(err)), 'error');
    }
  } finally {
    isSyncing.set(false);
  }
}

export async function assignRepo(
  id: number,
  projectId: number | null,
  role: string | null
): Promise<Repository | null> {
  // VB-005: Only one server allowed per project
  if (role === 'server' && projectId !== null) {
    const currentRepos = get(allRepos);
    const existingServer = currentRepos.find(
      (r) => r.project_id === projectId && r.role === 'server' && r.id !== id
    );
    if (existingServer) {
      addToast(t('toast.serverAlreadyExists'), 'error');
      return null;
    }
  }

  try {
    const updated = await tauriAssignRepository(id, projectId, role);
    allRepos.update((list) => list.map((r) => (r.id === id ? updated : r)));
    return updated;
  } catch (err) {
    addToast(tf('toast.failedToAssignRepo', String(err)), 'error');
    return null;
  }
}
