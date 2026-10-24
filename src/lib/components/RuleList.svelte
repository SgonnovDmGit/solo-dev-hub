<script module lang="ts">
  export type RuleFilter = 'all' | 'changed' | 'conflicts';
</script>

<script lang="ts">
  import { tStore, tf } from '$lib/i18n';
  import type { RuleInfo } from '$lib/types';
  import { GLOBAL_SECTION_FILE } from '$lib/utils/rules';

  let {
    rules,
    selected,
    filter = $bindable('all'),
    onSelect,
  }: {
    rules: RuleInfo[];
    selected: string | null;
    filter?: RuleFilter;
    onSelect: (rule: string) => void;
  } = $props();

  const FILTERS: { id: RuleFilter; label: 'rules.filterAll' | 'rules.filterChanged' | 'rules.filterConflicts' }[] = [
    { id: 'all', label: 'rules.filterAll' },
    { id: 'changed', label: 'rules.filterChanged' },
    { id: 'conflicts', label: 'rules.filterConflicts' },
  ];

  function passes(r: RuleInfo): boolean {
    if (filter === 'changed') return r.changed;
    if (filter === 'conflicts') return r.conflict;
    return true;
  }

  const globalRules = $derived(rules.filter((r) => r.file_name === GLOBAL_SECTION_FILE && passes(r)));
  const skillsAll = $derived(rules.filter((r) => r.file_name !== GLOBAL_SECTION_FILE));
  const skills = $derived(skillsAll.filter(passes));
</script>

<aside class="rule-list">
  <div class="filters">
    {#each FILTERS as f (f.id)}
      <button
        type="button"
        class="filter-chip"
        class:active={filter === f.id}
        onclick={() => (filter = f.id)}
      >
        {$tStore(f.label)}
      </button>
    {/each}
  </div>

  {#if globalRules.length > 0}
    <div class="sublabel">{$tStore('rules.groupGlobal')}</div>
    {#each globalRules as r (r.rule)}
      {@render item(r, $tStore('rules.globalName'), false)}
    {/each}
  {/if}

  {#if skills.length > 0}
    <div class="sublabel">{tf('rules.groupSkills', skillsAll.length)}</div>
    {#each skills as r (r.rule)}
      {@render item(r, r.rule, true)}
    {/each}
  {/if}
</aside>

{#snippet item(r: RuleInfo, name: string, mono: boolean)}
  <button
    type="button"
    class="rule-item"
    class:active={selected === r.rule}
    class:mono
    onclick={() => onSelect(r.rule)}
  >
    <span class="rule-name">{name}</span>
    <span class="chips">
      {#if r.changed}<span class="chip changed">{$tStore('rules.chipChanged')}</span>{/if}
      {#if r.canon_moved}<span class="chip moved" title={$tStore('rules.chipCanonMovedHint')}>{$tStore('rules.chipCanonMoved')}</span>{/if}
      {#if r.conflict}<span class="chip conflict">{$tStore('rules.chipConflict')}</span>{/if}
      {#if r.follows_main}<span class="chip main" title={r.follows_main}>{$tStore('rules.chipFollowsMain')}</span>{/if}
    </span>
  </button>
{/snippet}

<style>
  .rule-list { display: flex; flex-direction: column; border-right: 1px solid var(--border); background: var(--bg); padding: 8px; gap: 3px; overflow-y: auto; min-height: 0; }
  .filters { display: flex; gap: 4px; margin-bottom: 6px; }
  .filter-chip { font-size: 11px; padding: 2px 8px; border-radius: 10px; border: 1px solid var(--border); background: transparent; color: var(--text-muted); cursor: pointer; }
  .filter-chip:hover { background: var(--surface); }
  .filter-chip.active { border-color: var(--accent); color: var(--accent); background: var(--surface); }
  .sublabel { font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); margin: 6px 0 2px; padding: 0 4px; }
  .rule-item { display: flex; align-items: center; justify-content: space-between; gap: 6px; padding: 5px 8px; font-size: 12px; background: transparent; border: 1px solid transparent; border-radius: 3px; color: var(--text); cursor: pointer; text-align: left; }
  .rule-item.mono { font-family: monospace; }
  .rule-item:hover { background: var(--surface); }
  .rule-item.active { background: var(--surface); border-color: var(--accent); }
  .rule-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chips { display: flex; gap: 4px; flex-shrink: 0; }
  .chip { font-size: 9px; padding: 1px 6px; border-radius: 6px; white-space: nowrap; font-family: sans-serif; flex-shrink: 0; }
  .chip.changed { background: rgba(234, 179, 8, 0.15); color: rgb(234, 179, 8); }
  /* The canon moved under a fork: outlined amber, distinct from the filled "changed". */
  .chip.moved { border: 1px solid var(--toast-warning); color: var(--toast-warning); padding: 0 6px; }
  /* Red tint of --danger (#ef4444). */
  .chip.conflict { background: rgba(239, 68, 68, 0.15); color: var(--danger); }
  /* The owner's canon: outlined in the accent, like the pill of the shared-repo board. */
  .chip.main { border: 1px solid var(--accent); color: var(--accent); padding: 0 6px; }
</style>
