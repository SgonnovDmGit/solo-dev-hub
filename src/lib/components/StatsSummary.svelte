<script lang="ts">
  import type { StatsSummary } from '$lib/types';
  import { tStore, tf } from '$lib/i18n';
  import { getDisplayName } from '$lib/types';
  import type { Repository } from '$lib/types';

  interface Props {
    summary: StatsSummary | null;
    scope: 'repo' | 'project';
    /** Map repo_id → Repository for getDisplayName lookup in TopHot. Project-scope only. */
    reposLookup?: Map<number, Repository>;
  }
  let { summary, scope, reposLookup }: Props = $props();

  // tf positional: {0}=date, {1}=days, {2}=repoCount (project-only)
  const banner = $derived.by(() => {
    if (!summary || !summary.lifetime_since) return $tStore('stats.summary.bannerEmpty' as any);
    if (scope === 'repo') {
      return tf('stats.summary.bannerRepo' as any,
        summary.lifetime_since,
        summary.days_history);
    }
    return tf('stats.summary.bannerProject' as any,
      summary.lifetime_since,
      summary.days_history,
      summary.repo_count ?? 0);
  });

  const fixRatePercent = $derived(summary ? Math.round(summary.kpi.fix_rate * 100) : 0);

  function fmtAvg(v: number): string {
    return (Math.round(v * 10) / 10).toFixed(1);
  }

  function fmtMedian(v: number): string {
    // Median is integer-valued in our impl (LIMIT 1 OFFSET, single row from int column)
    return String(Math.round(v));
  }

  function topHotName(h: { repo_id: number; github_name: string | null; description: string | null }): string {
    if (reposLookup) {
      const r = reposLookup.get(h.repo_id);
      if (r) return getDisplayName(r);
    }
    // Fallback: github_name last segment, or description, or repo_id placeholder
    if (h.github_name) {
      const parts = h.github_name.split('/');
      return parts[parts.length - 1];
    }
    return h.description ?? `repo #${h.repo_id}`;
  }
</script>

