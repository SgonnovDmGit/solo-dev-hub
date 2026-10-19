<script lang="ts">
  import { tStore, tf } from '$lib/i18n';
  import type { RuleDiff } from '$lib/utils/rule-diff';

  let {
    diff,
    emptyText,
    legend = false,
  }: {
    diff: RuleDiff;
    /** Shown when the two texts are equal. */
    emptyText: string;
    /** Show the "− canon / + current text" legend above the rows. */
    legend?: boolean;
  } = $props();
</script>

{#if legend && diff.rows.length > 0}
  <div class="legend">
    <span class="del-key">− {$tStore('rules.diffOld')}</span>
    <span class="add-key">+ {$tStore('rules.diffNew')}</span>
  </div>
{/if}

{#if diff.rows.length === 0}
  <div class="empty">{emptyText}</div>
{:else}
  <div class="rows" role="table">
    {#each diff.rows as row}
      {#if row.kind === 'gap'}
        <div class="row gap" role="row">
          <span class="no"></span><span class="no"></span><span class="sign"></span>
          <span class="text">⋯ {tf('rules.diffGap', row.hidden)}</span>
        </div>
      {:else}
        <div class="row {row.kind}" role="row">
          <span class="no">{row.kind === 'add' ? '' : row.oldNo}</span>
          <span class="no">{row.kind === 'del' ? '' : row.newNo}</span>
          <span class="sign">{row.kind === 'del' ? '−' : row.kind === 'add' ? '+' : ''}</span>
          <span class="text">{row.text}</span>
        </div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .legend { display: flex; gap: 14px; font-size: 11px; margin-bottom: 6px; }
  /* Same red / green as the +N −M counters on the tab. */
  .del-key { color: #f87171; }
  .add-key { color: #4ade80; }
  .empty { font-size: 12px; color: var(--text-muted); padding: 12px 0; }
  .rows {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 11px;
    line-height: 1.5;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    overflow-x: auto;
  }
  .row { display: grid; grid-template-columns: 40px 40px 16px 1fr; min-width: max-content; width: 100%; }
  .no { text-align: right; padding-right: 6px; color: var(--text-muted); user-select: none; }
  .sign { text-align: center; color: var(--text-muted); user-select: none; }
  .text { white-space: pre; padding-right: 8px; }
  /* Row tints: red for removed, green for added (literal rgba — no theme variable exists). */
  .row.del { background: rgba(239, 68, 68, 0.15); }
  .row.add { background: rgba(34, 197, 94, 0.15); }
  .row.gap { color: var(--text-muted); background: var(--bg); font-style: italic; }
</style>
