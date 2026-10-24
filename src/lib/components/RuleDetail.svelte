<script lang="ts">
  import { tStore, tf } from '$lib/i18n';
  import type { RuleEvent, RuleInfo } from '$lib/types';
  import { buildRuleDiff } from '$lib/utils/rule-diff';
  import { GLOBAL_SECTION_FILE, formatRuleDate } from '$lib/utils/rules';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import RuleConflict from './RuleConflict.svelte';
  import RuleDiff from './RuleDiff.svelte';
  import RuleHistory from './RuleHistory.svelte';

  let {
    rule,
    repos,
    version,
    events,
    edited = $bindable(),
    dirty,
    busy,
    conflictGeneration,
    onSave,
    onReset,
    onResolve,
  }: {
    rule: RuleInfo;
    /** Repos a skill is delivered to. */
    repos: number;
    /** App version the canon belongs to; empty until known. */
    version: string;
    events: RuleEvent[];
    edited: string;
    dirty: boolean;
    busy: boolean;
    conflictGeneration: number;
    onSave: () => void;
    onReset: () => void;
    onResolve: (accept: string | null) => void;
  } = $props();

  type Tab = 'text' | 'diff' | 'history';
  let tab = $state<Tab>('text');
  let showResetConfirm = $state(false);

  const isGlobal = $derived(rule.file_name === GLOBAL_SECTION_FILE);
  // T-000224: the text comes from the owner's repo; Save and Reset are off.
  const followed = $derived(rule.follows_main !== null && rule.follows_main !== undefined);
  const title = $derived(isGlobal ? $tStore('rules.globalName') : rule.rule);

  // The diff compares the canon with the SAVED text, never the unsaved textarea.
  const diff = $derived(
    rule.canon === null
      ? { rows: [], added: 0, removed: 0 }
      : buildRuleDiff(rule.canon, rule.content)
  );

  const originText = $derived.by(() => {
    const o = rule.origin;
    if (!o) return rule.changed ? $tStore('rules.originUnknown') : $tStore('rules.originCanon');
    const when = formatRuleDate(o.created_at);
    switch (o.kind) {
      case 'taken': return tf('rules.originTaken', o.repo ?? '—', when);
      case 'saved': return tf('rules.originSaved', when);
      case 'reset': return tf('rules.originReset', when);
      case 'conflict_resolved': return tf('rules.originAccepted', o.repo ?? '—', when);
      default: return $tStore('rules.originUnknown');
    }
  });
  // Only a `taken` note is the marker's own line; other events carry a system note.
  const originNote = $derived(rule.origin?.kind === 'taken' ? rule.origin.note : null);

  function confirmReset() {
    showResetConfirm = false;
    onReset();
  }
</script>

