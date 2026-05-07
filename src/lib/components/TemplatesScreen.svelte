<script lang="ts">
  import { currentScreen } from '$lib/stores/ui';
  import { addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import { listTemplateLanguages } from '$lib/api/tauri-commands';
  import type { TemplateLanguage } from '$lib/types';
  import TemplateEditor from './TemplateEditor.svelte';

  let languages = $state<TemplateLanguage[]>([]);
  let selectedLangKey = $state<string | null>(null);
  let langSearch = $state('');

  const filteredLanguages = $derived(
    langSearch.trim() === ''
      ? languages
      : languages.filter((l) =>
          l.display_name.toLowerCase().includes(langSearch.toLowerCase()) ||
          l.language_key.toLowerCase().includes(langSearch.toLowerCase())
        )
  );

  async function loadLanguages() {
    try {
      const all = await listTemplateLanguages();
      languages = all.filter(l => l.language_key !== '_global');
      if (!selectedLangKey && languages.length > 0) {
        selectedLangKey = languages[0].language_key;
      }
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function back() {
    currentScreen.set({ name: 'settings' });
  }

  $effect(() => {
    loadLanguages();
  });
</script>

<div class="templates-screen">
  <div class="header">
    <button class="ghost back-btn" onclick={back} type="button">
      {$tStore('templates.back' as any)}
    </button>
    <h2>{$tStore('templates.title' as any)}</h2>
  </div>

  {#if languages.length === 0}
    <div class="empty">
      <p>{$tStore('templates.noLanguages' as any)}</p>
    </div>
  {:else}
    <div class="body">
      <aside class="lang-list">
        <input
          class="lang-search"
          type="text"
          bind:value={langSearch}
          placeholder={$tStore('templates.searchLang' as any)}
        />
        <div class="lang-items">
          {#each filteredLanguages as lang (lang.language_key)}
            <button
              class="lang-item"
              class:active={selectedLangKey === lang.language_key}
              onclick={() => (selectedLangKey = lang.language_key)}
              type="button"
            >
              <span class="lang-key">{lang.display_name}</span>
              <span class="file-count">{lang.file_count}</span>
            </button>
          {/each}
          {#if filteredLanguages.length === 0}
            <div class="no-match">{$tStore('templates.noMatch' as any)}</div>
          {/if}
        </div>
      </aside>

      {#if selectedLangKey}
        <TemplateEditor languageKey={selectedLangKey} />
      {/if}
    </div>
  {/if}
</div>

<style>
  .templates-screen { height: 100%; display: flex; flex-direction: column; overflow: hidden; }
  .header { flex-shrink: 0; padding: 12px 24px; border-bottom: 1px solid var(--border); display: flex; align-items: center; gap: 16px; }
  .back-btn { font-size: 12px; padding: 3px 8px; color: var(--text-muted); }
  .back-btn:hover { color: var(--accent); }
  h2 { font-size: 18px; font-weight: 700; margin: 0; }
  .empty { padding: 40px; text-align: center; color: var(--text-muted); }
  .body { flex: 1; display: grid; grid-template-columns: 220px 1fr; overflow: hidden; }
  .lang-list { display: flex; flex-direction: column; border-right: 1px solid var(--border); background: var(--bg); overflow: hidden; }
  .lang-search { margin: 8px; padding: 5px 8px; font-size: 12px; background: var(--surface); border: 1px solid var(--border); border-radius: 4px; color: var(--text); flex-shrink: 0; }
  .lang-items { flex: 1; overflow-y: auto; padding: 0 4px 8px 4px; }
  .lang-item { display: flex; align-items: center; justify-content: space-between; width: 100%; padding: 6px 8px; font-size: 12px; font-family: monospace; background: transparent; border: 1px solid transparent; border-radius: 4px; color: var(--text); cursor: pointer; text-align: left; margin-bottom: 2px; }
  .lang-item:hover { background: var(--surface); }
  .lang-item.active { background: var(--surface); border-color: var(--accent); color: var(--accent); }
  .lang-key { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .file-count { font-size: 10px; opacity: 0.6; background: var(--border); padding: 1px 6px; border-radius: 6px; flex-shrink: 0; }
  .no-match { padding: 12px 8px; color: var(--text-muted); font-size: 12px; text-align: center; }
</style>
