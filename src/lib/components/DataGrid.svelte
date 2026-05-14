<script lang="ts" generics="T extends Record<string, any>">
  import { onMount } from 'svelte';
  import { getSetting, setSetting } from '$lib/api/tauri-commands';
  import { tStore } from '$lib/i18n';

  interface ColumnDef<T> {
    key: keyof T & string;
    label: string;
    sortable: boolean;
    filter?: 'text' | 'select' | 'none';
    selectOptions?: string[];
    render?: 'default' | 'monospace' | 'priority-color' | 'date';
    flex?: number;
    wrap?: boolean;
    // B-000005: when set, sort uses these numeric weights instead of
    // localeCompare. Lower weight = earlier in asc order. Values not in the
    // map fall back to a sentinel after all known weights, sorted alphabetically.
    sortWeight?: Record<string, number>;
    // B-000005: when set, cell renders the mapped label (e.g. localized) but
    // the underlying value is unchanged for filter/search matching.
    labelMap?: Record<string, string>;
    // T-000109: when set, sort uses this comparator instead of localeCompare /
    // sortWeight. Higher-precedence than `sortWeight` (mutually exclusive in
    // practice — set one or the other per column). Used for SemVer-aware
    // version column ordering (v0.10.0 > v0.9.0).
    sortCompare?: (a: any, b: any) => number;
  }

  interface Props {
    columns: ColumnDef<T>[];
    rows: T[];
    defaultSort?: { key: string; direction: 'asc' | 'desc' };
    persistKey?: string;
    emptyMessage?: string;
  }

  let { columns, rows, defaultSort, persistKey, emptyMessage }: Props = $props();

  const totalFlex = $derived(columns.reduce((s, c) => s + (c.flex ?? 1), 0));
  function widthPercent(col: ColumnDef<T>): number {
    return ((col.flex ?? 1) / totalFlex) * 100;
  }

  let sortKey = $state<string | null>(null);
  let sortDirection = $state<'asc' | 'desc' | null>(null);
  let filters = $state<Record<string, string[]>>({});
  let openFilterKey = $state<string | null>(null);
  let searchText = $state('');
  let debouncedSearch = $state('');

  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    if (searchTimer) clearTimeout(searchTimer);
    const t = searchText;
    searchTimer = setTimeout(() => { debouncedSearch = t; }, 150);
  });

  onMount(async () => {
    // Apply defaultSort as baseline; persist overrides if present.
    sortKey = defaultSort?.key ?? null;
    sortDirection = defaultSort?.direction ?? null;
    if (!persistKey) return;
    try {
      const stored = await getSetting(persistKey);
      if (stored) {
        const state = JSON.parse(stored);
        sortKey = state.sortKey ?? null;
        sortDirection = state.sortDirection ?? null;
        filters = state.filters ?? {};
      }
    } catch (e) { console.warn('grid persist load fail', e); }
  });

  let persistTimer: ReturnType<typeof setTimeout> | null = null;
  function persistDebounced() {
    if (!persistKey) return;
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => {
      const state = { sortKey, sortDirection, filters };
      setSetting(persistKey!, JSON.stringify(state)).catch((e) => console.warn(e));
    }, 300);
  }

  $effect(() => {
    void sortKey; void sortDirection; void filters;
    persistDebounced();
  });

  function toggleSort(col: ColumnDef<T>) {
    if (!col.sortable) return;
    if (sortKey !== col.key) {
      sortKey = col.key;
      sortDirection = 'asc';
    } else if (sortDirection === 'asc') {
      sortDirection = 'desc';
    } else {
      sortKey = null;
      sortDirection = null;
    }
  }

  function toggleFilter(colKey: string, value: string) {
    const current = filters[colKey] ?? [];
    const idx = current.indexOf(value);
    if (idx >= 0) {
      filters[colKey] = current.filter((v) => v !== value);
      if (filters[colKey].length === 0) delete filters[colKey];
    } else {
      filters[colKey] = [...current, value];
    }
  }

  function clearFilter(colKey: string, value: string) {
    filters[colKey] = (filters[colKey] ?? []).filter((v) => v !== value);
    if (filters[colKey].length === 0) delete filters[colKey];
  }

  const filteredRows = $derived.by(() => {
    let r = rows;
    if (debouncedSearch.trim()) {
      const q = debouncedSearch.toLowerCase();
      const searchCols = columns
        .filter((c) => c.filter === 'text' || c.key === 'description' || c.render === 'monospace')
        .map((c) => c.key);
      r = r.filter((row) =>
        searchCols.some((k) => String(row[k] ?? '').toLowerCase().includes(q))
      );
    }
    for (const [k, vals] of Object.entries(filters)) {
      if (vals.length === 0) continue;
      r = r.filter((row) => vals.includes(String(row[k] ?? '')));
    }
    if (sortKey && sortDirection) {
      const key = sortKey;
      const dir = sortDirection === 'asc' ? 1 : -1;
      // B-000005: if the column declares sortWeight, sort by numeric weight
      // (workflow / severity order) instead of alphabetic localeCompare.
      // Unknown values fall back to Number.MAX_SAFE_INTEGER so they bucket
      // together at the end, then break ties alphabetically.
      const col = columns.find((c) => c.key === key);
      const weights = col?.sortWeight;
      const customCompare = col?.sortCompare;
      r = [...r].sort((a, b) => {
        const av = a[key], bv = b[key];
        // T-000109: custom comparator wins. Use it for SemVer-aware version
        // ordering. The comparator itself decides how to treat null/empty.
        if (customCompare) return customCompare(av, bv) * dir;
        if (av == null && bv == null) return 0;
        if (av == null) return 1;
        if (bv == null) return -1;
        if (weights) {
          const aw = weights[String(av)] ?? Number.MAX_SAFE_INTEGER;
          const bw = weights[String(bv)] ?? Number.MAX_SAFE_INTEGER;
          if (aw !== bw) return (aw - bw) * dir;
          return String(av).localeCompare(String(bv)) * dir;
        }
        if (typeof av === 'number' && typeof bv === 'number') return (av - bv) * dir;
        return String(av).localeCompare(String(bv)) * dir;
      });
    }
    return r;
  });

  function cellRender(col: ColumnDef<T>, row: T): string {
    const v = row[col.key];
    if (v == null) return '';
    if (col.render === 'date' && typeof v === 'string') {
      return v.length >= 10 ? v.slice(0, 10) : v;
    }
    const raw = String(v);
    // B-000005: localized label if mapped, raw value otherwise (preserves
    // non-format values like custom statuses unchanged).
    if (col.labelMap && col.labelMap[raw] != null) {
      return col.labelMap[raw].replace(/\\n/g, '\n');
    }
    return raw.replace(/\\n/g, '\n');
  }

  // B-000005: shared helper for filter dropdown options and chips.
  function displayLabel(col: ColumnDef<T> | undefined, raw: string): string {
    if (col?.labelMap && col.labelMap[raw] != null) return col.labelMap[raw];
    return raw;
  }

  function priorityColor(p: string): string {
    if (p === 'critical') return 'rgb(239, 68, 68)';
    if (p === 'high') return 'rgb(249, 115, 22)';
    if (p === 'medium') return 'rgb(234, 179, 8)';
    return 'var(--text-muted)';
  }

  // M4 review-fix: close the column filter dropdown on outside-click and Esc.
  // Without this it stayed open after the user clicked anywhere else in the
  // grid, breaking the "open one filter at a time" expectation. Mirrors the
  // pattern used in InputContextMenu (B-000007).
  function handleDocumentClick(e: MouseEvent) {
    if (openFilterKey === null) return;
    const target = e.target as Element | null;
    if (target && target.closest('.filter-dropdown')) return;
    openFilterKey = null;
  }
  function handleKeydown(e: KeyboardEvent) {
    if (openFilterKey !== null && e.key === 'Escape') openFilterKey = null;
  }
