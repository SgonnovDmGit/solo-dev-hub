<script lang="ts">
  import { onMount } from 'svelte';
  import { currentScreen, addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import {
    listTemplateFiles,
    saveTemplateFile,
    resetTemplateFile,
    syncGlobalClaudeMd,
  } from '$lib/api/tauri-commands';
  import { aiRulesLastSyncAt } from '$lib/stores/settings';
  import { formatRelativeTime, nowTick } from '$lib/utils/time-format';
  import ConfirmDialog from './ConfirmDialog.svelte';

  const FILE_NAME = 'claude.md.global.tmpl';
  const LANGUAGE_KEY = '_global';

  let originalContent = $state('');
  let editedContent = $state('');
  let loading = $state(true);
  let syncing = $state(false);
  let showResetConfirm = $state(false);

  const isDirty = $derived(editedContent !== originalContent);
  const lastSyncDisplay = $derived(
    $aiRulesLastSyncAt
      ? $tStore('settings.aiRulesLastSync' as any).replace('{time}', formatRelativeTime($aiRulesLastSyncAt, $nowTick))
      : $tStore('settings.aiRulesNeverSynced' as any)
  );

  async function loadContent() {
    try {
      const files = await listTemplateFiles(LANGUAGE_KEY);
      const target = files.find((f) => f.file_name === FILE_NAME);
      if (!target) {
        addToast(`Template ${FILE_NAME} not found`, 'error');
        loading = false;
        return;
      }
      originalContent = target.content;
      editedContent = target.content;
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    loadContent();
  });

  async function handleSave() {
    try {
      await saveTemplateFile(LANGUAGE_KEY, FILE_NAME, editedContent);
      await loadContent();
      addToast($tStore('toast.templateSaved' as any), 'success');
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function handleSync() {
    if (isDirty || syncing) return;
    syncing = true;
    try {
      const result = await syncGlobalClaudeMd();
      aiRulesLastSyncAt.set(result.synced_at);
      addToast(
        $tStore('appDefaults.syncGlobalDone' as any).replace('{0}', result.path),
        'success',
      );
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      syncing = false;
    }
  }

  async function handleReset() {
    try {
      await resetTemplateFile(LANGUAGE_KEY, FILE_NAME);
      await loadContent();
      addToast($tStore('toast.templateReset' as any), 'success');
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      showResetConfirm = false;
    }
  }

  function back() {
    currentScreen.set({ name: 'settings' });
  }
</script>

<div class="screen">
  <div class="header">
    <button class="ghost back-btn" onclick={back} type="button">
      {$tStore('settings.back' as any)}
    </button>
    <h2>{$tStore('settings.aiRulesCard' as any)}</h2>
    <div class="header-actions">
      <span class="sync-status">{lastSyncDisplay}</span>
      <button
        class="sync-btn"
        onclick={handleSync}
        disabled={isDirty || syncing}
        title={isDirty ? $tStore('settings.aiRulesSyncTooltipDirty' as any) : undefined}
        type="button"
      >
        {syncing ? '...' : $tStore('settings.aiRulesSync' as any)}
      </button>
    </div>
  </div>

  <div class="body">
    {#if loading}
      <div class="loading">{$tStore('common.loading' as any)}</div>
    {:else}
      <textarea
        bind:value={editedContent}
        class="editor"
        spellcheck="false"
      ></textarea>
    {/if}
  </div>

  <div class="footer">
    <button
      class="ghost reset-btn"
      onclick={() => (showResetConfirm = true)}
      type="button"
      disabled={loading}
    >
      {$tStore('templates.resetDefault' as any)}
    </button>
    <button
      class="save-btn"
      onclick={handleSave}
      type="button"
      disabled={loading || !isDirty}
    >
      {$tStore('settings.save' as any)}
    </button>
  </div>
</div>

{#if showResetConfirm}
  <ConfirmDialog
    title={$tStore('templates.resetDefault' as any)}
    message={$tStore('templates.resetConfirm' as any)}
    onConfirm={handleReset}
    onCancel={() => (showResetConfirm = false)}
  />
{/if}

<style>
  .screen {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .header {
    flex-shrink: 0;
    padding: 12px 24px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .back-btn {
    font-size: 12px;
    padding: 3px 8px;
    color: var(--text-muted);
  }
  .back-btn:hover {
    color: var(--accent);
  }
  h2 {
    font-size: 18px;
    font-weight: 700;
    margin: 0;
    flex: 1;
  }
  .header-actions {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .sync-status {
    font-size: 11px;
    color: var(--text-muted);
    font-style: italic;
  }
  .sync-btn {
    font-size: 12px;
    padding: 5px 14px;
    border-radius: 4px;
    border: 1px solid var(--accent);
    background: transparent;
    color: var(--accent);
    cursor: pointer;
  }
  .sync-btn:not(:disabled):hover {
    background: var(--accent);
    color: white;
  }
  .sync-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .body {
    flex: 1;
    overflow: hidden;
    display: flex;
    padding: 12px 24px;
  }
  .editor {
    flex: 1;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: 13px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--surface);
    color: var(--text);
    resize: none;
    line-height: 1.5;
  }
  .editor:focus {
    outline: none;
    border-color: var(--accent);
  }
  .loading {
    padding: 40px;
    color: var(--text-muted);
    text-align: center;
    flex: 1;
  }
  .footer {
    flex-shrink: 0;
    padding: 10px 24px;
    border-top: 1px solid var(--border);
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }
  .reset-btn {
    font-size: 12px;
    padding: 5px 14px;
    color: var(--text-muted);
  }
  .reset-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .save-btn {
    font-size: 12px;
    padding: 5px 14px;
    border-radius: 4px;
    border: 1px solid var(--accent);
    background: var(--accent);
    color: white;
    cursor: pointer;
  }
  .save-btn:not(:disabled):hover {
    opacity: 0.9;
  }
  .save-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
