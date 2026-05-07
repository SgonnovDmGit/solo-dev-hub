<script lang="ts">
  import type { KpiCard } from '$lib/types';
  import { tf, tStore } from '$lib/i18n';

  interface Props {
    label: string;
    hint: string;
    card: KpiCard;
    formatValue?: (v: number) => string;
    invertDelta?: boolean;  // true for "Attempts per closed" (lower = better)
  }

  let { label, hint, card, formatValue, invertDelta = false }: Props = $props();

  const fmt = $derived(formatValue ?? ((v: number) => String(v)));

  const valueStr = $derived(card.value === null ? '—' : fmt(card.value));

  const delta = $derived.by(() => {
    if (card.value === null || card.prev_value === null) return null;
    return card.value - card.prev_value;
  });

  const deltaClass = $derived.by(() => {
    if (delta === null || delta === 0) return 'neutral';
    const positive = invertDelta ? delta < 0 : delta > 0;
    return positive ? 'good' : 'bad';
  });

  const deltaStr = $derived.by(() => {
    if (delta === null) return null;
    const sign = delta > 0 ? '+' : '';
    return `${sign}${fmt(delta)}`;
  });
</script>

<div class="kpi">
  <div class="kpi-label">{label}</div>
  <div class="kpi-label-hint">{hint}</div>
  <div class="kpi-value">{valueStr}</div>
  <div class="kpi-sub">
    {#if card.critical_count !== null && card.critical_count !== undefined}
      {@html tf('dashboard.kpi.activeBugsSubCritical' as any, `<span class="bad">${card.critical_count}</span>`)}
    {:else if deltaStr !== null && card.prev_value !== null}
      <span class={deltaClass}>{deltaStr}</span> {$tStore('dashboard.deltaToPrev')} ({fmt(card.prev_value)})
    {/if}
  </div>
</div>

<style>
  .kpi {
    background: rgba(0, 0, 0, 0.2);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 9px 10px;
  }
  .kpi-label { font-size: 11px; color: var(--text); font-weight: 600; margin-bottom: 3px; }
  .kpi-label-hint {
    font-size: 10px; color: var(--text-muted);
    margin-bottom: 6px; min-height: 22px; line-height: 1.3;
  }
  .kpi-value {
    font-size: 22px; font-weight: 700;
    font-variant-numeric: tabular-nums; line-height: 1; color: var(--text);
  }
  .kpi-sub { font-size: 10px; color: var(--text-muted); margin-top: 3px; }
  .kpi-sub :global(.good) { color: #22c55e; }
  .kpi-sub :global(.bad) { color: #ef4444; }
  .kpi-sub .good { color: #22c55e; }
  .kpi-sub .bad { color: #ef4444; }
  .kpi-sub .neutral { color: var(--text-muted); }
</style>
