<script lang="ts">
  import type { ActivityEvent } from '$lib/types';
  import { tStore } from '$lib/i18n';
  import { readTimeline } from '$lib/api/tauri-commands';
  import { currentScreen, timelineInitialRepoIds } from '$lib/stores/ui';
  import { allRepos } from '$lib/stores/repos';

  interface Props {
    scope: 'repo' | 'project';
    scopeId: number;          // repo.id (scope='repo') или project.id (scope='project')
    limit?: number;           // default 10
  }
  let { scope, scopeId, limit = 10 }: Props = $props();

  let events = $state<ActivityEvent[]>([]);
  let loading = $state(true);

  $effect(() => {
    void scope; void scopeId;
    fetchEvents();
  });

  async function fetchEvents() {
    loading = true;
    try {
      const filter = {
        start_date: '2000-01-01',
        end_date: new Date().toISOString().slice(0, 10),
        event_kinds: undefined,
        project_ids: scope === 'project' ? [scopeId] : undefined,
        repo_ids: scope === 'repo' ? [scopeId] : undefined,
        search: undefined,
      };
      events = await readTimeline(filter, 0, limit);
    } catch {
      events = [];
    } finally {
      loading = false;
    }
  }

  function viewAll() {
    if (scope === 'repo') {
      timelineInitialRepoIds.set([scopeId]);
    } else {
      const repoIds = ($allRepos ?? [])
        .filter((r) => r.project_id === scopeId)
        .map((r) => r.id);
      timelineInitialRepoIds.set(repoIds);
    }
    currentScreen.set({ name: 'timeline' });
  }

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
        case 'created': return '+';
        case 'taken': return '→';
        case 'review': return '👀';
        case 'done': return '✓';
        case 'reopened': return '↻';
      }
    }
    if (ev.kind === 'bug_event') {
      switch (ev.event_type) {
        case 'created': return '+';
        case 'taken': return '→';
        case 'entered_testing': return '🚧';
        case 'confirmed': return '✓';
        case 'rejected': return '✗';
        case 'reopened': return '↻';
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

  function displayId(ev: ActivityEvent): string | null {
    return ev.bug_display_id ?? ev.task_display_id ?? null;
  }
</script>

<div class="stats-card">
  <div class="section-title">
    📅 {$tStore('stats.summary.recentActivityTitle' as any)}
    <button class="view-all" onclick={viewAll}>
      {scope === 'repo'
        ? $tStore('stats.summary.recentActivityViewAllRepo' as any)
        : $tStore('stats.summary.recentActivityViewAllProject' as any)} →
    </button>
  </div>

  {#if loading}
    <div class="empty">{$tStore('stats.summary.recentActivityLoading' as any)}</div>
  {:else if events.length === 0}
    <div class="empty">{$tStore('stats.summary.recentActivityEmpty' as any)}</div>
  {:else}
    {#each grouped as group}
      <div class="day-group">
        <div class="day-header">{group.day}</div>
        {#each group.events as ev}
          <div class="event-row">
            <span class="icon">{eventIcon(ev)}</span>
            <span class="time">{timeOfDay(ev.ts)}</span>
            <span class="label">
              {#if displayId(ev)}<span class="display-id">{displayId(ev)}</span>&nbsp;{/if}
              {eventLabel(ev)}
            </span>
          </div>
        {/each}
      </div>
    {/each}
  {/if}
</div>

<style>
  .stats-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 16px;
    margin-bottom: 14px;
  }
  .section-title {
    font-size: 12px;
    font-weight: 600;
    margin-bottom: 10px;
    color: var(--text);
    display: flex;
    align-items: center;
  }
  .view-all {
    background: transparent;
    border: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 11px;
    font-weight: 500;
    padding: 0;
    margin-left: auto;
  }
  .view-all:hover { text-decoration: underline; }

  .day-group { margin-bottom: 10px; }
  .day-group:last-child { margin-bottom: 0; }
  .day-header {
    font-size: 10px;
    color: var(--text-muted);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 4px;
    padding-left: 4px;
  }
  .event-row {
    display: grid;
    grid-template-columns: 18px 36px 1fr;
    gap: 8px;
    padding: 3px 4px;
    font-size: 11px;
    align-items: center;
  }
  .event-row .icon { color: var(--text-muted); text-align: center; }
  .event-row .time { color: var(--text-muted); font-variant-numeric: tabular-nums; font-size: 10px; }
  .event-row .label { color: var(--text); }
  .display-id { color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .empty { font-size: 11px; color: var(--text-muted); padding: 8px 0; font-style: italic; }
</style>
