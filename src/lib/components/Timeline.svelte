<script lang="ts">
  import { onMount } from 'svelte';
  import { tStore, locale } from '$lib/i18n';
  import { readTimeline } from '$lib/api/tauri-commands';
  import { allRepos } from '$lib/stores/repos';
  import { selectedRepoId, currentScreen, timelineInitialRepoIds } from '$lib/stores/ui';
  import { get } from 'svelte/store';
  import { getDisplayName } from '$lib/types';
  import type { ActivityEvent, TimelineFilter } from '$lib/types';

  const PAGE_SIZE = 50;
  const ALL_KINDS = ['bug_event','task_event','repo_rename','sync_event','deploy_event'];

  function todayStr(): string { return new Date().toISOString().slice(0, 10); }
  function firstOfMonthStr(): string {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-01`;
  }

  let startDate = $state(firstOfMonthStr());
  let endDate = $state(todayStr());
  let selectedKinds = $state<string[]>([...ALL_KINDS]);
  let selectedRepos = $state<number[]>([]);
  let searchText = $state('');

  let events = $state<ActivityEvent[]>([]);
  let offset = $state(0);
  let loading = $state(false);
  let hasMore = $state(true);

  function buildFilter(): TimelineFilter {
    return {
      start_date: startDate,
      end_date: endDate,
      event_kinds: selectedKinds.length === ALL_KINDS.length || selectedKinds.length === 0 ? undefined : selectedKinds,
      project_ids: undefined,
      repo_ids: selectedRepos.length === 0 ? undefined : selectedRepos,
      search: searchText.trim() || undefined,
    };
  }

  async function loadFirstPage() {
    loading = true;
    offset = 0;
    try {
      const r = await readTimeline(buildFilter(), 0, PAGE_SIZE);
      events = r;
      offset = r.length;
      hasMore = r.length === PAGE_SIZE;
    } finally {
      loading = false;
    }
  }

  async function loadMore() {
    if (loading || !hasMore) return;
    loading = true;
    try {
      const r = await readTimeline(buildFilter(), offset, PAGE_SIZE);
      events = [...events, ...r];
      offset += r.length;
      hasMore = r.length === PAGE_SIZE;
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    // v0.22.0 (T-000056): consume transient repo-filter from RecentActivityFeed
    // deep-link. The $effect below will pick up the assignment and fire
    // loadFirstPage once — no explicit call needed (M3 review-fix: previous
    // versions called loadFirstPage here AND let the effect fire, causing
    // a duplicate fetch on every deep-link mount).
    const initRepoIds = get(timelineInitialRepoIds);
    if (initRepoIds && initRepoIds.length > 0) {
      selectedRepos = initRepoIds;
      timelineInitialRepoIds.set(null);  // one-shot: clear after consume
    }
  });

  let filterChangeTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    void startDate; void endDate; void selectedKinds; void selectedRepos; void searchText;
    if (filterChangeTimer) clearTimeout(filterChangeTimer);
    filterChangeTimer = setTimeout(loadFirstPage, 200);
  });

  const grouped = $derived.by(() => {
    const out: { day: string; events: ActivityEvent[] }[] = [];
    let currentDay: string | null = null;
    let currentList: ActivityEvent[] = [];
    for (const ev of events) {
      const day = ev.ts.slice(0, 10);
      if (day !== currentDay) {
        if (currentList.length) out.push({ day: currentDay!, events: currentList });
        currentDay = day;
        currentList = [];
      }
      currentList.push(ev);
    }
    if (currentList.length) out.push({ day: currentDay!, events: currentList });
    return out;
  });

  function eventIcon(ev: ActivityEvent): string {
    if (ev.kind === 'repo_rename') return '🔄';
    if (ev.kind === 'sync_event') return '⟳';
    if (ev.kind === 'deploy_event') return '📦';
    if (ev.kind === 'task_event') {
      switch (ev.event_type) {
        case 'created': return '+'; case 'taken': return '→';
        case 'review': return '👀'; case 'done': return '✓'; case 'reopened': return '↻';
      }
    }
    if (ev.kind === 'bug_event') {
      switch (ev.event_type) {
        case 'created': return '+'; case 'taken': return '→';
        case 'entered_testing': return '🚧'; case 'confirmed': return '✓';
        case 'rejected': return '✗'; case 'reopened': return '↻';
      }
    }
    return '·';
  }

  function eventLabel(ev: ActivityEvent): string {
    const family = ev.kind === 'repo_rename' ? 'repo_rename'
      : ev.kind === 'sync_event' ? 'sync'
      : ev.kind === 'deploy_event' ? 'deploy'
      : ev.kind === 'task_event' ? 'task'
      : 'bug';
    const key = `timeline.event.${family}.${ev.event_type}`;
    const label = $tStore(key as any);
    return label && label !== key ? label : ev.event_type;
  }

  function timeOfDay(iso: string): string {
    return iso.length >= 16 ? iso.slice(11, 16) : '';
  }

  function go(ev: ActivityEvent) {
    if (ev.repo_id != null) {
      selectedRepoId.set(ev.repo_id);
      currentScreen.set({ name: 'repo-detail' });
    }
  }

  function toggleKind(kind: string) {
    if (selectedKinds.includes(kind)) selectedKinds = selectedKinds.filter((k) => k !== kind);
    else selectedKinds = [...selectedKinds, kind];
  }
</script>

<div class="timeline">
  <div class="header">
    <h2>{$tStore('timeline.title' as any)}</h2>
    <button class="refresh-btn" onclick={loadFirstPage}>↻</button>
  </div>

  <div class="toolbar">
    <input type="date" bind:value={startDate} />
    <span>—</span>
    <input type="date" bind:value={endDate} />

    <details class="dropdown">
      <summary>{$tStore('timeline.filter.events' as any)} ({selectedKinds.length}/{ALL_KINDS.length})</summary>
      <div class="dropdown-list">
        {#each ALL_KINDS as kind}
          <label>
            <input type="checkbox" checked={selectedKinds.includes(kind)} onchange={() => toggleKind(kind)} />
            {$tStore(`timeline.kind.${kind}` as any)}
          </label>
        {/each}
      </div>
    </details>

    <details class="dropdown">
      <summary>{$tStore('timeline.filter.repos' as any)} ({selectedRepos.length || $tStore('timeline.filter.all' as any)})</summary>
      <div class="dropdown-list">
        {#each $allRepos as r}
          <label>
            <input type="checkbox"
              checked={selectedRepos.includes(r.id)}
              onchange={() => selectedRepos = selectedRepos.includes(r.id) ? selectedRepos.filter((x) => x !== r.id) : [...selectedRepos, r.id]} />
            {getDisplayName(r)}
          </label>
        {/each}
      </div>
    </details>

    <input type="search" placeholder={$tStore('timeline.filter.search' as any)} bind:value={searchText} class="search" />
  </div>

  <div class="body">
    {#if loading && events.length === 0}
      <p class="muted">{$tStore('common.loading' as any)}</p>
    {:else if events.length === 0}
      <p class="empty">{$tStore('timeline.empty' as any)}</p>
    {:else}
      {#each grouped as group}
        <h3 class="day-header">## {group.day}</h3>
        <ul>
          {#each group.events as ev, i (`${group.day}-${i}-${ev.ts}`)}
            <li>
              <button type="button" class="row" onclick={() => go(ev)}>
                <span class="icon">{eventIcon(ev)}</span>
                <span class="repo">{ev.repo_display_name ?? '?'}</span>
                {#if ev.bug_display_id}<span class="id">/ {ev.bug_display_id}</span>{/if}
                {#if ev.task_display_id}<span class="id">/ {ev.task_display_id}</span>{/if}
                <span class="label">· {eventLabel(ev)}</span>
                {#if ev.kind === 'repo_rename' && ev.old_canonical}<span class="meta">({ev.old_canonical} → {ev.new_canonical})</span>{/if}
                {#if ev.change_count != null && ev.change_count > 0}<span class="meta">[{ev.change_count}]</span>{/if}
                <span class="time">· {timeOfDay(ev.ts)}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/each}
      {#if hasMore}
        <div class="load-more">
          <button onclick={loadMore} disabled={loading}>
            {loading ? $tStore('common.loading' as any) : $tStore('timeline.loadMore' as any).replace('{0}', String(PAGE_SIZE))}
          </button>
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .timeline { padding: 16px; height: 100%; display: flex; flex-direction: column; overflow: hidden; }
  .header { display: flex; justify-content: space-between; align-items: center; padding-bottom: 8px; flex-shrink: 0; }
  h2 { margin: 0; font-size: 16px; font-weight: 700; }
  .refresh-btn { background: none; border: 1px solid var(--border); border-radius: 4px; padding: 2px 10px; cursor: pointer; }
  .toolbar { display: flex; gap: 8px; align-items: center; padding: 6px 0; flex-wrap: wrap; flex-shrink: 0; border-bottom: 1px solid var(--border); }
  .toolbar input[type="date"] { font-size: 12px; padding: 3px 6px; }
  .dropdown summary { cursor: pointer; font-size: 12px; padding: 3px 8px; border: 1px solid var(--border); border-radius: 4px; list-style: none; user-select: none; }
  .dropdown-list { position: absolute; background: var(--bg); border: 1px solid var(--border); padding: 6px; max-height: 300px; overflow-y: auto; z-index: 5; display: flex; flex-direction: column; gap: 2px; min-width: 200px; }
  .dropdown-list label { display: flex; gap: 4px; font-size: 12px; cursor: pointer; }
  .search { flex: 1; min-width: 100px; max-width: 300px; padding: 4px 8px; font-size: 12px; }
  .body { flex: 1; overflow-y: auto; padding: 8px 0; }
  .muted { color: var(--text-muted); padding: 24px; text-align: center; }
  .empty { color: var(--text-muted); padding: 24px; font-style: italic; text-align: center; }
  .day-header { position: sticky; top: 0; background: var(--bg); margin: 16px 0 4px; font-size: 12px; font-weight: 600; padding: 4px 0; border-bottom: 1px solid var(--border); z-index: 1; }
  ul { list-style: none; padding: 0; margin: 0; }
  .row { width: 100%; text-align: left; background: none; border: none; padding: 4px 8px; font-size: 12px; color: var(--text-muted); cursor: pointer; border-radius: 3px; display: flex; gap: 4px; flex-wrap: wrap; align-items: baseline; }
  .row:hover { background: var(--surface-hover); color: var(--text); }
  .icon { width: 16px; text-align: center; }
  .repo { font-weight: 600; color: var(--accent); }
  .id { font-family: var(--font-mono, monospace); font-size: 11px; }
  .label { color: var(--text-muted); }
  .meta { color: var(--text-muted); font-size: 11px; opacity: 0.8; }
  .time { color: var(--text-muted); font-size: 11px; margin-left: auto; }
  .load-more { padding: 12px; text-align: center; }
  .load-more button { padding: 6px 18px; font-size: 12px; background: var(--surface); border: 1px solid var(--border); border-radius: 4px; cursor: pointer; }
  .load-more button:hover:not(:disabled) { background: var(--surface-hover); }
</style>
