<script lang="ts">
  import type { CategoryEfficiencyRow } from '$lib/types';
  import { tStore } from '$lib/i18n';

  interface Props {
    rows: CategoryEfficiencyRow[];
  }

  let { rows }: Props = $props();

  const sortedVisibleRows = $derived.by(() => {
    return rows
      .filter(r => r.touched_in_period > 0)
      .sort((a, b) => {
        const ra = a.resolution_rate ?? -1;
        const rb = b.resolution_rate ?? -1;
        if (ra !== rb) return rb - ra;
        return b.touched_in_period - a.touched_in_period;
      });
  });

  function barClass(rate: number | null): string {
    if (rate === null) return 'low';
    if (rate >= 75) return 'high';
    if (rate >= 35) return 'mid';
    return 'low';
  }
</script>

<div class="section">
  <div class="section-title">{$tStore('dashboard.categoriesTitle' as any)}</div>
  <div class="section-hint">{$tStore('dashboard.categoriesHint' as any)}</div>

  {#if sortedVisibleRows.length === 0}
    <div class="empty">{$tStore('dashboard.noDataInPeriod' as any)}</div>
  {:else}
    {#each sortedVisibleRows as row (row.category)}
      <div class="bar-row">
        <span class="bar-label">{row.category}</span>
        <div class="bar-track">
          <div class="bar-fill {barClass(row.resolution_rate)}"
               style="width: {Math.max(row.resolution_rate ?? 0, 0)}%">
            {Math.round(row.resolution_rate ?? 0)}%
          </div>
        </div>
        <span class="bar-meta">
          {row.touched_in_period} {$tStore('dashboard.bugsAbbrev')} · {row.attempts_in_period} {$tStore('dashboard.attemptsAbbrev')}
        </span>
      </div>
    {/each}
  {/if}
</div>

<style>
  .section { margin-bottom: 14px; }
  .section-title { font-size: 12px; font-weight: 600; color: var(--text); margin-bottom: 3px; }
  .section-hint { font-size: 10px; color: var(--text-muted); margin-bottom: 8px; }
  .empty { padding: 16px; text-align: center; color: var(--text-muted); font-size: 11px; }
  .bar-row { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; font-size: 11px; }
  .bar-label { width: 110px; color: var(--text); font-weight: 500; font-family: monospace; }
  .bar-track { flex: 1; height: 17px; background: rgba(255, 255, 255, 0.05); border-radius: 3px; overflow: hidden; }
  .bar-fill {
    height: 100%; border-radius: 3px; display: flex; align-items: center;
    padding-left: 7px; color: white; font-size: 10px; font-weight: 600;
    min-width: 28px;
  }
  .bar-fill.high { background: linear-gradient(90deg, #22c55e, #16a34a); }
  .bar-fill.mid { background: linear-gradient(90deg, #f59e0b, #d97706); }
  .bar-fill.low { background: linear-gradient(90deg, #ef4444, #dc2626); }
  .bar-meta { width: 130px; text-align: right; font-variant-numeric: tabular-nums; font-size: 10px; color: var(--text-muted); }
</style>
