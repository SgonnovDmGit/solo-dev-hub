<script lang="ts">
  import { onMount } from 'svelte';
  import {
    currentPeriod,
    dashboardData,
    dashboardLoading,
    loadDashboard,
  } from '$lib/stores/dashboard';
  import { tStore } from '$lib/i18n';
  import DashboardFilters from './DashboardFilters.svelte';
  import DashboardKpi from './DashboardKpi.svelte';
  import DashboardTopHot from './DashboardTopHot.svelte';
  import DashboardDailyChart from './DashboardDailyChart.svelte';
  import DashboardCategoryBars from './DashboardCategoryBars.svelte';
  import DashboardActivityFeed from './DashboardActivityFeed.svelte';

  onMount(() => {
    loadDashboard();
  });

  function rateFmt(v: number): string {
    return `${Math.round(v)}%`;
  }

  function attemptsFmt(v: number): string {
    return v.toFixed(1);
  }

  function rangeText(p: { start: string; end: string }): string {
    return `${p.start} — ${p.end}`;
  }
</script>

<div class="dashboard">
  <div class="dash-header">
    <div>
      <h2 class="dash-title">{$tStore('dashboard.title' as any)}</h2>
      <div class="range-line">
        <span class="period">{rangeText($currentPeriod)}</span>
      </div>
    </div>
    <DashboardFilters />
  </div>

  {#if $dashboardLoading && !$dashboardData}
    <div class="loading">...</div>
  {:else if !$dashboardData}
    <p class="no-data">{$tStore('dashboard.noDataInPeriod' as any)}</p>
  {:else}
    <div class="kpi-row">
      <DashboardKpi
        label={$tStore('dashboard.kpi.activeBugs' as any)}
        hint={$tStore('dashboard.kpi.activeBugsHint' as any)}
        card={$dashboardData.active_bugs}
      />
      <DashboardKpi
        label={$tStore('dashboard.kpi.closedInPeriod' as any)}
        hint={$tStore('dashboard.kpi.closedInPeriodHint' as any)}
        card={$dashboardData.closed_in_period}
      />
      <DashboardKpi
        label={$tStore('dashboard.kpi.tasksDone' as any)}
        hint={$tStore('dashboard.kpi.tasksDoneHint' as any)}
        card={$dashboardData.tasks_done}
      />
      <DashboardKpi
        label={$tStore('dashboard.kpi.solveRate' as any)}
        hint={$tStore('dashboard.kpi.solveRateHint' as any)}
        card={$dashboardData.solve_rate}
        formatValue={rateFmt}
      />
      <DashboardKpi
        label={$tStore('dashboard.kpi.attemptsPerClosed' as any)}
        hint={$tStore('dashboard.kpi.attemptsPerClosedHint' as any)}
        card={$dashboardData.attempts_per_closed}
        formatValue={attemptsFmt}
        invertDelta={true}
      />
    </div>

    <DashboardTopHot projects={$dashboardData.top_hot} />

    <div class="section">
      <div class="section-title">{$tStore('dashboard.dailyFlowTitle' as any)}</div>
      <DashboardDailyChart days={$dashboardData.bugs_daily} variant="bugs" />
      <DashboardDailyChart days={$dashboardData.tasks_daily} variant="tasks" />
    </div>

    <DashboardCategoryBars rows={$dashboardData.categories} />
  {/if}

  <DashboardActivityFeed />
</div>

<style>
  .dashboard { padding: 16px; height: 100%; display: flex; flex-direction: column; gap: 12px; overflow: auto; }
  .dash-header {
    display: flex; align-items: flex-start; justify-content: space-between;
    padding-bottom: 10px; border-bottom: 1px solid var(--border);
    gap: 12px; flex-wrap: wrap;
  }
  .dash-title { font-size: 16px; font-weight: 700; margin: 0 0 3px 0; }
  .range-line { font-size: 10.5px; color: var(--text-muted); line-height: 1.4; }
  .range-line .period { color: var(--text); font-weight: 600; }
  .loading { padding: 20px; color: var(--text-muted); font-size: 12px; }
  .no-data { padding: 20px; color: var(--text-muted); font-size: 12px; }
  .kpi-row { display: grid; grid-template-columns: repeat(5, 1fr); gap: 8px; margin-bottom: 14px; }
  .section { margin-bottom: 14px; }
  .section-title { font-size: 12px; font-weight: 600; color: var(--text); margin-bottom: 3px; }

  @media (max-width: 1100px) {
    .kpi-row { grid-template-columns: repeat(3, 1fr); }
  }
  @media (max-width: 700px) {
    .kpi-row { grid-template-columns: repeat(2, 1fr); }
  }
</style>
