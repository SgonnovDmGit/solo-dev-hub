import { tf } from '$lib/i18n';
import { addToast } from '$lib/stores/ui';

/**
 * T-000207: toast what a rule refresh found beside the agents' copies. Every
 * path that renders rule copies collects hand-backs first, and nothing verifies
 * the approval behind a marker — so whatever was taken up, lost or set aside is
 * always named, wherever the refresh ran. Errors stay with the caller: each
 * surface already words its own failure toast.
 */
export function toastRulesFindings(r: { taken: string[]; lost: string[]; superseded: string[] }): void {
  if (r.taken.length > 0) {
    addToast(tf('toast.rulesTaken', r.taken.join(', ')), 'info');
  }
  if (r.lost.length > 0) {
    addToast(tf('toast.rulesLost', r.lost.join(', ')), 'warning');
  }
  if (r.superseded.length > 0) {
    addToast(tf('toast.rulesSuperseded', r.superseded.join(', ')), 'info');
  }
}
