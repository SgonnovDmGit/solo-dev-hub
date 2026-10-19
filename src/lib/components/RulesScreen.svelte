<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { getVersion } from '@tauri-apps/api/app';
  import { currentScreen, addToast } from '$lib/stores/ui';
  import { aiRulesLastSyncAt } from '$lib/stores/settings';
  import { tStore } from '$lib/i18n';
  import {
    listRules,
    listRuleEvents,
    saveTemplateFile,
    resetTemplateFile,
    resolveRuleConflict,
  } from '$lib/api/tauri-commands';
  import type { RuleEvent, RulesOverview } from '$lib/types';
  import { formatRelativeTime, nowTick } from '$lib/utils/time-format';
  import { GLOBAL_SECTION_FILE } from '$lib/utils/rules';
  import { ruleSummary } from '$lib/utils/rules-view';
  import { reportRuleOutcome, syncRulesFromApp } from '$lib/utils/rules-actions';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import RuleList, { type RuleFilter } from './RuleList.svelte';
  import RuleDetail from './RuleDetail.svelte';

  const LANGUAGE_KEY = '_global';

  let overview = $state<RulesOverview | null>(null);
  let selectedRule = $state<string | null>(null);
  let filter = $state<RuleFilter>('all');
  let edited = $state('');
  let events = $state<RuleEvent[]>([]);
  let version = $state('');
  // The first read of the overview failed — show it with a retry, not "Loading…" forever.
  let loadError = $state<string | null>(null);
  let busy = $state(false);
  let syncing = $state(false);
  let conflictGeneration = $state(0);
  // An action waiting for the "leave without saving?" answer.
  let pendingLeave = $state<(() => void) | null>(null);

  const selected = $derived(overview?.rules.find((r) => r.rule === selectedRule) ?? null);
  const dirty = $derived(selected !== null && !selected.conflict && edited !== selected.content);
  const syncBlocked = $derived(dirty && selected?.file_name === GLOBAL_SECTION_FILE);
  const lastSync = $derived(
    $aiRulesLastSyncAt
      ? $tStore('settings.aiRulesLastSync').replace('{time}', formatRelativeTime($aiRulesLastSyncAt, $nowTick))
      : $tStore('settings.aiRulesNeverSynced')
  );

  async function loadEvents(rule: string) {
    try {
      const list = await listRuleEvents(rule);
      if (rule === selectedRule) events = list;
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  /** Re-read the overview. An unsaved edit survives unless `resetEdit` is set. */
  async function reload(resetEdit = false) {
    try {
      const wasDirty = dirty;
      const ov = await listRules();
      overview = ov;
      const next = ov.rules.find((r) => r.rule === selectedRule);
      if (!next) {
        const first = ov.rules[0] ?? null;
        selectedRule = first?.rule ?? null;
        edited = first?.content ?? '';
        events = [];
      } else if (resetEdit || !wasDirty) {
        edited = next.content;
      }
      if (selectedRule) await loadEvents(selectedRule);
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function select(rule: string) {
    selectedRule = rule;
    edited = overview?.rules.find((r) => r.rule === rule)?.content ?? '';
    events = [];
    void loadEvents(rule);
  }

  /** Run `action` now, or after the owner confirms dropping an unsaved edit. */
  function guard(action: () => void) {
    if (dirty) pendingLeave = action;
    else action();
  }

  function requestSelect(rule: string) {
    if (rule === selectedRule) return;
    guard(() => select(rule));
  }

  function back() {
    guard(() => currentScreen.set({ name: 'settings' }));
  }

  function confirmLeave() {
    const action = pendingLeave;
    pendingLeave = null;
    action?.();
  }

  async function save() {
    if (!selected || busy) return;
    busy = true;
    try {
      const outcome = await saveTemplateFile(LANGUAGE_KEY, selected.file_name, edited);
      reportRuleOutcome(outcome, 'toast.templateSaved');
      await reload(true);
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      busy = false;
    }
  }

  async function reset() {
    if (!selected || busy) return;
    busy = true;
    try {
      const outcome = await resetTemplateFile(LANGUAGE_KEY, selected.file_name);
      reportRuleOutcome(outcome, 'toast.templateReset');
      await reload(true);
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      busy = false;
    }
  }

  async function resolve(accept: string | null) {
    if (!selected || busy) return;
    busy = true;
    try {
      const outcome = await resolveRuleConflict(selected.rule, accept);
      reportRuleOutcome(outcome);
    } catch (err) {
      const text = String(err);
      if (text.includes('rule_conflict_side_gone')) {
        addToast($tStore('rules.sideGone'), 'info');
      } else if (text.includes('rule_conflict_gone')) {
        addToast($tStore('rules.conflictGone'), 'info');
      } else {
        addToast(text, 'error');
      }
    } finally {
      // Either way the screen re-reads: the conflict may be closed or its sides changed.
      conflictGeneration++;
      await reload(true);
      busy = false;
    }
  }

  async function sync() {
    if (syncing || syncBlocked) return;
    syncing = true;
    try {
      if (await syncRulesFromApp()) await reload();
    } finally {
      syncing = false;
    }
  }

  onMount(async () => {
    try {
      version = await getVersion();
    } catch (err) {
      console.warn('getVersion failed', err);
    }
    await loadInitial();
  });

  // One-shot snapshot of the requested rule — not reactive state.
  const wantedRule = untrack(() => get(currentScreen).params?.rule);

  async function loadInitial() {
    loadError = null;
    try {
      const ov = await listRules();
      overview = ov;
      const initial = ov.rules.find((r) => r.rule === wantedRule) ?? ov.rules[0] ?? null;
      if (initial) select(initial.rule);
    } catch (err) {
      loadError = String(err);
    }
  }
</script>

<div class="rules-screen">
  <div class="header">
    <button class="ghost back-btn" onclick={back} type="button">
      {$tStore('settings.back')}
    </button>
    <h2>{$tStore('rules.title')}</h2>
    {#if overview}
      <span class="summary">{ruleSummary(overview)}</span>
    {/if}
    <div class="header-actions">
      <span class="sync-status">{lastSync}</span>
      <button
        class="sync-btn"
        onclick={sync}
        disabled={syncBlocked || syncing}
        title={syncBlocked ? $tStore('settings.aiRulesSyncTooltipDirty') : undefined}
        type="button"
      >
        {syncing ? '...' : $tStore('settings.aiRulesSync')}
      </button>
    </div>
  </div>

  {#if loadError}
    <div class="loading error">
      <div>{loadError}</div>
      <button type="button" onclick={loadInitial}>{$tStore('rules.retry')}</button>
    </div>
  {:else if !overview}
    <div class="loading">{$tStore('common.loading')}</div>
  {:else}
    <div class="body">
      <RuleList rules={overview.rules} selected={selectedRule} bind:filter onSelect={requestSelect} />
      <div class="right">
        {#if selected}
          {#key selected.rule}
            <RuleDetail
              rule={selected}
              repos={overview.repos}
              {version}
              {events}
              bind:edited
              {dirty}
              {busy}
              {conflictGeneration}
              onSave={save}
              onReset={reset}
              onResolve={resolve}
            />
          {/key}
        {/if}
      </div>
    </div>
  {/if}
</div>

{#if pendingLeave}
  <ConfirmDialog
    title={$tStore('rules.title')}
    message={$tStore('rules.unsavedConfirm')}
    onConfirm={confirmLeave}
    onCancel={() => (pendingLeave = null)}
  />
{/if}

<style>
  .rules-screen { height: 100%; display: flex; flex-direction: column; overflow: hidden; }
  .header { flex-shrink: 0; padding: 12px 24px; border-bottom: 1px solid var(--border); display: flex; align-items: center; gap: 16px; }
  .back-btn { font-size: 12px; padding: 3px 8px; color: var(--text-muted); }
  .back-btn:hover { color: var(--accent); }
  h2 { font-size: 18px; font-weight: 700; margin: 0; }
  .summary { font-size: 12px; color: var(--text-muted); min-width: 0; }
  .header-actions { margin-left: auto; display: flex; align-items: center; gap: 12px; flex-shrink: 0; }
  .sync-status { font-size: 11px; color: var(--text-muted); font-style: italic; }
  .sync-btn { font-size: 12px; padding: 5px 14px; border-radius: 4px; border: 1px solid var(--accent); background: transparent; color: var(--accent); cursor: pointer; }
  .sync-btn:not(:disabled):hover { background: var(--accent); color: white; }
  .sync-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .body { flex: 1; min-height: 0; display: grid; grid-template-columns: 270px 1fr; overflow: hidden; }
  /* Grid child: the detail inside is a flex column that must be able to shrink. */
  .right { display: flex; flex-direction: column; min-width: 0; min-height: 0; overflow: hidden; }
  .right > :global(.detail) { flex: 1; }
  .loading { padding: 40px; color: var(--text-muted); text-align: center; }
  .loading.error { color: var(--danger); display: flex; flex-direction: column; align-items: center; gap: 12px; overflow-wrap: anywhere; }
</style>
