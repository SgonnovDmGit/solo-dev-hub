<script lang="ts">
  // F-000044 / T-000223 (frame 4): the status line of a shared repo, in the
  // place of the auto-commit "paused" line. One line per status code with its
  // action: "Слить" for a diverged current branch, "Принять новую ветку
  // главного" for a changed owner's branch.
  import { tStore } from '$lib/i18n';
  import { addToast } from '$lib/stores/ui';
  import { mergeSharedUpstream, acceptNewMainBranch } from '$lib/api/tauri-commands';
  import type { RepoSharedState } from '$lib/types';
  import { describeSharedStatus, fillParams, parseMergeDetail } from '$lib/utils/shared-mode';

  interface Props {
    repoId: number;
    shared: RepoSharedState;
    /** The owner's login (structure), for the "pending approval" line. */
    ownerLogin: string | null;
    /** An action changed the state: reload it. */
    onRefresh: () => void;
  }

  let { repoId, shared, ownerLogin, onRefresh }: Props = $props();

  const line = $derived(describeSharedStatus(shared, ownerLogin));
  const text = $derived(
    line ? fillParams($tStore(line.textKey as any), line.params) : '',
  );

  let busy = $state(false);
  // "Слить" stopped on a code conflict: shown as a second, error line.
  let conflictText = $state<string | null>(null);

  // A new status (or none) makes an old conflict message stale. The signature
  // is a derived string, so a reload that brings the same status keeps it.
  const signature = $derived(`${shared.shared_status ?? ''}|${shared.shared_status_detail ?? ''}`);
  $effect(() => {
    void signature;
    conflictText = null;
  });

  async function handleMerge() {
    if (busy) return;
    busy = true;
    conflictText = null;
    try {
      const res = await mergeSharedUpstream(repoId);
      if (res.outcome === 'merged') {
        addToast($tStore('shared.merge.done' as any), 'success');
      } else if (res.outcome === 'up_to_date') {
        addToast($tStore('shared.merge.upToDate' as any), 'info');
      } else {
        const branch = parseMergeDetail(shared.shared_status_detail)?.branch ?? '<branch>';
        conflictText = $tStore('shared.merge.conflict' as any)
          .replace('{0}', res.files.join(', '))
          .replace('{1}', branch);
      }
      onRefresh();
    } catch (err) {
      addToast($tStore('shared.merge.failed' as any).replace('{0}', String(err)), 'error');
    } finally {
      busy = false;
    }
  }

  async function handleAccept() {
    if (busy) return;
    busy = true;
    try {
      await acceptNewMainBranch(repoId);
      addToast($tStore('shared.acceptBranch.done' as any), 'success');
      onRefresh();
    } catch (err) {
      addToast($tStore('shared.acceptBranch.failed' as any).replace('{0}', String(err)), 'error');
    } finally {
      busy = false;
    }
  }
</script>

{#if line}
  <div class="shared-status {line.severity}" data-status={line.code}>
    <span class="icon" aria-hidden="true">{line.icon}</span>
    <span class="text">{text}</span>
    {#if line.action === 'merge'}
      <button type="button" class="action" onclick={handleMerge} disabled={busy} title={$tStore('shared.merge.hint' as any)}>
        {$tStore('shared.merge.button' as any)}
      </button>
    {:else if line.action === 'accept_branch'}
      <button type="button" class="action" onclick={handleAccept} disabled={busy} title={$tStore('shared.acceptBranch.hint' as any)}>
        {$tStore('shared.acceptBranch.button' as any)}
      </button>
    {/if}
  </div>
{/if}
{#if conflictText}
  <div class="shared-status error" data-status="merge_conflict">
    <span class="icon" aria-hidden="true">✕</span>
    <span class="text">{conflictText}</span>
  </div>
{/if}

<style>
  .shared-status {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 12px;
    padding-top: 4px;
    line-height: 1.4;
  }
  .shared-status.warn {
    color: var(--toast-warning);
  }
  .shared-status.error {
    color: var(--danger);
  }
  .shared-status.info {
    color: var(--text-muted);
  }
  .icon {
    flex-shrink: 0;
  }
  .text {
    min-width: 0;
  }
  .action {
    flex-shrink: 0;
    font-size: 11px;
    padding: 1px 10px;
    border-radius: 4px;
    border: 1px solid currentColor;
    background: transparent;
    color: inherit;
    cursor: pointer;
    white-space: nowrap;
  }
  .action:hover:not(:disabled) {
    background: var(--surface);
  }
  .action:disabled {
    opacity: 0.5;
    cursor: wait;
  }
</style>
