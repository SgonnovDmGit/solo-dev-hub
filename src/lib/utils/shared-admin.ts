// T-000227: pure helpers of the administrator verification that backs the trust
// of a shared structure in an organization's repo. No Svelte, no Tauri, no
// network -- everything here is unit-tested. The network part is in
// `$lib/sync-core.ts`.
import type { AdminCheck, SyncResult } from '$lib/types';

/** Outcome of one verification the backend recorded. */
export interface AdminOutcome {
  check: AdminCheck;
  /** true when the login became an admin of the repo with this verification. */
  newlyAdmin: boolean;
}

/** The checks without repeats (repo and login compared case-insensitively). */
export function uniqueAdminChecks(checks: AdminCheck[] | null | undefined): AdminCheck[] {
  const seen = new Set<string>();
  const out: AdminCheck[] = [];
  for (const c of checks ?? []) {
    const key = `${c.repo.toLowerCase()}|${c.login.toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(c);
  }
  return out;
}

/**
 * Re-run the sync (or the probe) only when a verification just made somebody an
 * admin who was not one before: that is the only answer that can change what the
 * run would trust. A revocation, a repeated yes and a failed check change
 * nothing the first run did not already do. The caller re-runs at most once.
 */
export function shouldRerunAfterAdminChecks(outcomes: AdminOutcome[]): boolean {
  return outcomes.some((o) => o.newlyAdmin);
}

/** `owner/name` -> parts; null for anything else. */
export function splitFullName(fullName: string): { owner: string; repo: string } | null {
  const parts = fullName.split('/');
  if (parts.length !== 2 || parts[0] === '' || parts[1] === '') return null;
  return { owner: parts[0], repo: parts[1] };
}

/**
 * Does enabling shared mode on `githubName` need a look at GitHub? Only when the
 * repo owner is not this install's own login (the owner needs no verification).
 */
export function needsOwnAdminCheck(githubName: string | null | undefined, login: string | null | undefined): boolean {
  const parts = githubName ? splitFullName(githubName) : null;
  const me = (login ?? '').trim();
  if (!parts || me === '') return false;
  return parts.owner.toLowerCase() !== me.toLowerCase();
}

/**
 * Combine the results of a run and of its one re-run: the counters add up (the
 * first run really did that work), the lists are joined without repeats, and the
 * issues and the remaining checks are those of the second run -- it saw the
 * state after the verification.
 */
export function mergeRerun(first: SyncResult, second: SyncResult): SyncResult {
  const join = (a: string[], b: string[]) => [...a, ...b.filter((x) => !a.includes(x))];
  return {
    copied: first.copied + second.copied,
    responses: first.responses + second.responses,
    migrated: first.migrated + second.migrated,
    errors: second.errors,
    auto_closed: join(first.auto_closed, second.auto_closed),
    committed: join(first.committed, second.committed),
    rules_taken: join(first.rules_taken, second.rules_taken),
    rules_lost: join(first.rules_lost, second.rules_lost),
    admin_checks: second.admin_checks ?? [],
  };
}
