// F-000044 / T-000223: contributor-mode state of every repo, for the sidebar
// badge and the RepoDetail header. Refreshed on app start, after every sync
// (manual and automatic) and after the shared-mode actions of RepoDetail.
import { writable, get } from 'svelte/store';
import type { RepoSharedState } from '$lib/types';
import { listRepoSharedStates, getSetting } from '$lib/api/tauri-commands';
import { loadAllRepos } from './repos';
import { loadProjects } from './projects';

/** Shared state by repo id (only repos that take part in shared mode). */
export const sharedStates = writable<Map<number, RepoSharedState>>(new Map());
/** GitHub login of this install; null while unknown. */
export const githubLogin = writable<string | null>(null);

async function runRefresh(): Promise<void> {
  try {
    const [states, login] = await Promise.all([
      listRepoSharedStates(),
      getSetting('github_login').catch(() => null),
    ]);
    sharedStates.set(new Map(states.map((s) => [s.repo_id, s])));
    const trimmed = (login ?? '').trim();
    githubLogin.set(trimmed === '' ? null : trimmed);
  } catch {
    /* keep what is shown */
  }
}

let current: Promise<void> | null = null;
let followUp: Promise<void> | null = null;

/**
 * Reload the shared states and the login. Quiet on failure: the badge is
 * optional. A call made while a refresh is running is not answered with that
 * (possibly stale) run: exactly one more run is scheduled after it, and every
 * caller in between gets that one.
 */
export function refreshShared(): Promise<void> {
  if (!current) {
    current = runRefresh().finally(() => {
      current = null;
    });
    return current;
  }
  if (!followUp) {
    followUp = current.then(() => {
      followUp = null;
      return refreshShared();
    });
  }
  return followUp;
}

/**
 * After a sync finished (manual or automatic): the sync may have changed the
 * statuses, linked a contributor's repo to a project or created one, so reload
 * the shared states, the repos and the projects. Quiet on failure.
 */
export async function refreshAfterSync(): Promise<void> {
  await Promise.all([
    refreshShared(),
    loadAllRepos().catch(() => {}),
    loadProjects().catch(() => {}),
  ]);
}

export function sharedStateOf(repoId: number | null | undefined): RepoSharedState | null {
  if (repoId == null) return null;
  return get(sharedStates).get(repoId) ?? null;
}
