<script lang="ts">
  import { tStore } from '$lib/i18n';
  import { syncTasksForRepo, readDoneFromDb } from '$lib/api/tauri-commands';
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
  }

  interface Props { repoId: number; }
  let { repoId }: Props = $props();

  let tasks = $state<Task[]>([]);
  let loading = $state(true);

  const columns: ColumnDef<Task>[] = $derived([
    { key: 'task_id', label: $tStore('done.col.id' as any), sortable: true, render: 'monospace', flex: 0.6 },
    { key: 'description', label: $tStore('done.col.description' as any), sortable: false, filter: 'text', flex: 4, wrap: true },
    { key: 'created_at', label: $tStore('done.col.date' as any), sortable: true, render: 'date', flex: 1 },
    { key: 'version', label: $tStore('done.col.version' as any), sortable: true, filter: 'text', flex: 1 },
  ]);

  async function load() {
    loading = true;
    try {
      tasks = await readDoneFromDb(repoId);
    } finally {
      loading = false;
    }
  }

  // B-000011: reload on repoId change. RepoDetail keeps the same DoneTab
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

<div class="done-tab">
  <div class="header">
    <h3>{$tStore('done.tabTitle' as any)}</h3>
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
        defaultSort={{ key: 'created_at', direction: 'desc' }}
        persistKey={`done_grid_state_${repoId}`}
        emptyMessage={$tStore('done.empty' as any)}
      />
    {/key}
  {/if}
</div>

<style>
  .done-tab { flex: 1; display: flex; flex-direction: column; min-height: 0; padding: 12px 16px; }
  .header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; flex-shrink: 0; }
  h3 { margin: 0; font-size: 14px; font-weight: 600; }
  .refresh-btn { background: none; border: 1px solid var(--border); border-radius: 4px; padding: 2px 8px; cursor: pointer; color: var(--text-muted); }
  .refresh-btn:hover { color: var(--text); background: var(--surface-hover); }
  .muted { color: var(--text-muted); padding: 12px 0; }
</style>
