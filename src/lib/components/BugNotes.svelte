<script lang="ts">
  import { onMount } from 'svelte';
  import { bugs, bugWarnings, confirmedCount, showConfirmed, addBug, refreshBugs, toggleShowConfirmed } from '$lib/stores/bugs';
  import { tStore, tf } from '$lib/i18n';
  import BugItem from './BugItem.svelte';
  import EmptyState from './EmptyState.svelte';

  interface Props {
    repoRole: string;
  }

  let { repoRole }: Props = $props();

  // Re-reconcile on every bug-tab open (remount). RepoDetail's $effect fires
  // only on repo switch; tab-switching within the same repo doesn't retrigger
  // loadBugsForRepo, so MD edits made while the user was on another tab would
  // otherwise stay invisible until explicit Refresh. OnMount closes that gap.
  // Safe when currentRepoId is already set by initial loadBugsForRepo; no-op
  // when user hasn't selected a repo yet.
  onMount(() => {
    refreshBugs();
  });

  async function handleAddBug() {
    await addBug('', 'medium', 'other');
  }

  async function handleRefresh() {
    await refreshBugs();
  }

  async function handleToggleConfirmed() {
    await toggleShowConfirmed();
  }
</script>

<div class="bug-notes">
  <div class="bugs-header">
    <h3 class="section-title">{$tStore('bugNotes.title')}</h3>
    <div class="header-actions">
      <label class="confirmed-toggle" title={$tStore('bugs.showConfirmedHint' as any)}>
        <input type="checkbox" checked={$showConfirmed} onchange={handleToggleConfirmed} />
        {tf('bugs.showConfirmed' as any, String($confirmedCount))}
      </label>
      <button class="ghost" onclick={handleRefresh} title={$tStore('bugNotes.reloadTooltip')} type="button">
        {$tStore('bugNotes.reload')}
      </button>
      <button class="ghost" onclick={handleAddBug} title={$tStore('bugNotes.addBugTooltip')} type="button">
        {$tStore('bugNotes.addBug')}
      </button>
    </div>
  </div>

  {#if $bugWarnings.length > 0}
    <div class="warnings-banner">
      {#each $bugWarnings as w}
        <p>⚠ {w}</p>
      {/each}
    </div>
  {/if}

  <div class="bug-scroll">
    {#if $bugs.length === 0}
      <EmptyState icon="🐛" title={$tStore('bugNotes.noBugs')} hint={$tStore('bugNotes.noBugsHint')} />
    {:else}
      <div class="bug-list">
        {#each $bugs as bug (bug.id)}
          <BugItem {bug} {repoRole} />
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .bug-notes {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 12px 24px 24px;
  }

  .bugs-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
    flex-shrink: 0;
  }

  .section-title {
    font-size: 14px;
    font-weight: 600;
  }

  .header-actions {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .confirmed-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-muted);
    cursor: pointer;
    user-select: none;
  }

  .confirmed-toggle input[type="checkbox"] {
    margin: 0;
    cursor: pointer;
  }

  .bug-scroll {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }

  .bug-list {
    background-color: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px 10px;
  }

  .warnings-banner {
    background-color: rgba(245, 158, 11, 0.1);
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: 4px;
    padding: 6px 10px;
    margin-bottom: 8px;
    font-size: 12px;
    color: #f59e0b;
    flex-shrink: 0;
  }

  .warnings-banner p { margin: 0; }
</style>
