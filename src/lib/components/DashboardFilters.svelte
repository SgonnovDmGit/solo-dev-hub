<script lang="ts">
  import { projects } from '$lib/stores/projects';
  import {
    currentPreset,
    currentPeriod,
    selectedProjectIds,
    setPreset,
    setProjectFilter,
  } from '$lib/stores/dashboard';
  import type { PeriodPreset } from '$lib/types';
  import { tStore } from '$lib/i18n';

  let projectsOpen = $state(false);

  const presets: PeriodPreset[] = ['week', 'month', 'quarter', 'custom'];

  function presetLabel(p: PeriodPreset): string {
    const key = `dashboard.period${p.charAt(0).toUpperCase() + p.slice(1)}`;
    return $tStore(key as any);
  }

  async function handlePreset(p: PeriodPreset) {
    await setPreset(p);
  }

  async function toggleProject(projId: number) {
    const current = $selectedProjectIds ?? $projects.map(p => p.id);
    const has = current.includes(projId);
    const next = has ? current.filter(id => id !== projId) : [...current, projId];
    // Deselect-all behaves same as all (null)
    const normalized = next.length === 0 || next.length === $projects.length ? null : next;
    await setProjectFilter(normalized);
  }

  async function selectAllProjects() {
    await setProjectFilter(null);
  }

  async function deselectAllProjects() {
    // Same as all — display count 0, but data = all repos
    await setProjectFilter(null);
  }

  const selectedCount = $derived(
    $selectedProjectIds === null ? $projects.length : $selectedProjectIds.length
  );

  function isProjectSelected(id: number): boolean {
    if ($selectedProjectIds === null) return true;
    return $selectedProjectIds.includes(id);
  }
</script>

<div class="filters">
  <div class="filter-block">
    <span class="filter-label">{$tStore('dashboard.periodLabel' as any)}</span>
    <span class="period-picker">
      {#each presets as p (p)}
        <button class:active={$currentPreset === p} onclick={() => handlePreset(p)} type="button">
          {presetLabel(p)}
        </button>
      {/each}
    </span>
  </div>

  <div class="filter-block">
    <span class="filter-label">{$tStore('dashboard.projectsLabel' as any)}</span>
    <div class="project-filter-wrap">
      <button class="project-filter" onclick={() => (projectsOpen = !projectsOpen)} type="button">
        <span><span class="count">{selectedCount}</span> {$tStore('dashboard.outOfFmt').replace('{0}', String($projects.length))}</span>
        <span class="arrow">▼</span>
      </button>
      {#if projectsOpen}
        <div class="project-dropdown">
          <div class="drop-actions">
            <button onclick={selectAllProjects} type="button">{$tStore('common.selectAll')}</button>
            <button onclick={deselectAllProjects} type="button">{$tStore('common.clearAll')}</button>
          </div>
          {#each $projects as p (p.id)}
            <label class="drop-item">
              <input type="checkbox" checked={isProjectSelected(p.id)} onchange={() => toggleProject(p.id)} />
              {p.name}
            </label>
          {/each}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .filters { display: flex; gap: 10px; align-items: flex-end; flex-wrap: wrap; }
  .filter-block { display: flex; flex-direction: column; gap: 3px; }
  .filter-label {
    font-size: 9.5px; text-transform: uppercase;
    letter-spacing: 0.06em; color: var(--text-muted);
  }
  .period-picker {
    display: inline-flex; gap: 0;
    background: rgba(0, 0, 0, 0.3);
    border-radius: 5px; padding: 2px;
  }
  .period-picker button {
    background: transparent; border: none; color: var(--text-muted);
    font-size: 11px; padding: 4px 9px; border-radius: 3px; cursor: pointer;
  }
  .period-picker button.active { background: var(--accent, #7c3aed); color: white; }

  .project-filter-wrap { position: relative; }
  .project-filter {
    display: inline-flex; align-items: center; gap: 6px;
    background: rgba(0, 0, 0, 0.3);
    border: 1px solid var(--border);
    border-radius: 5px; padding: 5px 10px; font-size: 11px;
    cursor: pointer; color: var(--text); min-width: 160px;
  }
  .project-filter .count { color: var(--accent, #7c3aed); font-weight: 600; }
  .project-filter .arrow { margin-left: auto; color: var(--text-muted); }

  .project-dropdown {
    position: absolute; top: 100%; right: 0; z-index: 10;
    background: var(--surface); border: 1px solid var(--border);
    border-radius: 5px; padding: 6px; margin-top: 2px;
    min-width: 200px; max-height: 300px; overflow-y: auto;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  }
  .drop-actions { display: flex; gap: 6px; margin-bottom: 6px; padding-bottom: 4px; border-bottom: 1px solid var(--border); }
  .drop-actions button {
    font-size: 10px; padding: 3px 8px;
    background: rgba(255, 255, 255, 0.05); border: none;
    color: var(--text); border-radius: 3px; cursor: pointer;
  }
  .drop-item { display: flex; align-items: center; gap: 6px; padding: 4px; font-size: 11px; cursor: pointer; }
  .drop-item input[type="checkbox"] { margin: 0; }
</style>
