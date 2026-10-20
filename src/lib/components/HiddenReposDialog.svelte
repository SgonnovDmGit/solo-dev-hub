<script lang="ts">
  import { tStore, tf } from '$lib/i18n';
  import type { HiddenCandidate } from '$lib/utils/hidden-repos';
  import type { Repository } from '$lib/types';

  interface Props {
    candidates: HiddenCandidate<Repository>[];
    busy?: boolean;
    onRemove: (ids: number[]) => void;
    onDismiss: () => void;
  }

  let { candidates, busy = false, onRemove, onDismiss }: Props = $props();

  // One-shot snapshot of the preselection; the user edits it from here.
  // svelte-ignore state_referenced_locally
  let selected = $state<Set<number>>(
    new Set(candidates.filter((c) => c.preselected).map((c) => c.repo.id)),
  );

  const allSelected = $derived(selected.size === candidates.length);

  function toggle(id: number) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selected = next;
  }

  // Focus inside the dialog on open, so Escape works without a click first.
  let notNowButton: HTMLButtonElement | undefined = $state();
  $effect(() => {
    notNowButton?.focus();
  });

  function setAll(on: boolean) {
    selected = on ? new Set(candidates.map((c) => c.repo.id)) : new Set();
  }
</script>

<div
  class="overlay"
  role="presentation"
  onclick={() => !busy && onDismiss()}
  onkeydown={(e) => e.key === 'Escape' && !busy && onDismiss()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="hidden-repos-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && !busy && onDismiss()}
  >
    <h3 id="hidden-repos-title" class="title">{tf('hiddenRepos.title' as any, candidates.length)}</h3>
    <p class="message">{$tStore('hiddenRepos.description' as any)}</p>

    <div class="list-toolbar">
      <button type="button" class="mini" onclick={() => setAll(true)} disabled={allSelected}>
        {$tStore('hiddenRepos.selectAll' as any)}
      </button>
      <button type="button" class="mini" onclick={() => setAll(false)} disabled={selected.size === 0}>
        {$tStore('hiddenRepos.deselectAll' as any)}
      </button>
      <span class="counter">{tf('hiddenRepos.nSelected' as any, selected.size)}</span>
    </div>

    <div class="repo-list">
      {#each candidates as c (c.repo.id)}
        <label class="row">
          <input type="checkbox" checked={selected.has(c.repo.id)} onchange={() => toggle(c.repo.id)} />
          <span class="info">
            <span class="name">{c.repo.github_name}</span>
            <span class="path" class:muted={!c.repo.local_path}>
              {c.repo.local_path ?? $tStore('hiddenRepos.noLocal' as any)}
            </span>
          </span>
        </label>
      {/each}
    </div>

    <div class="actions">
      <button type="button" bind:this={notNowButton} onclick={onDismiss} disabled={busy}>
        {$tStore('hiddenRepos.notNow' as any)}
      </button>
      <button
        class="primary"
        type="button"
        onclick={() => onRemove([...selected])}
        disabled={busy || selected.size === 0}
      >
        {$tStore('hiddenRepos.remove' as any)}
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
    padding: 12px;
  }

  .dialog {
    background-color: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 24px;
    width: 640px;
    max-width: 100%;
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

  .list-toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
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

  .repo-list {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 6px;
    margin-bottom: 12px;
  }

  .row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 4px 6px;
    cursor: pointer;
    border-radius: 3px;
  }

  .row:hover {
    background: var(--border);
  }

  .row input[type='checkbox'] {
    cursor: pointer;
    flex-shrink: 0;
    margin-top: 3px;
  }

  .info {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .name {
    font-size: 13px;
    color: var(--text);
    word-break: break-all;
  }

  .path {
    font-family: monospace;
    font-size: 11px;
    color: var(--text-muted);
    word-break: break-all;
  }

  .path.muted {
    font-family: inherit;
    font-style: italic;
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    flex-wrap: wrap;
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
