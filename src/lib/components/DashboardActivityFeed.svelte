<script lang="ts">
  import { onMount } from 'svelte';
  import { tStore, locale } from '$lib/i18n';
  import { readRecentActivity } from '$lib/api/tauri-commands';
  import { selectedRepoId, currentScreen } from '$lib/stores/ui';
  import { formatRelativeTime, nowTick } from '$lib/utils/time-format';
  import type { ActivityEvent } from '$lib/types';

  let events = $state<ActivityEvent[]>([]);
  let loading = $state(true);

  onMount(async () => {
    try {
      events = await readRecentActivity(10);
    } catch (e) {
      console.warn('readRecentActivity failed:', e);
      events = [];
    } finally {
      loading = false;
    }
  });

  function eventIcon(ev: ActivityEvent): string {
    if (ev.kind === 'repo_rename') return '🔄';
    if (ev.kind === 'sync_event') return '⟳';
    if (ev.kind === 'deploy_event') return '📦';
    if (ev.kind === 'task_event') {
      switch (ev.event_type) {
        case 'created': return '+'; case 'taken': return '→';
        case 'review': return '👀'; case 'done': return '✓'; case 'reopened': return '↻';
        default: return '·';
      }
    }
    if (ev.kind === 'bug_event') {
      switch (ev.event_type) {
        case 'created': return '+'; case 'taken': return '→';
        case 'entered_testing': return '🚧'; case 'confirmed': return '✓';
        case 'rejected': return '✗'; case 'reopened': return '↻';
        default: return '·';
      }
    }
    return '·';
  }

  function eventLabel(ev: ActivityEvent): string {
    if (ev.kind === 'repo_rename') return $tStore('dashboard.activity.repoRenamed' as any);
    // T-000103 Task 6: render v25 migration-conflict summary from `details` JSON.
    if (ev.kind === 'sync_event' && ev.sync_type === 'migration') {
      return renderMigrationLabel(ev);
    }
    const map: Record<string, string> = {
      created: 'dashboard.activity.bugCreated',
      taken: 'dashboard.activity.bugTaken',
      entered_testing: 'dashboard.activity.bugTesting',
      confirmed: 'dashboard.activity.bugConfirmed',
      rejected: 'dashboard.activity.bugRejected',
      reopened: 'dashboard.activity.bugReopened',
    };
    const k = map[ev.event_type];
    return k ? $tStore(k as any) : ev.event_type;
  }

  /** T-000103 Task 6: parse `details` JSON for migration events. */
  function renderMigrationLabel(ev: ActivityEvent): string {
    if (!ev.details) {
      return $tStore('timeline.event.sync.migrationNoDetail' as any);
    }
    try {
      const parsed = JSON.parse(ev.details);
      const conflicts = Array.isArray(parsed?.conflicts) ? parsed.conflicts : [];
      if (conflicts.length === 0) {
        return $tStore('timeline.event.sync.migrationNoDetail' as any);
      }
      const first = conflicts[0] ?? {};
      const firstKey = String(first.key ?? '?');
      const keptEnv = String(first.kept_env ?? '?');
      const key = conflicts.length === 1
        ? 'timeline.event.sync.migration'
        : 'timeline.event.sync.migrationMany';
      const tmpl = $tStore(key as any);
      return tmpl
        .replace('{count}', String(conflicts.length))
        .replace('{firstKey}', firstKey)
        .replace('{keptEnv}', keptEnv);
    } catch {
      return $tStore('timeline.event.sync.migrationNoDetail' as any);
    }
  }

  function go(ev: ActivityEvent) {
    if (ev.repo_id == null) return;  // portfolio-wide events have no repo
    selectedRepoId.set(ev.repo_id);
    currentScreen.set({ name: 'repo-detail' });
  }
</script>

<div class="activity">
  <div class="title">{$tStore('dashboard.activity.title' as any)}</div>
  {#if loading}
    <div class="muted">…</div>
  {:else if events.length === 0}
    <div class="muted empty">{$tStore('dashboard.activity.empty' as any)}</div>
  {:else}
    <ul class="rows">
      {#each events as ev, i (i)}
        <li>
          <button type="button" class="row" onclick={() => go(ev)}>
            <span class="icon">{eventIcon(ev)}</span>
            <span class="repo">{ev.repo_display_name ?? '?'}</span>
            {#if ev.bug_display_id}<span class="bug">/ {ev.bug_display_id}</span>{/if}
            <span class="label">· {eventLabel(ev)}</span>
            {#if ev.kind === 'repo_rename' && ev.old_canonical && ev.new_canonical}
              <span class="rename-detail">({ev.old_canonical} → {ev.new_canonical})</span>
            {/if}
            <span class="time">· {formatRelativeTime(ev.ts, $nowTick, $locale)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .activity {
    border-top: 1px solid var(--border);
    padding-top: 12px;
    margin-top: 6px;
  }
  .title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    margin-bottom: 6px;
  }
  .muted { color: var(--text-muted); font-size: 12px; padding: 4px 0; }
  .empty { font-style: italic; }
  .rows { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0; }
  .row {
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    padding: 5px 6px;
    font-size: 12px;
    color: var(--text-muted);
    cursor: pointer;
    border-radius: 4px;
    display: flex;
    align-items: baseline;
    gap: 4px;
    flex-wrap: wrap;
  }
  .row:hover { background: var(--surface-hover); color: var(--text); }
  .icon { font-size: 13px; flex-shrink: 0; width: 16px; text-align: center; }
  .repo { font-weight: 600; color: var(--accent); }
  .bug { font-family: var(--font-mono, monospace); font-size: 11px; }
  .label { color: var(--text-muted); }
  .rename-detail { color: var(--text-muted); font-size: 11px; opacity: 0.8; }
  .time { color: var(--text-muted); font-size: 11px; margin-left: auto; flex-shrink: 0; }
</style>
