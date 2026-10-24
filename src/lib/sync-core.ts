// T-000227: the toast-free core of the two sync runs. A shared structure in an
// organization's repo is trusted only if its main is a GitHub administrator of
// the repo; the backend never calls GitHub, so this layer asks (Octokit) and
// reports the answers back. Used by the manual runs (`sync-run.ts`) and by the
// background timer (`stores/autosync.ts`) alike.
import { get } from 'svelte/store';
import { pat } from '$lib/stores/settings';
import { syncProject, probeSharedRepos, setSharedAdminVerified } from '$lib/api/tauri-commands';
import { isRepoAdmin } from '$lib/api/github';
import {
  uniqueAdminChecks,
  shouldRerunAfterAdminChecks,
  splitFullName,
  mergeRerun,
  type AdminOutcome,
} from '$lib/utils/shared-admin';
import type { AdminCheck, SyncResult } from '$lib/types';

/**
 * Ask GitHub about one login and store the answer. Resolves to whether the login
 * is an admin of the repo and whether it became one with this answer; throws when
 * the API call fails (nothing is stored then, the previous record stays as it was).
 */
export async function verifyAdmin(
  token: string,
  repoFullName: string,
  login: string,
): Promise<{ admin: boolean; newlyAdmin: boolean }> {
  const parts = splitFullName(repoFullName);
  if (!parts) throw new Error(`'${repoFullName}' is not an owner/name repo`);
  const admin = await isRepoAdmin(token, parts.owner, parts.repo, login);
  const newlyAdmin = await setSharedAdminVerified(repoFullName, login, admin);
  return { admin, newlyAdmin };
}

/**
 * Verify every check of a run. Without a token nothing is verified. A failing
 * check is skipped on its own: it leaves the previous record, and the others
 * still run.
 */
export async function verifyAdminChecks(checks: AdminCheck[] | null | undefined): Promise<AdminOutcome[]> {
  const todo = uniqueAdminChecks(checks);
  if (todo.length === 0) return [];
  const token = get(pat);
  if (!token) return [];
  const outcomes: AdminOutcome[] = [];
  for (const check of todo) {
    try {
      outcomes.push({ check, newlyAdmin: (await verifyAdmin(token, check.repo, check.login)).newlyAdmin });
    } catch {
      // network / 403 / 404: no record is written
    }
  }
  return outcomes;
}

/**
 * Run a backend sync-like call, verify the administrator checks it asked for and,
 * only if one of them just turned a login into an admin, run the call once more.
 * Never loops: the second result is final.
 */
async function withAdminChecks(run: () => Promise<SyncResult>): Promise<SyncResult> {
  const first = await run();
  const outcomes = await verifyAdminChecks(first.admin_checks);
  if (!shouldRerunAfterAdminChecks(outcomes)) return first;
  return mergeRerun(first, await run());
}

/** Sync one project, verifying administrator checks (at most one re-run). No toasts. */
export function syncProjectCore(projectId: number): Promise<SyncResult> {
  return withAdminChecks(() => syncProject(projectId));
}

/** The probe of role-less repos, with the same verification step. No toasts. */
export function probeSharedCore(): Promise<SyncResult> {
  return withAdminChecks(() => probeSharedRepos());
}
