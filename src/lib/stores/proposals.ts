// T-000264: the header chip's store. The summary is derived on the backend
// from live state (scan + acceptance rows), so polling never lies about
// persistence — after a restart the same numbers reappear, which is what
// makes the "auto-decided: N" surface a persisted notification without a
// notification store.
import { writable } from 'svelte/store';
import { ruleProposalsSummary } from '$lib/api/tauri-commands';
import type { RuleProposalsSummary } from '$lib/types';

export const proposalsSummary = writable<RuleProposalsSummary>({
  open_manual: 0,
  attention: false,
  auto_decided_recent: 0,
});

let timer: ReturnType<typeof setInterval> | null = null;
let inflight = false;

export async function refreshProposalsSummary(): Promise<void> {
  if (inflight) return;
  inflight = true;
  try {
    proposalsSummary.set(await ruleProposalsSummary());
  } catch {
    // A failed read keeps the last numbers — a chip that flickers to empty
    // on every transient error would cry wolf.
  } finally {
    inflight = false;
  }
}

/** Start the 60s poll; idempotent. Called once from the root layout. */
export function startProposalsPolling(): void {
  if (timer) return;
  void refreshProposalsSummary();
  timer = setInterval(() => void refreshProposalsSummary(), 60_000);
}
