<script lang="ts">
  import { tStore } from '$lib/i18n';
  import { readRepoFile } from '$lib/api/tauri-commands';

  interface Props {
    repoId: number;
  }
  let { repoId }: Props = $props();

  let content = $state<string | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  async function load() {
    loading = true;
    error = null;
    try {
      const r = await readRepoFile(repoId, 'Changelog.md');
      content = r ?? '';
    } catch (err) {
      content = null;
      error = String(err);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void repoId;
    load();
  });
</script>

<div class="changelog-tab">
  {#if loading}
    <p class="muted">{$tStore('tasks.loading' as any)}</p>
  {:else if error}
    <p class="muted error">{error}</p>
  {:else if !content}
    <p class="muted">{$tStore('changelog.emptyState' as any)}</p>
  {:else}
    <pre class="md">{content}</pre>
  {/if}
</div>

<style>
  .changelog-tab {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 20px 24px;
  }
  .md {
    font-family: var(--font-sans, system-ui, sans-serif);
    font-size: 13px;
    line-height: 1.55;
    color: var(--text);
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0;
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
    margin: 0;
  }
  .muted.error {
    color: var(--danger, #ef4444);
    font-family: monospace;
    white-space: pre-wrap;
  }
</style>