</script>

<svelte:window onclick={handleDocumentClick} onkeydown={handleKeydown} />

<div class="grid">
  <div class="toolbar">
    <input
      type="search"
      class="search"
      placeholder={$tStore('grid.searchPlaceholder' as any)}
      bind:value={searchText}
    />
  </div>
  {#if Object.values(filters).some((v) => v.length > 0)}
    <div class="filter-chips">
      {#each Object.entries(filters) as [k, vals]}
        {#each vals as v}
          {@const col = columns.find((c) => c.key === k)}
          <button class="chip" onclick={() => clearFilter(k, v)}>{col?.label ?? k}: {displayLabel(col, v)} ✕</button>
        {/each}
      {/each}
    </div>
  {/if}
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          {#each columns as col}
            <th
              class:sortable={col.sortable}
              style="width: {widthPercent(col)}%;"
              onclick={() => toggleSort(col)}
            >
              <div class="th-inner">
                <span>{col.label}</span>
                {#if col.sortable && sortKey === col.key}
                  <span class="sort-icon">{sortDirection === 'asc' ? '▴' : '▾'}</span>
                {/if}
                {#if col.filter === 'select' && col.selectOptions}
                  <div class="filter-dropdown">
                    <button
                      type="button"
                      class="filter-btn"
                      class:active={(filters[col.key] ?? []).length > 0}
                      onclick={(e) => { e.stopPropagation(); openFilterKey = openFilterKey === col.key ? null : col.key; }}
                      aria-label="Filter {col.label}"
                      title="Фильтр"
                    >▾{#if (filters[col.key] ?? []).length > 0}<span class="badge">{(filters[col.key] ?? []).length}</span>{/if}</button>
                    {#if openFilterKey === col.key}
                      <!-- svelte-ignore a11y_click_events_have_key_events -->
                      <!-- svelte-ignore a11y_no_static_element_interactions -->
                      <div class="filter-list" onclick={(e) => e.stopPropagation()}>
                        {#each col.selectOptions as opt}
                          <label>
                            <input type="checkbox"
                              checked={(filters[col.key] ?? []).includes(opt)}
                              onchange={() => toggleFilter(col.key, opt)} />
                            {displayLabel(col, opt)}
                          </label>
                        {/each}
                      </div>
                    {/if}
                  </div>
                {/if}
              </div>
            </th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#if filteredRows.length === 0}
          <tr><td colspan={columns.length} class="empty">{emptyMessage ?? 'Нет данных'}</td></tr>
        {:else}
          {#each filteredRows as row, i (i)}
            <tr>
              {#each columns as col}
                <td
                  class="cell"
                  class:monospace={col.render === 'monospace' || col.render === 'date'}
                  class:wrap={col.wrap}
                  style="{col.render === 'priority-color' ? `color: ${priorityColor(String(row[col.key]))}` : ''}"
                  title={String(row[col.key] ?? '').replace(/\\n/g, '\n')}
                >
                  {cellRender(col, row)}
                </td>
              {/each}
            </tr>
          {/each}
        {/if}
      </tbody>
    </table>
  </div>
</div>

<style>
  .grid { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .toolbar {
    display: flex;
    gap: 8px;
    padding: 8px 0;
    flex-shrink: 0;
  }
  .search {
    flex: 1;
    padding: 6px 10px;
    font-size: 13px;
  }
  .filter-chips {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    padding: 0 0 6px 0;
  }
  .chip {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    cursor: pointer;
  }
  .chip:hover { background: var(--surface-hover); }
  .table-wrap {
    flex: 1;
    overflow: auto;
    border-top: 1px solid var(--border);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
    table-layout: fixed;
  }
  thead {
    position: sticky;
    top: 0;
    background: var(--bg);
    z-index: 1;
  }
  th, .cell {
    text-align: left;
    padding: 5px 8px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  .cell {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  th {
    overflow: visible;
    position: relative;
  }
  .cell.wrap {
    white-space: normal;
    word-break: break-word;
    text-overflow: clip;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
  }
  tbody tr {
    vertical-align: top;
  }
  th {
    font-weight: 600;
    color: var(--text-muted);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    user-select: none;
  }
  th.sortable { cursor: pointer; }
  th.sortable:hover { background: var(--surface-hover); }
  .th-inner { display: flex; align-items: center; gap: 4px; }
  .sort-icon { color: var(--accent); }
  .filter-dropdown {
    margin-left: auto;
    position: relative;
  }
  .filter-btn {
    font-size: 11px;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 3px;
    cursor: pointer;
    padding: 1px 5px;
    line-height: 1.2;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    text-transform: none;
    letter-spacing: 0;
    font-weight: 400;
  }
  .filter-btn:hover { color: var(--accent); border-color: var(--accent); }
  .filter-btn.active { color: var(--accent); border-color: var(--accent); background: var(--surface-hover); }
  .filter-btn .badge {
    font-size: 10px;
    background: var(--accent);
    color: var(--bg);
    border-radius: 8px;
    padding: 0 4px;
    min-width: 12px;
    text-align: center;
  }
  .filter-list {
    position: absolute;
    background: var(--bg);
    border: 1px solid var(--border);
    padding: 6px;
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .filter-list label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    cursor: pointer;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text);
  }
  tr:hover { background: var(--surface-hover); }
  .cell.monospace { font-family: var(--font-mono, monospace); color: var(--accent); }
  .empty { text-align: center; color: var(--text-muted); padding: 24px; font-style: italic; }
</style>
