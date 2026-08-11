<script lang="ts">
  import { onMount } from 'svelte';
  import { tStore } from '$lib/i18n';
  import { helpNav } from '$lib/help/sections';
  import HelpQuickStart from './help/HelpQuickStart.svelte';
  import HelpProjectSetup from './help/HelpProjectSetup.svelte';
  import HelpBugWorkflow from './help/HelpBugWorkflow.svelte';

  let search = $state('');
  let current = $state(helpNav[0].items[0].anchor);
  let docEl: HTMLDivElement | undefined = $state();

  const filteredNav = $derived.by(() => {
    const q = search.trim().toLowerCase();
    if (!q) return helpNav;
    // Search section CONTENT, not just the table of contents: match the group
    // title + the item label + the section body text (item.searchKeys).
    return helpNav
      .map((group) => ({
        ...group,
        items: group.items.filter((item) =>
          [group.titleKey, item.labelKey, ...item.searchKeys]
            .map((k) => $tStore(k).toLowerCase())
            .join(' ')
            .includes(q),
        ),
      }))
      .filter((group) => group.items.length > 0);
  });

  function goTo(anchor: string) {
    current = anchor;
    const el = document.getElementById(anchor);
    el?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  onMount(() => {
    current = helpNav[0].items[0].anchor;
  });
</script>

<div class="help2">
  <nav class="help-nav">
    <input
      class="help-search"
      type="text"
      placeholder={$tStore('help.searchPlaceholder')}
      bind:value={search}
    />
    {#if filteredNav.length === 0}
      <div class="search-empty">{$tStore('help.searchEmpty')}</div>
    {/if}
    {#each filteredNav as group (group.titleKey)}
      <div class="grp">
        <div class="grp-h"><span aria-hidden="true">{group.icon}</span>{$tStore(group.titleKey)}</div>
        {#each group.items as item (item.anchor)}
          <a
            href={`#${item.anchor}`}
            class:cur={current === item.anchor}
            onclick={(e) => {
              e.preventDefault();
              goTo(item.anchor);
            }}
          >
            {$tStore(item.labelKey)}
          </a>
        {/each}
      </div>
    {/each}
  </nav>

  <div class="help-doc doc" bind:this={docEl}>
    <HelpQuickStart />
    <HelpProjectSetup />
    <HelpBugWorkflow />
  </div>
</div>

<style>
  .help2 {
    display: flex;
    height: 100%;
  }

  .help-nav {
    width: 230px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
    background: var(--surface);
    overflow: auto;
    padding: 0.8rem 0.5rem;
  }

  .help-search {
    width: 100%;
    padding: 0.4rem 0.55rem;
    margin-bottom: 0.7rem;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    font: inherit;
    font-size: 0.82rem;
    box-sizing: border-box;
  }

  .search-empty {
    padding: 0.4rem 0.5rem;
    color: var(--text-muted);
    font-size: 0.82rem;
  }

  .grp {
    margin-bottom: 0.9rem;
  }

  .grp-h {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
    padding: 0.2rem 0.5rem;
    font-weight: 700;
  }

  .help-nav a {
    display: block;
    text-decoration: none;
    color: var(--text);
    font-size: 0.86rem;
    padding: 0.36rem 0.5rem 0.36rem 0.9rem;
    border-radius: 5px;
    cursor: pointer;
    border-left: 2px solid transparent;
  }

  .help-nav a:hover {
    background: var(--surface-hover);
  }

  .help-nav a.cur {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-left-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  .help-doc {
    flex: 1;
    min-width: 0;
    overflow: auto;
    padding: 1.6rem 2rem;
    max-width: 760px;
  }

  .help-doc :global(h1) {
    font-size: 1.4rem;
    margin: 0 0 0.3rem;
  }

  .help-doc :global(.lead) {
    color: var(--text-muted);
    font-size: 0.94rem;
    margin: 0 0 1.4rem;
    line-height: 1.5;
  }

  .help-doc :global(section) {
    margin-bottom: 1.8rem;
    scroll-margin-top: 1rem;
  }

  .help-doc :global(h3) {
    font-size: 1.02rem;
    margin: 0 0 0.5rem;
    padding-bottom: 0.3rem;
    border-bottom: 1px solid var(--border);
  }

  .help-doc :global(p) {
    line-height: 1.6;
    font-size: 0.9rem;
    margin: 0.4rem 0;
  }

  .help-doc :global(ol),
  .help-doc :global(ul) {
    line-height: 1.6;
    font-size: 0.9rem;
    padding-left: 1.3rem;
    margin: 0.4rem 0;
  }

  .help-doc :global(li) {
    margin: 0.3rem 0;
  }

  .help-doc :global(code) {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.05rem 0.35rem;
    font-size: 0.82em;
  }

  .help-doc :global(strong) {
    color: var(--text);
  }

  .help-doc :global(.callout) {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: 6px;
    padding: 0.7rem 0.9rem;
    font-size: 0.85rem;
    line-height: 1.55;
    margin: 0.8rem 0;
  }

  .help-doc :global(.steps) {
    counter-reset: s;
    list-style: none;
    padding: 0;
  }

  .help-doc :global(.steps li) {
    position: relative;
    padding: 0.5rem 0 0.5rem 2.2rem;
    border-bottom: 1px dashed var(--border);
    margin: 0;
  }

  .help-doc :global(.steps li:last-child) {
    border-bottom: 0;
  }

  .help-doc :global(.steps li::before) {
    counter-increment: s;
    content: counter(s);
    position: absolute;
    left: 0;
    top: 0.5rem;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
    display: grid;
    place-items: center;
    font-size: 0.8rem;
    font-weight: 700;
  }

  .help-doc :global(.flow) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    margin: 0.6rem 0;
  }

  .help-doc :global(.pill) {
    border: 1px solid var(--border);
    border-radius: 20px;
    padding: 0.2rem 0.6rem;
    font-size: 0.78rem;
    background: var(--surface);
  }

  .help-doc :global(.pill.created) {
    border-color: var(--text-muted);
  }

  .help-doc :global(.pill.prog) {
    border-color: var(--accent);
    color: var(--accent);
  }

  .help-doc :global(.pill.test) {
    border-color: var(--accent-hover);
    color: var(--accent-hover);
  }

  .help-doc :global(.pill.ok) {
    border-color: #16a34a;
    color: #16a34a;
  }

  .help-doc :global(.pill.rej) {
    border-color: var(--danger);
    color: var(--danger);
  }

  .help-doc :global(.arw) {
    color: var(--text-muted);
  }

  .help-doc :global(table.edit) {
    width: 100%;
    border-collapse: collapse;
    margin: 0.6rem 0;
    font-size: 0.84rem;
  }

  .help-doc :global(table.edit th),
  .help-doc :global(table.edit td) {
    border: 1px solid var(--border);
    padding: 0.4rem 0.55rem;
    text-align: left;
  }

  .help-doc :global(table.edit th) {
    background: var(--surface);
    font-weight: 600;
  }

  .help-doc :global(.yes) {
    color: #16a34a;
    font-weight: 700;
  }

  .help-doc :global(.no) {
    color: var(--danger);
    font-weight: 700;
  }
</style>
