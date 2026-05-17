<script lang="ts">
  import { tStore, tf } from '$lib/i18n';
  import { listGitignoredTracked, untrackFiles } from '$lib/api/tauri-commands';
  import type { GitignoredListing } from '$lib/types';
  import { addToast } from '$lib/stores/ui';

  interface Props {
    repositoryId: number;
    onClose: () => void;
  }

  let { repositoryId, onClose }: Props = $props();

  let loading = $state(true);
  let listing = $state<GitignoredListing | null>(null);
  let errorMsg = $state<string | null>(null);
  let selected = $state<Set<string>>(new Set());
  let submitting = $state(false);

  // Initial load: list gitignored-tracked files, default-check all.
  $effect(() => {
    void repositoryId;
    loading = true;
    listing = null;
    errorMsg = null;
    selected = new Set();

    listGitignoredTracked(repositoryId)
      .then((res) => {
        listing = res;
        selected = new Set(res.files);
      })
      .catch((e) => {
        errorMsg = String(e);
      })
      .finally(() => {
        loading = false;
      });
  });

  const midMergeBlocked = $derived(
    listing !== null && listing.repo_state !== 'clean'
  );
  const selectedCount = $derived(selected.size);
  const totalFiles = $derived(listing?.files.length ?? 0);
  const isAllSelected = $derived(totalFiles > 0 && selectedCount === totalFiles);
  const canUntrack = $derived(
    !midMergeBlocked && selectedCount > 0 && !submitting
  );

  function toggleFile(path: string) {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }

  function selectAll() {
    if (!listing) return;
    selected = new Set(listing.files);
  }

  function deselectAll() {
    selected = new Set();
  }

  async function handleUntrack() {
    if (!canUntrack) return;
    submitting = true;
    try {
      const files = Array.from(selected);
      const report = await untrackFiles(repositoryId, files);
      if (report.errors.length === 0) {
        addToast(
          $tStore('toast.untrackSuccess' as any).replace('{0}', String(report.untracked)),
          'success',
        );
      } else {
        addToast(
          $tStore('toast.untrackPartial' as any)
            .replace('{0}', String(report.untracked))
            .replace('{1}', String(report.errors.length)),
          'error',
        );
      }
      onClose();
    } catch (e) {
      errorMsg = String(e);
    } finally {
      submitting = false;
    }
  }
</script>

<div
  class="overlay"
  role="presentation"
  onclick={onClose}
  onkeydown={(e) => e.key === 'Escape' && onClose()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="untrack-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && onClose()}
  >
    <h3 id="untrack-title" class="title">{$tStore('untrackDialog.title' as any)}</h3>
    <p class="message">{$tStore('untrackDialog.description' as any)}</p>

    {#if loading}
      <div class="state">{$tStore('untrackDialog.loading' as any)}</div>
    {:else if errorMsg}
      <div class="state error">
        {tf('untrackDialog.errorRead' as any, errorMsg)}
      </div>
    {:else if listing && listing.files.length === 0}
      <div class="state">{$tStore('untrackDialog.emptyState' as any)}</div>
    {:else if listing}
      <div class="list-toolbar">
        <button
          type="button"
          class="mini"
          onclick={selectAll}
          disabled={isAllSelected}
        >{$tStore('untrackDialog.selectAll' as any)}</button>
        <button
          type="button"
          class="mini"
          onclick={deselectAll}
          disabled={selectedCount === 0}
        >{$tStore('untrackDialog.deselectAll' as any)}</button>
        <span class="counter">
          {tf('untrackDialog.nSelected' as any, selectedCount)}
        </span>
      </div>

      <div class="untrack-list">
        {#each listing.files as path (path)}
          <label class="row">
            <input
              type="checkbox"
              checked={selected.has(path)}
              onchange={() => toggleFile(path)}
            />
            <span class="path">{path}</span>
          </label>
        {/each}
      </div>

      {#if midMergeBlocked}
        <div class="banner error">
          {$tStore('untrackDialog.midMergeError' as any)}
        </div>
      {/if}

      {#if listing.other_staged_count > 0 && !midMergeBlocked}
        <div class="banner warning">
          {tf('untrackDialog.otherStagedWarning' as any, listing.other_staged_count)}
        </div>
      {/if}
    {/if}

    <div class="actions">
      <button type="button" onclick={onClose}>{$tStore('dialog.cancel' as any)}</button>
      <button
        class="primary"
        type="button"
        onclick={handleUntrack}
        disabled={!canUntrack}
      >
        {tf('untrackDialog.confirmAction' as any, selectedCount)}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background-color: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10000;
  }

  .dialog {
    background-color: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 24px;
    min-width: 520px;
    max-width: 720px;
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }

  .title {
    font-size: 15px;
    font-weight: 600;
    margin-bottom: 10px;
  }

  .message {
    font-size: 13px;
    color: var(--text-muted);
    margin-bottom: 12px;
    line-height: 1.5;
  }

  .state {
    font-size: 13px;
    color: var(--text-muted);
    padding: 16px;
    border: 1px dashed var(--border);
    border-radius: 4px;
    text-align: center;
    margin-bottom: 14px;
  }

  .state.error {
    color: #ef4444;
    border-color: #ef4444;
  }

  .list-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }

  .list-toolbar .mini {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }

  .list-toolbar .mini:hover:not(:disabled) {
    background: var(--surface);
    border-color: var(--accent);
  }

  .list-toolbar .mini:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .counter {
    margin-left: auto;
    font-size: 12px;
    color: var(--text-muted);
  }

  .untrack-list {
    max-height: 50vh;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 6px;
    margin-bottom: 12px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    cursor: pointer;
    border-radius: 3px;
  }

  .row:hover {
    background: var(--border);
  }

  .row input[type="checkbox"] {
    cursor: pointer;
    flex-shrink: 0;
  }

  .path {
    font-family: monospace;
    font-size: 12px;
    color: var(--text);
    word-break: break-all;
  }

  .banner {
    font-size: 12px;
    padding: 8px 10px;
    border-radius: 4px;
    margin-bottom: 10px;
    line-height: 1.4;
  }

  .banner.error {
    background: rgba(239, 68, 68, 0.12);
    color: #ef4444;
    border: 1px solid rgba(239, 68, 68, 0.4);
  }

  .banner.warning {
    background: rgba(234, 179, 8, 0.12);
    color: #eab308;
    border: 1px solid rgba(234, 179, 8, 0.4);
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 8px;
  }

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }

  .primary:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
