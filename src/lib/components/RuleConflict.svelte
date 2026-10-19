<script lang="ts">
  import { untrack } from 'svelte';
  import { tStore, tf } from '$lib/i18n';
  import { addToast } from '$lib/stores/ui';
  import { getRuleConflict } from '$lib/api/tauri-commands';
  import type { RuleConflictSide, RuleInfo } from '$lib/types';
  import { buildRuleDiff } from '$lib/utils/rule-diff';
  import { formatRuleDate, shortRepoName } from '$lib/utils/rules';
  import RuleDiff from './RuleDiff.svelte';

  let {
    rule,
    busy,
    generation,
    onResolve,
  }: {
    rule: RuleInfo;
    busy: boolean;
    /** Bumped by the parent after a decision attempt so the sides are re-read. */
    generation: number;
    /** `null` rejects every side. */
    onResolve: (accept: string | null) => void;
  } = $props();

  let sides = $state<RuleConflictSide[]>([]);
  let loaded = $state(false);
  // A failed read is not "no live markers" — keep it apart and offer a retry.
  let loadError = $state<string | null>(null);

  async function load(name: string) {
    loadError = null;
    try {
      const result = await getRuleConflict(name);
      if (name === rule.rule) {
        sides = result;
        loaded = true;
      }
    } catch (err) {
      if (name === rule.rule) loadError = String(err);
    }
  }

  $effect(() => {
    const name = rule.rule;
    void generation;
    untrack(() => load(name));
  });

  // Each side's diff is against the rule's current (saved) text.
  const cards = $derived(
    sides.map((side) => ({ side, diff: buildRuleDiff(rule.content, side.content) }))
  );
</script>

<div class="conflict">
  <div class="banner">{$tStore('rules.conflictBanner')}</div>

  {#if loadError}
    <div class="load-error">
      <span>{loadError}</span>
      <button type="button" onclick={() => load(rule.rule)}>{$tStore('rules.retry')}</button>
    </div>
  {:else if !loaded}
    <div class="muted">{$tStore('common.loading')}</div>
  {:else if cards.length === 0}
    <div class="muted">{$tStore('rules.noSides')}</div>
  {:else}
    <div class="sides" style:grid-template-columns="repeat({cards.length}, minmax(0, 1fr))">
      {#each cards as { side, diff } (side.repo)}
        <section class="side">
          <code class="repo">{side.repo}</code>
          {#if side.marked_at}
            <div class="meta">{tf('rules.marked', formatRuleDate(side.marked_at))}</div>
          {/if}
          <div class="meta">
            {side.note ? tf('rules.originNote', side.note) : $tStore('rules.noteEmpty')}
          </div>
          <div class="vs">{$tStore('rules.diffVsCurrent')}</div>
          <div class="diff-box">
            <RuleDiff {diff} emptyText={$tStore('rules.diffEmpty')} />
          </div>
          <button type="button" class="primary" disabled={busy} onclick={() => onResolve(side.repo)}>
            {tf('rules.accept', shortRepoName(side.repo))}
          </button>
        </section>
      {/each}
    </div>
  {/if}

  <div class="reject">
    <button type="button" class="danger" disabled={busy} onclick={() => onResolve(null)}>
      {$tStore('rules.rejectAll')}
    </button>
    <span class="muted">{$tStore('rules.rejectHint')}</span>
  </div>
</div>

<style>
  .conflict { flex: 1; min-height: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 12px; }
  /* Red tint of --danger (#ef4444). */
  .banner { font-size: 12px; line-height: 1.5; padding: 8px 12px; border: 1px solid var(--danger); border-radius: var(--radius); background: rgba(239, 68, 68, 0.1); }
  .load-error { display: flex; align-items: center; gap: 12px; font-size: 12px; color: var(--danger); overflow-wrap: anywhere; }
  .muted { font-size: 11px; color: var(--text-muted); line-height: 1.4; }
  .sides { display: grid; gap: 12px; align-items: start; }
  .side { display: flex; flex-direction: column; gap: 6px; min-width: 0; padding: 10px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface); }
  .repo { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 12px; overflow-wrap: anywhere; }
  .meta { font-size: 11px; color: var(--text-muted); overflow-wrap: anywhere; }
  .vs { font-size: 11px; font-weight: 600; margin-top: 4px; }
  .diff-box { max-height: 320px; overflow: auto; }
  .side button { align-self: flex-start; margin-top: 4px; }
  .reject { display: flex; align-items: center; gap: 12px; }
  .reject button { flex-shrink: 0; }
</style>
