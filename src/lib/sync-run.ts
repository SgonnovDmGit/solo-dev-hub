// T-000235: the two sync runs a user can start from several screens (Sync
// screen, sidebar, repo screen). The logic lives here once; callers keep only
// their own local busy flag and any follow-up reload.
import { addToast } from '$lib/stores/ui';
import { isSyncing } from '$lib/stores/repos';
import { refreshAfterSync } from '$lib/stores/shared';
import { noteManualSync } from '$lib/stores/autosync';
import { syncProjectCore, probeSharedCore } from '$lib/sync-core';
import { toastRulesFindings } from '$lib/utils/rules-report';
import { formatSyncIssues } from '$lib/utils/sync-issues';
import { tf } from '$lib/i18n';
import type { SyncResult } from '$lib/types';

// T-000227: the toast-free core (sync + administrator checks + at most one re-run).
export { syncProjectCore, probeSharedCore } from '$lib/sync-core';

/**
 * Sync one project and report the outcome in toasts. Resolves to the result,
 * or null when the sync threw (the error is already toasted).
 */
export async function runProjectSync(projectId: number): Promise<SyncResult | null> {
  // T-000136: reset the auto-sync elapsed clock so the next timed run waits a
  // full interval after this manual sync.
  noteManualSync();
  try {
    const result = await syncProjectCore(projectId);
    const msgKey = result.migrated > 0 ? 'sync.syncCompleteFull' : 'sync.syncComplete';
    const msg =
      result.migrated > 0
        ? tf(msgKey, result.copied, result.responses, result.migrated)
        : tf(msgKey, result.copied, result.responses);
    addToast(msg, 'success');
    if (result.errors.length > 0) {
      // T-000198: one translated line per issue under a counted header.
      const header = tf('sync.errors', result.errors.length);
      addToast(`${header}\n${formatSyncIssues(result.errors)}`, 'warning');
    }
    // F-000039: report REQ pairs auto-closed via .impl.md acknowledgement.
    if (result.auto_closed.length > 0) {
      addToast(tf('sync.autoClosed', result.auto_closed.length), 'info');
    }
    // T-000137: report repos where synced cross-repo files were auto-committed.
    if (result.committed.length > 0) {
      addToast(tf('sync.committed', result.committed.length), 'info');
    }
    // T-000205 / T-000207: a rule taken back out of a repo's flagged copy now
    // applies to the whole portfolio, and a lost marker means an edit never
    // arrived. Nothing verifies the approval behind a flag, so both are always
    // named.
    toastRulesFindings({ taken: result.rules_taken, lost: result.rules_lost, superseded: [] });
    return result;
  } catch (e) {
    addToast(String(e), 'error');
    return null;
  } finally {
    // F-000044: statuses of shared repos are settled by the sync, and it may
    // have linked a contributor's repo to a project.
    void refreshAfterSync();
  }
}

/**
 * F-000044: a contributor who has no project yet has nothing to press a
 * project sync on. Probe the repos that are in no shared project for the
 * owner's structure; the backend runs the sync of every project they land in,
 * which also sends the join request.
 */
export async function runSharedProbe(): Promise<void> {
  isSyncing.set(true);
  try {
    const result = await probeSharedCore();
    if (result.errors.length > 0) {
      const header = tf('sync.errors', result.errors.length);
      addToast(`${header}\n${formatSyncIssues(result.errors)}`, 'warning');
    }
  } catch {
    // The probe is a side job of the repo-list sync: a failure must not
    // read as a failed sync.
  } finally {
    isSyncing.set(false);
    await refreshAfterSync();
  }
}
