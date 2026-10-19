// F-000044 / T-000223: contributor-mode state of every repo, for the sidebar
// badge and the RepoDetail header. Refreshed on app start, after every sync
// (manual and automatic) and after the shared-mode actions of RepoDetail.
import { writable, get } from 'svelte/store';
import type { RepoSharedState } from '$lib/types';
import { listRepoSharedStates, getSetting } from '$lib/api/tauri-commands';

/** Shared state by repo id (only repos that take part in shared mode). */
export const sharedStates = writable<Map<number, RepoSharedState>>(new Map());
/** GitHub login of this install; null while unknown. */
export const githubLogin = writable<string | null>(null);

let inFlight: Promise<void> | null = null;

/** Reload the shared states and the login. Quiet on failure: the badge is optional. */
export function refreshShared(): Promise<void> {
  // Coalesce parallel refreshes (a sync of several projects ends in a burst).
  if (inFlight) return inFlight;
  inFlight = (async () => {
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
    } finally {
      inFlight = null;
    }
  })();
  return inFlight;
}

export function sharedStateOf(repoId: number | null | undefined): RepoSharedState | null {
  if (repoId == null) return null;
  return get(sharedStates).get(repoId) ?? null;
}