{#if !summary}
  <div class="no-data">{$tStore('stats.summary.noDataSummary' as any)}</div>
{:else}
  <div class="lifetime-banner">{banner}</div>

  <!-- KPI row (4 tiles) -->
  <div class="kpi-row">
    <!-- Active -->
    <div class="kpi">
      <div class="kpi-label">{$tStore('stats.summary.kpiActive' as any)}</div>
      <div class="kpi-hint">{$tStore('stats.summary.kpiActiveHint' as any)}</div>
      <div class="kpi-value">{summary.kpi.active}</div>
      <div class="kpi-sub">
        {#if summary.kpi.active_critical > 0}
          {@html tf('stats.summary.kpiActiveSubCritical' as any,
            `<span class="crit">${summary.kpi.active_critical}</span>`)}
        {:else}
          {$tStore('stats.summary.kpiActiveSubNoCritical' as any)}
        {/if}
      </div>
    </div>

    <!-- Closed total -->
    <div class="kpi">
      <div class="kpi-label">{$tStore('stats.summary.kpiClosed' as any)}</div>
      <div class="kpi-hint">{$tStore('stats.summary.kpiClosedHint' as any)}</div>
      <div class="kpi-value">{summary.kpi.closed_total}</div>
      <div class="kpi-sub">
        {tf('stats.summary.kpiClosedSub' as any, summary.days_history)}
      </div>
    </div>

    <!-- Avg attempts -->
    <div class="kpi">
      <div class="kpi-label">{$tStore('stats.summary.kpiAttempts' as any)}</div>
      <div class="kpi-hint">{$tStore('stats.summary.kpiAttemptsHint' as any)}</div>
      <div class="kpi-value">{summary.kpi.closed_total > 0 ? fmtAvg(summary.kpi.avg_attempts) : '—'}</div>
      <div class="kpi-sub">
        {#if summary.kpi.closed_total > 0}
          {tf('stats.summary.kpiAttemptsSub' as any, fmtMedian(summary.kpi.median_attempts))}
        {:else}
          {$tStore('stats.summary.kpiAttemptsNoData' as any)}
        {/if}
      </div>
    </div>

    <!-- Fix rate -->
    <div class="kpi">
      <div class="kpi-label">{$tStore('stats.summary.kpiFixRate' as any)}</div>
      <div class="kpi-hint">{$tStore('stats.summary.kpiFixRateHint' as any)}</div>
      <div class="kpi-value">{summary.kpi.created_total > 0 ? fixRatePercent + '%' : '—'}</div>
      <div class="kpi-sub">
        {tf('stats.summary.kpiFixRateSub' as any,
          summary.kpi.closed_total,
          summary.kpi.created_total)}
      </div>
    </div>
  </div>

  <!-- Top hot repos (project-scope only) -->
  {#if scope === 'project' && summary.top_hot_repos && summary.top_hot_repos.length > 0}
    <div class="stats-card">
      <div class="section-title" title={$tStore('dashboard.topHotFormulaTooltip' as any)}>
        {$tStore('stats.summary.topHotTitle' as any)}
        <span class="count">({$tStore('stats.summary.topHotSubtitle' as any)})</span>
      </div>
      {#each summary.top_hot_repos as hot, idx}
        <div class="hot-row">
          <div class="hot-rank">{idx + 1}.</div>
          <div class="hot-name">{topHotName(hot)}</div>
          <div class="hot-meta">
            {#if hot.critical > 0}
              <span class="crit">{hot.critical} {$tStore('dashboard.topHotCritShort' as any)}</span> /
            {:else}
              {hot.critical} {$tStore('dashboard.topHotCritShort' as any)} /
            {/if}
            {hot.major} {$tStore('dashboard.topHotMajShort' as any)} /
            {hot.active} {$tStore('dashboard.topHotActShort' as any)} /
            {hot.bugs_closed} {$tStore('dashboard.topHotClosedShort' as any)}
            <span class="sep">·</span>
            {hot.tasks_done} {$tStore('dashboard.topHotTasksShort' as any)}
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <!-- Category efficiency bars -->
  {#if summary.categories.length > 0}
    <div class="stats-card">
      <div class="section-title">
        {$tStore('stats.summary.categoriesTitle' as any)}
        <span class="count">({scope === 'repo'
          ? $tStore('stats.summary.categoriesSubtitleRepo' as any)
          : $tStore('stats.summary.categoriesSubtitleProject' as any)})</span>
      </div>
      {#each summary.categories as cat}
        {@const barClass = cat.percent >= 70 ? '' : cat.percent >= 50 ? ' low' : ' bad'}
        <div class="cat-row">
          <div class="cat-label">{$tStore(`category.${cat.category}` as any)}</div>
          <div class="cat-bar-wrap"><div class="cat-bar{barClass}" style="width: {cat.percent}%"></div></div>
          <div class="cat-num">{Math.round(cat.percent)}% ({cat.closed}/{cat.total})</div>
        </div>
      {/each}
    </div>
  {/if}
{/if}

<style>
  .no-data { font-size: 12px; color: var(--text-muted); padding: 12px 0; font-style: italic; }
  .lifetime-banner { font-size: 11px; color: var(--text-muted); margin-bottom: 14px; font-style: italic; }
  .stats-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 16px;
    margin-bottom: 14px;
  }
  .kpi-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
    margin-bottom: 14px;
  }
  .kpi {
    background: rgba(0, 0, 0, 0.2);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 9px 10px;
  }
  :global([data-theme="light"]) .kpi { background: rgba(0, 0, 0, 0.04); }
  .kpi-label { font-size: 11px; font-weight: 600; margin-bottom: 3px; color: var(--text); }
  .kpi-hint { font-size: 10px; color: var(--text-muted); margin-bottom: 6px; min-height: 22px; line-height: 1.3; }
  .kpi-value { font-size: 22px; font-weight: 700; line-height: 1; font-variant-numeric: tabular-nums; color: var(--text); }
  .kpi-sub { font-size: 10px; color: var(--text-muted); margin-top: 4px; min-height: 12px; }
  .kpi-sub :global(.crit) { color: var(--danger); font-weight: 600; }
  .section-title { font-size: 12px; font-weight: 600; margin-bottom: 10px; color: var(--text); }
  .section-title .count { font-size: 11px; color: var(--text-muted); font-weight: 400; margin-left: 6px; }
  .hot-row {
    display: grid;
    grid-template-columns: 18px 1fr auto;
    gap: 10px;
    align-items: center;
    padding: 8px 4px;
    border-bottom: 1px solid var(--border);
  }
  .hot-row:last-child { border-bottom: none; }
  .hot-rank { color: var(--text-muted); font-size: 12px; font-weight: 600; }
  .hot-name { font-weight: 500; font-size: 12px; color: var(--text); }
  .hot-meta { font-size: 11px; color: var(--text-muted); }
  .hot-meta :global(.crit) { color: var(--danger); font-weight: 600; }
  .hot-meta :global(.sep) { margin: 0 2px; opacity: 0.6; }
  .section-title[title] { cursor: help; }
  .cat-row {
    display: grid;
    grid-template-columns: 110px 1fr 80px;
    gap: 10px;
    align-items: center;
    margin-bottom: 6px;
  }
  .cat-label { font-size: 11px; color: var(--text); }
  .cat-bar-wrap {
    height: 12px;
    background: rgba(0, 0, 0, 0.25);
    border-radius: 3px;
    overflow: hidden;
    border: 1px solid var(--border);
  }
  :global([data-theme="light"]) .cat-bar-wrap { background: rgba(0, 0, 0, 0.06); }
  .cat-bar { height: 100%; background: linear-gradient(90deg, #22c55e, #4ade80); }
  .cat-bar.low { background: linear-gradient(90deg, #f59e0b, #fbbf24); }
  .cat-bar.bad { background: linear-gradient(90deg, var(--danger), #f87171); }
  .cat-num { font-size: 11px; color: var(--text-muted); text-align: right; font-variant-numeric: tabular-nums; }
</style>
