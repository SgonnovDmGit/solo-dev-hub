<script lang="ts">
  import { tStore } from '$lib/i18n';
  import { syncTasksForRepo, readTasksFromDb } from '$lib/api/tauri-commands';
  import type { Task } from '$lib/types';
  import DataGrid from './DataGrid.svelte';

  interface ColumnDef<T> {
    key: keyof T & string;
    label: string;
    sortable: boolean;
    filter?: 'text' | 'select' | 'none';
    selectOptions?: string[];
    render?: 'default' | 'monospace' | 'priority-color' | 'date';
    flex?: number;
    wrap?: boolean;
    sortWeight?: Record<string, number>;
    labelMap?: Record<string, string>;
  }

  interface Props { repoId: number; }
  let { repoId }: Props = $props();

  let tasks = $state<Task[]>([]);
  let loading = $state(true);

  // B-000005: $derived — columns rebuild on locale switch (was const, captured
  // initial $tStore values once). sortWeight gives priority/status workflow
  // order instead of alphabetical; labelMap shows localized labels in cells +
  // filter dropdown + chips while raw values stay unchanged for match logic.
  const PRIORITY_WEIGHT: Record<string, number> = { critical: 0, high: 1, medium: 2, low: 3 };
  const TASK_STATUS_WEIGHT: Record<string, number> = { open: 0, 'in-progress': 1, review: 2 };
  const columns = $derived<ColumnDef<Task>[]>([
    { key: 'task_id', label: $tStore('tasks.col.id' as any), sortable: true, render: 'monospace', flex: 0.6 },
    { key: 'description', label: $tStore('tasks.col.description' as any), sortable: false, filter: 'text', flex: 4, wrap: true },
    { key: 'effort', label: $tStore('tasks.col.effort' as any), sortable: true, flex: 0.6 },
    {
      key: 'priority',
      label: $tStore('tasks.col.priority' as any),
      sortable: true,
      filter: 'select',
      selectOptions: ['critical', 'high', 'medium', 'low'],
      render: 'priority-color',
      flex: 1.3,
      sortWeight: PRIORITY_WEIGHT,
      labelMap: {
        critical: $tStore('tasks.priority.critical' as any),
        high: $tStore('tasks.priority.high' as any),
        medium: $tStore('tasks.priority.medium' as any),
        low: $tStore('tasks.priority.low' as any),
      },
    },
    {
      key: 'status',
      label: $tStore('tasks.col.status' as any),
      sortable: true,
      filter: 'select',
      selectOptions: ['open', 'in-progress', 'review'],
      flex: 1.3,
      sortWeight: TASK_STATUS_WEIGHT,
      labelMap: {
        open: $tStore('tasks.status.open' as any),
        'in-progress': $tStore('tasks.status.in-progress' as any),
        review: $tStore('tasks.status.review' as any),
      },
    },
    { key: 'created_at', label: $tStore('tasks.col.createdAt' as any), sortable: true, render: 'date', flex: 1 },
  ]);

  async function load() {
    loading = true;
    try {
      tasks = await readTasksFromDb(repoId);
    } finally {
      loading = false;
    }
  }

  // B-000011: reload on repoId change. RepoDetail keeps the same TasksTab
  // instance when switching repos (only prop changes), so onMount fires only
  // once. $effect re-runs whenever repoId changes and reloads the grid.
  $effect(() => {
    void repoId;
    load();
  });

  async function refresh() {
    await syncTasksForRepo(repoId);
    await load();
  }
</script>

<div class="tasks-tab">
  <div class="header">
    <h3>{$tStore('tasks.tabTitle' as any)}</h3>
    <button class="refresh-btn" onclick={refresh} title="Refresh">↻</button>
  </div>
  {#if loading}
    <p class="muted">{$tStore('common.loading' as any)}</p>
  {:else}
    <!-- B-000011: {#key repoId} forces DataGrid to remount on repo switch so
         persisted sort/filter state is reloaded from the new repo's setting,
         instead of leaking the previous repo's grid state across persistKey. -->
    {#key repoId}
      <DataGrid
        {columns}
        rows={tasks}
        defaultSort={{ key: 'priority', direction: 'asc' }}
        persistKey={`tasks_grid_state_${repoId}`}
        emptyMessage={$tStore('tasks.empty' as any)}
      />
    {/key}
  {/if}
</div>

<style>
  .tasks-tab { flex: 1; display: flex; flex-direction: column; min-height: 0; padding: 12px 16px; }
  .header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; flex-shrink: 0; }
  h3 { margin: 0; font-size: 14px; font-weight: 600; }
  .refresh-btn { background: none; border: 1px solid var(--border); border-radius: 4px; padding: 2px 8px; cursor: pointer; color: var(--text-muted); }
  .refresh-btn:hover { color: var(--text); background: var(--surface-hover); }
  .muted { color: var(--text-muted); padding: 12px 0; }
</style>