<section class="detail">
  <div class="head">
    <span class="title" class:mono={!isGlobal}>{title}</span>
    {#if rule.changed}<span class="chip changed">{$tStore('rules.chipChanged')}</span>{/if}
    {#if rule.canon_moved}<span class="chip moved" title={$tStore('rules.chipCanonMovedHint')}>{$tStore('rules.chipCanonMoved')}</span>{/if}
    {#if rule.conflict}<span class="chip conflict">{$tStore('rules.chipConflict')}</span>{/if}
    {#if rule.follows_main}<span class="chip main">{$tStore('rules.chipFollowsMain')}</span>{/if}
    {#if version}<span class="canon">{tf('rules.canonOf', version)}</span>{/if}
  </div>

  <div class="origin">
    <span>{originText}</span>
    {#if originNote}
      <span class="origin-note">{tf('rules.originNote', originNote)}</span>
    {/if}
  </div>

  {#if rule.canon_moved && !rule.conflict}
    <div class="moved-line">
      <span title={$tStore('rules.chipCanonMovedHint')}>{$tStore('rules.canonMovedLine')}</span>
      <button type="button" class="ghost compare-btn" onclick={() => (tab = 'diff')}>{$tStore('rules.canonMovedCompare')}</button>
    </div>
  {/if}

  {#if rule.conflict}
    <RuleConflict {rule} {busy} generation={conflictGeneration} {onResolve} />
  {:else}
    <div class="tabs" role="tablist">
      <button type="button" role="tab" aria-selected={tab === 'text'} class:active={tab === 'text'} onclick={() => (tab = 'text')}>
        {$tStore('rules.tabText')}
      </button>
      <button type="button" role="tab" aria-selected={tab === 'diff'} class:active={tab === 'diff'} onclick={() => (tab = 'diff')}>
        {$tStore('rules.tabDiff')}
        <span class="add-count">+{diff.added}</span> <span class="del-count">−{diff.removed}</span>
      </button>
      <button type="button" role="tab" aria-selected={tab === 'history'} class:active={tab === 'history'} onclick={() => (tab = 'history')}>
        {$tStore('rules.tabHistory')} · {events.length}
      </button>
    </div>

    {#if tab === 'text'}
      <textarea class="editor" bind:value={edited} spellcheck="false" readonly={followed}></textarea>
      {#if followed}
        <div class="follows">{tf('rules.followsMain', rule.follows_main ?? '')}</div>
      {:else}
        <div class="warn">
          {isGlobal ? $tStore('rules.warnGlobal') : tf('rules.warnSkill', repos)}
        </div>
      {/if}
      <div class="actions">
        <button type="button" class="ghost reset-btn" disabled={followed || !rule.changed || busy} onclick={() => (showResetConfirm = true)}>
          {$tStore('rules.resetCanon')}
        </button>
        <button type="button" class="primary" disabled={followed || !dirty || busy} onclick={onSave}>
          {isGlobal ? $tStore('rules.save') : $tStore('rules.saveAndSend')}
        </button>
      </div>
    {:else if tab === 'diff'}
      <div class="pane">
        <RuleDiff {diff} legend emptyText={$tStore('rules.diffEmpty')} />
        <div class="footer">{$tStore('rules.diffReadOnly')}</div>
      </div>
    {:else}
      <div class="pane">
        <RuleHistory {events} />
      </div>
    {/if}
  {/if}
</section>

{#if showResetConfirm}
  <ConfirmDialog
    title={$tStore('rules.resetCanon')}
    message={dirty ? $tStore('rules.resetConfirmDirty') : $tStore('rules.resetConfirm')}
    onConfirm={confirmReset}
    onCancel={() => (showResetConfirm = false)}
  />
{/if}

<style>
  .detail { display: flex; flex-direction: column; gap: 8px; padding: 12px; min-width: 0; min-height: 0; overflow: hidden; }
  .head { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
  .title { font-size: 14px; font-weight: 600; }
  .title.mono { font-family: ui-monospace, "SF Mono", Menlo, monospace; }
  .canon { margin-left: auto; font-size: 11px; color: var(--text-muted); }
  .chip { font-size: 9px; padding: 1px 6px; border-radius: 6px; white-space: nowrap; }
  .chip.changed { background: rgba(234, 179, 8, 0.15); color: rgb(234, 179, 8); }
  /* Red tint of --danger (#ef4444). */
  .chip.conflict { background: rgba(239, 68, 68, 0.15); color: var(--danger); }
  .chip.moved { border: 1px solid var(--toast-warning); color: var(--toast-warning); padding: 0 6px; }
  .chip.main { border: 1px solid var(--accent); color: var(--accent); padding: 0 6px; }
  .follows { flex-shrink: 0; font-size: 11px; line-height: 1.4; color: var(--text-muted); }
  .origin { flex-shrink: 0; display: flex; flex-direction: column; gap: 2px; font-size: 12px; padding: 8px 10px; background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius); }
  .moved-line { flex-shrink: 0; display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 12px; padding: 6px 10px; border: 1px solid var(--toast-warning); border-radius: var(--radius); background: rgba(217, 119, 6, 0.12); }
  .compare-btn { font-size: 12px; padding: 3px 10px; flex-shrink: 0; }
  .origin-note { color: var(--text-muted); font-size: 11px; overflow-wrap: anywhere; }
  .tabs { display: flex; gap: 2px; border-bottom: 1px solid var(--border); flex-shrink: 0; }
  .tabs button { font-size: 12px; padding: 5px 12px; background: transparent; border: none; border-bottom: 2px solid transparent; border-radius: 0; color: var(--text-muted); cursor: pointer; }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { color: var(--text); border-bottom-color: var(--accent); }
  .add-count { color: #4ade80; }
  .del-count { color: #f87171; }
  .editor { flex: 1; min-height: 0; font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 12px; line-height: 1.5; padding: 10px; resize: none; }
  /* Amber tint of --toast-warning (#d97706). */
  .warn { flex-shrink: 0; font-size: 11px; line-height: 1.4; padding: 6px 10px; border: 1px solid var(--toast-warning); border-radius: var(--radius); background: rgba(217, 119, 6, 0.12); }
  .actions { display: flex; justify-content: space-between; gap: 8px; flex-shrink: 0; }
  .reset-btn { font-size: 12px; padding: 4px 10px; }
  .reset-btn:disabled { opacity: 0.3; cursor: not-allowed; }
  .pane { flex: 1; min-height: 0; overflow: auto; }
  .footer { font-size: 11px; color: var(--text-muted); margin-top: 8px; }
</style>
