import { tf } from '$lib/i18n';
import { addToast } from '$lib/stores/ui';
import { aiRulesLastSyncAt } from '$lib/stores/settings';
import { syncGlobalClaudeMd } from '$lib/api/tauri-commands';
import type { TemplateSaveOutcome } from '$lib/types';
import { formatSyncIssues } from './sync-issues';
import { toastRulesFindings } from './rules-report';

/**
 * Report a save / reset / conflict decision on a rule. A skill is re-rendered
 * into every repo, so say where it landed; anything else (the global section,
 * which only reaches disk on "Sync") gets `fallbackKey`'s plain toast.
 */
export function reportRuleOutcome(
  outcome: TemplateSaveOutcome,
  fallbackKey?: 'toast.templateSaved' | 'toast.templateReset',
): void {
  if (!outcome.rendered) {
    if (fallbackKey) addToast(tf(fallbackKey), 'success');
    return;
  }
  addToast(tf('toast.ruleApplied', outcome.skills, outcome.repos), 'success');
  toastRulesFindings(outcome);
  if (outcome.errors.length > 0) {
    addToast(tf('toast.ruleApplyErrors', formatSyncIssues(outcome.errors)), 'error');
  }
}

/**
 * "Sync" — write the global section and skills to ~/.claude and report it.
 * Returns true when the sync ran, so the caller can re-read the overview.
 */
export async function syncRulesFromApp(): Promise<boolean> {
  try {
    const result = await syncGlobalClaudeMd();
    aiRulesLastSyncAt.set(result.synced_at);
    // T-000207: the render collected pending hand-backs first — name them.
    toastRulesFindings(result.rules);
    if (result.rules.errors.length > 0) {
      addToast(tf('toast.ruleApplyErrors', formatSyncIssues(result.rules.errors)), 'error');
    }
    addToast(tf('appDefaults.syncGlobalDone', result.path), 'success');
    return true;
  } catch (err) {
    addToast(String(err), 'error');
    return false;
  }
}
