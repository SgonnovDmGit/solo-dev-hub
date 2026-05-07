<script lang="ts">
  import type { DailyFlowDay } from '$lib/types';
  import { tStore } from '$lib/i18n';

  interface Props {
    days: DailyFlowDay[];
    variant: 'bugs' | 'tasks';
  }

  let { days, variant }: Props = $props();

  const maxValue = $derived.by(() => {
    let m = 1;
    for (const d of days) {
      const vals = [d.opened ?? 0, d.closed ?? 0, d.done ?? 0];
      for (const v of vals) if (v > m) m = v;
    }
    return m;
  });

  function barHeight(v: number | null): string {
    if (v === null || v === 0) return '0px';
    return `${Math.round((v / maxValue) * 50)}px`;
  }

  function dayOfWeek(date: string): string {
    const d = new Date(date);
    return $tStore(`dashboard.dow.${d.getDay()}` as any);
  }

  function shortDate(date: string): string {
    return date.slice(8, 10);
  }

  const title = $derived(
    variant === 'bugs'
      ? $tStore('dashboard.dailyFlowBugsTitle' as any)
      : $tStore('dashboard.dailyFlowTasksTitle' as any)
  );

  const hasAnyData = $derived(
    days.some(d => (d.opened ?? 0) > 0 || (d.closed ?? 0) > 0 || (d.done ?? 0) > 0)
  );
</script>

<div class="chart-box">
  <div class="chart-head">
    <span class="chart-title">{title}</span>
    <span class="chart-legend">
      {#if variant === 'bugs'}
        <span><span class="legend-dot" style="background:#ef4444"></span>{$tStore('dashboard.dailyFlowLegendOpened' as any)}</span>
        <span><span class="legend-dot" style="background:#22c55e"></span>{$tStore('dashboard.dailyFlowLegendClosed' as any)}</span>
      {:else}
        <span><span class="legend-dot" style="background:#8b5cf6"></span>{$tStore('dashboard.dailyFlowLegendDone' as any)}</span>
      {/if}
    </span>
  </div>

  {#if !hasAnyData && variant === 'tasks'}
    <div class="empty">{$tStore('dashboard.dailyFlowNoTasks' as any)}</div>
  {:else}
    <div class="day-grid" style="grid-template-columns: repeat({days.length}, 1fr)">
      {#each days as d (d.date)}
        <div class="day-cell">
          <div class="day-nums">
            {#if d.is_future}
              —
            {:else if variant === 'bugs'}
              {(d.opened ?? 0)}/{(d.closed ?? 0)}
            {:else}
              {(d.done ?? 0)}
            {/if}
          </div>
          <div class="day-bars">
            {#if d.is_future}
              <div class="day-bar future" style="height: 50px"></div>
            {:else if variant === 'bugs'}
              <div class="day-bar opened" style="height: {barHeight(d.opened)}"></div>
              <div class="day-bar closed" style="height: {barHeight(d.closed)}"></div>
            {:else}
              <div class="day-bar task-done" style="height: {barHeight(d.done)}"></div>
            {/if}
          </div>
          <div class="day-label" class:future={d.is_future}>
            {dayOfWeek(d.date)} {shortDate(d.date)}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .chart-box { background: rgba(0, 0, 0, 0.15); border-radius: 5px; padding: 8px 12px; margin-bottom: 8px; }
  .chart-head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  .chart-title { font-size: 11px; font-weight: 600; color: var(--text); }
  .chart-legend { display: flex; gap: 10px; font-size: 9.5px; color: var(--text-muted); }
  .legend-dot { display: inline-block; width: 8px; height: 8px; border-radius: 2px; margin-right: 3px; vertical-align: middle; }
  .day-grid { display: grid; gap: 6px; }
  .day-cell { display: flex; flex-direction: column; align-items: center; gap: 3px; }
  .day-bars { display: flex; gap: 2px; align-items: flex-end; height: 50px; }
  .day-bar { width: 10px; border-radius: 2px 2px 0 0; }
  .day-bar.opened { background: linear-gradient(180deg, #ef4444, #dc2626); }
  .day-bar.closed { background: linear-gradient(180deg, #22c55e, #16a34a); }
  .day-bar.task-done { background: linear-gradient(180deg, #8b5cf6, #7c3aed); width: 14px; }
  .day-bar.future { background: rgba(255, 255, 255, 0.05); border: 1px dashed rgba(255, 255, 255, 0.12); }
  .day-label { font-size: 9.5px; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .day-label.future { opacity: 0.5; font-style: italic; }
  .day-nums { font-size: 9.5px; color: var(--text-muted); font-variant-numeric: tabular-nums; height: 11px; }
  .empty { padding: 20px; text-align: center; color: var(--text-muted); font-size: 11px; }
</style>
