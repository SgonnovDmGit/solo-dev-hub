import { writable, get } from 'svelte/store';
import type { BugView } from '$lib/types';
import {
  ensureBugsMigrated,
  reconcileBugsForRepo,
  readBugsFromDb,
  countConfirmedBugs,
  createBug,
  updateBugFields,
  deleteBug,
  resolveBug,
  rejectBug,
} from '$lib/api/tauri-commands';
import { addToast } from './ui';
import { tf, t } from '$lib/i18n';

export const bugs = writable<BugView[]>([]);
export const bugWarnings = writable<string[]>([]);
export const showConfirmed = writable<boolean>(false);
export const confirmedCount = writable<number>(0);

let currentRepoId: number | null = null;

/**
 * v0.16.0 load flow — DB-centric. Triggered on bug-tab open or repo switch.
 * 1. `ensure_bugs_migrated` — idempotent lazy MD→DB import on first open.
 *    Shows toast only on first actual migration (not when `already=true`).
 * 2. `reconcile_bugs_for_repo` — syncs LLM edits of status/comment from MD
 *    into DB, silently reverts protected-field violations via regen.
 * 3. `read_bugs_from_db` + `count_confirmed_bugs` — populate stores.
 *
 * If the repo has no `local_path`, migration/reconcile are skipped — the DB
 * still holds whatever was created via the app UI.
 */
export async function loadBugsForRepo(repoId: number, hasLocalPath: boolean): Promise<void> {
  currentRepoId = repoId;
  try {
    if (hasLocalPath) {
      const report = await ensureBugsMigrated(repoId);
      if (!report.already && report.imported > 0) {
        addToast(
          tf('bugs.migrationToast' as any, String(report.imported), String(report.confirmed_archived)),
          'success',
        );
      }
      await reconcileBugsForRepo(repoId);
    }
    const include = get(showConfirmed);
    bugs.set(await readBugsFromDb(repoId, include));
    confirmedCount.set(await countConfirmedBugs(repoId));
    bugWarnings.set([]);
  } catch (err) {
    bugs.set([]);
    confirmedCount.set(0);
    bugWarnings.set([String(err)]);
    addToast(tf('bugs.migrationError' as any, String(err)), 'error');
  }
}

/**
 * Refresh button handler — reconcile + re-read from DB.
 * Same as loadBugsForRepo but skips the migration-toast path (migration
 * already happened on initial load).
 */
export async function refreshBugs(): Promise<void> {
  if (currentRepoId === null) return;
  try {
    await reconcileBugsForRepo(currentRepoId);
    const include = get(showConfirmed);
    bugs.set(await readBugsFromDb(currentRepoId, include));
    confirmedCount.set(await countConfirmedBugs(currentRepoId));
  } catch (err) {
    addToast(tf('toast.bugsSaveFailed', String(err)), 'error');
  }
}

/**
 * Re-read bugs from DB for current repo without a full reconcile.
 * Used after local mutations (create/update/resolve/reject/delete) where
 * backend has already regenerated MD — we just need to refresh the UI list.
 */
async function reloadBugsList(): Promise<void> {
  if (currentRepoId === null) return;
  const include = get(showConfirmed);
  bugs.set(await readBugsFromDb(currentRepoId, include));
  confirmedCount.set(await countConfirmedBugs(currentRepoId));
}

export async function toggleShowConfirmed(): Promise<void> {
  showConfirmed.update((v) => !v);
  await reloadBugsList();
}

export async function addBug(
  description: string = '',
  severity: string = 'minor',
  category: string = 'other',
): Promise<void> {
  if (currentRepoId === null) return;
  try {
    await createBug(currentRepoId, description, severity, category);
    await reloadBugsList();
  } catch (err) {
    addToast(tf('toast.bugsSaveFailed', String(err)), 'error');
  }
}

export async function editBug(
  displayId: string,
  description?: string,
  severity?: string,
  category?: string,
  comment?: string,
): Promise<void> {
  if (currentRepoId === null) return;
  try {
    await updateBugFields(currentRepoId, displayId, {
      description,
      severity,
      category,
      comment,
    });
    await reloadBugsList();
  } catch (err) {
    addToast(tf('toast.bugsSaveFailed', String(err)), 'error');
  }
}

/** Mark bug as confirmed (✓ button). Valid from `testing` only.
 *  v0.21.1: optimistic local update — keep the just-confirmed row visible in the
 *  list (with `confirmed` status visual marker) instead of refetching from DB
 *  which would filter it out immediately. User gets "yes, the click registered"
 *  feedback. Manual Refresh re-reads from DB and the row drops to the archive view.
 */
export async function confirmBug(displayId: string): Promise<void> {
  if (currentRepoId === null) return;
  try {
    await resolveBug(currentRepoId, displayId);
    const today = new Date().toISOString().slice(0, 10);
    bugs.update((list) => list.map((b) =>
      b.id === displayId ? { ...b, status: 'confirmed', confirmed_at: today } : b,
    ));
    confirmedCount.update((n) => n + 1);
  } catch (err) {
    addToast(String(err), 'error');
  }
}

/** Mark bug as rejected (✗ button). Valid from `testing` only.
 *  If `comment` is provided, updates the comment first (in the same flow).
 */
export async function rejectBugWithComment(displayId: string, comment?: string): Promise<void> {
  if (currentRepoId === null) return;
  try {
    if (comment && comment.trim().length > 0) {
      await updateBugFields(currentRepoId, displayId, { comment: comment.trim() });
    }
    await rejectBug(currentRepoId, displayId);
    await reloadBugsList();
  } catch (err) {
    addToast(String(err), 'error');
  }
}

/** Hard-delete a bug — UI only offers this on `status='created'` (accidental
 *  creation). For real closed bugs, use confirmBug (soft-archive for history). */
export async function removeBug(displayId: string): Promise<void> {
  if (currentRepoId === null) return;
  try {
    await deleteBug(currentRepoId, displayId);
    await reloadBugsList();
  } catch (err) {
    addToast(tf('toast.bugsSaveFailed', String(err)), 'error');
  }
}

export function clearBugs(): void {
  bugs.set([]);
  bugWarnings.set([]);
  confirmedCount.set(0);
  currentRepoId = null;
}
