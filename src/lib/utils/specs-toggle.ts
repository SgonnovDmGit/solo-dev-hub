// T-000243 review: the pure core of the specs checkbox of `SharedModeChip`.
// Two things went wrong around a naive `onchange` + `await save()`:
//
// * the browser flips the native checkbox BEFORE `onchange` runs, so a rejected
//   save left the box showing a value the backend never accepted (the DB stayed
//   on, the checkbox said off). Everything the checkbox displays now comes from
//   the outcome of this function: on any refusal it is the STORED value again.
// * the backend setter needs the owner / a verified administrator of the repo,
//   and the chip is shown from a repo-list permission snapshot: an
//   organization's admin had no `shared_admin_verified` record yet and was
//   refused. The shared-mode switch bridges that with `verifyAdmin` first; the
//   specs switch does the same here (it persists the record, it never enables
//   sharing).
//
// No Svelte, no Tauri, no network in this file — the component injects the real
// calls, the tests inject fakes (see `__tests__/specs-toggle.test.ts`).

import { needsOwnAdminCheck } from '$lib/utils/shared-admin';

/** The outside world the toggle needs. */
export interface SpecsSaveDeps {
  /** T-000227 bridge: ask GitHub and persist `shared_admin_verified`. Throws on an API failure. */
  verifyAdmin: (token: string, repoFullName: string, login: string) => Promise<{ admin: boolean }>;
  /** The Tauri command: persists the flag, may refuse, may return a warning note. */
  setRepoShareSpecs: (repoId: number, on: boolean) => Promise<unknown>;
}

export interface SpecsSaveInput {
  repoId: number;
  /** `owner/name` of the repo; null for a local-only repo. */
  repoFullName: string | null;
  /** This install's GitHub login. */
  login: string | null;
  /** The GitHub token; without it an organization's repo is treated as not admin (as the shared switch does). */
  token: string | null;
  /** The value the backend holds right now — never a guess read from the DOM. */
  stored: boolean;
}

export type SpecsSaveOutcome =
  /** Saved: `value` is the new stored value, `note` an optional warning to show. */
  | { ok: true; value: boolean; note: string | null }
  /** Not saved: `value` is what the checkbox must display again (the stored one). */
  | { ok: false; value: boolean; kind: 'notAdmin' | 'failed'; error?: unknown };

/**
 * Flip the specs policy of one repo. The admin bridge runs BEFORE the save and
 * never enables sharing; a refused or failed save changes nothing, so the caller
 * rolls the displayed value back to `value`.
 */
export async function saveSpecsToggle(
  input: SpecsSaveInput,
  deps: SpecsSaveDeps,
): Promise<SpecsSaveOutcome> {
  const next = !input.stored;
  if (input.repoFullName && input.login && needsOwnAdminCheck(input.repoFullName, input.login)) {
    try {
      const admin = input.token
        ? (await deps.verifyAdmin(input.token, input.repoFullName, input.login)).admin
        : false;
      if (!admin) {
        return { ok: false, value: input.stored, kind: 'notAdmin' };
      }
    } catch (err) {
      return { ok: false, value: input.stored, kind: 'failed', error: err };
    }
  }
  try {
    const note = await deps.setRepoShareSpecs(input.repoId, next);
    return { ok: true, value: next, note: typeof note === 'string' ? note : null };
  } catch (err) {
    // A failed save must not look like a successful switch-off.
    return { ok: false, value: input.stored, kind: 'failed', error: err };
  }
}
