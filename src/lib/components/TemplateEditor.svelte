<script lang="ts">
  import { addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import { listTemplateFiles, saveTemplateFile, resetTemplateFile } from '$lib/api/tauri-commands';
  import type { TemplateFile } from '$lib/types';
  import ConfirmDialog from './ConfirmDialog.svelte';

  let { languageKey, excludeFiles = [] }: { languageKey: string; excludeFiles?: string[] } = $props();

  let files = $state<TemplateFile[]>([]);
  let selectedFileName = $state<string | null>(null);
  let editedContent = $state('');
  let editedDirty = $state(false);
  let jsonError = $state('');
  let showResetConfirm = $state(false);

  const selectedFile = $derived(
    selectedFileName ? files.find((f) => f.file_name === selectedFileName) ?? null : null
  );

  async function loadFiles(key: string) {
    try {
      const all = await listTemplateFiles(key);
      files = all.filter((f) => !excludeFiles.includes(f.file_name));
      if (files.length > 0 && !selectedFileName) {
        selectFile(files[0].file_name);
      }
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function selectFile(fileName: string) {
    selectedFileName = fileName;
    const f = files.find((x) => x.file_name === fileName);
    editedContent = f?.content ?? '';
    editedDirty = false;
    jsonError = '';
  }

  function onEdit() {
    editedDirty = editedContent !== (selectedFile?.content ?? '');
    if (selectedFileName === 'meta.json' && editedContent.trim() !== '') {
      try { JSON.parse(editedContent); jsonError = ''; }
      catch (e) { jsonError = String(e); }
    } else {
      jsonError = '';
    }
  }

  async function handleSave() {
    if (!selectedFileName) return;
    if (jsonError) {
      addToast($tStore('templates.invalidJson' as any), 'error');
      return;
    }
    try {
      await saveTemplateFile(languageKey, selectedFileName, editedContent);
      addToast($tStore('toast.templateSaved' as any), 'success');
      await loadFiles(languageKey);
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function handleReset() {
    if (!selectedFileName) return;
    try {
      await resetTemplateFile(languageKey, selectedFileName);
      addToast($tStore('toast.templateReset' as any), 'success');
      await loadFiles(languageKey);
      showResetConfirm = false;
    } catch (err) {
      addToast(String(err), 'error');
      showResetConfirm = false;
    }
  }

  $effect(() => {
    void excludeFiles;
    loadFiles(languageKey);
  });
</script>

<div class="template-editor">
  <aside class="file-list">
    <div class="sublabel">{$tStore('templates.files' as any)}</div>
    {#each files as file (file.file_name)}
      <button
        class="file-item"
        class:active={selectedFileName === file.file_name}
        onclick={() => selectFile(file.file_name)}
        type="button"
      >
        <span class="file-name">{file.file_name.replace(/\.tmpl$/, '')}</span>
        {#if file.is_custom}
          <span class="custom-tag">{$tStore('templates.customized' as any)}</span>
        {/if}
      </button>
    {/each}
  </aside>

  <section class="editor">
    {#if selectedFile}
      <div class="editor-meta">
        <code class="file-path">{languageKey}/{selectedFile.file_name.replace(/\.tmpl$/, '')}</code>
        {#if selectedFile.is_custom}
          <span class="custom-tag">{$tStore('templates.customized' as any)}</span>
        {:else}
          <span class="bundle-tag">{$tStore('templates.bundleDefault' as any)}</span>
        {/if}
      </div>
      <textarea
        class="editor-content"
        bind:value={editedContent}
        oninput={onEdit}
        spellcheck="false"
      ></textarea>
      {#if jsonError}
        <div class="json-error">⚠ {jsonError}</div>
      {/if}
      <div class="actions">
        <button
          class="ghost reset-btn"
          onclick={() => (showResetConfirm = true)}
          disabled={!selectedFile.is_custom}
          type="button"
        >
          {$tStore('templates.resetDefault' as any)}
        </button>
        <button
          class="save-btn"
          onclick={handleSave}
          disabled={!editedDirty || !!jsonError}
          type="button"
        >
          {$tStore('templates.save' as any)}
        </button>
      </div>
    {:else}
      <div class="empty-editor">{$tStore('templates.selectFile' as any)}</div>
    {/if}
  </section>
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
  .template-editor {
    flex: 1;
    display: grid;
    grid-template-columns: 200px 1fr;
    overflow: hidden;
  }
  .file-list { display: flex; flex-direction: column; border-right: 1px solid var(--border); background: var(--bg); padding: 8px; gap: 3px; overflow-y: auto; }
  .sublabel { font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); margin-bottom: 4px; padding: 0 4px; }
  .file-item { display: flex; align-items: center; justify-content: space-between; gap: 6px; padding: 5px 8px; font-size: 12px; font-family: monospace; background: transparent; border: 1px solid transparent; border-radius: 3px; color: var(--text); cursor: pointer; text-align: left; }
  .file-item:hover { background: var(--surface); }
  .file-item.active { background: var(--surface); border-color: var(--accent); }
  .file-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .custom-tag, .bundle-tag { font-size: 9px; padding: 1px 6px; border-radius: 6px; white-space: nowrap; font-family: sans-serif; }
  .custom-tag { background: rgba(234, 179, 8, 0.15); color: rgb(234, 179, 8); }
  .bundle-tag { background: rgba(100, 100, 100, 0.15); color: var(--text-muted); }
  .editor { display: flex; flex-direction: column; padding: 12px; overflow: hidden; }
  .editor-meta { display: flex; align-items: center; gap: 10px; margin-bottom: 8px; flex-shrink: 0; }
  .file-path { font-family: monospace; font-size: 11px; color: var(--text-muted); background: var(--surface); padding: 2px 6px; border-radius: 3px; }
  .editor-content { flex: 1; font-size: 11px; font-family: monospace; padding: 10px; background: var(--surface); border: 1px solid var(--border); border-radius: 4px; color: var(--text); resize: none; line-height: 1.5; min-height: 0; }
  .editor-content:focus { outline: none; border-color: var(--accent); }
  .json-error { font-size: 11px; color: var(--danger, #ef4444); font-family: monospace; padding: 6px 0; flex-shrink: 0; }
  .actions { display: flex; justify-content: space-between; gap: 8px; margin-top: 8px; flex-shrink: 0; }
  .save-btn { font-size: 12px; padding: 5px 14px; border-radius: 4px; border: 1px solid var(--accent); background: var(--accent); color: white; cursor: pointer; }
  .save-btn:hover:not(:disabled) { opacity: 0.9; }
  .save-btn:disabled { opacity: 0.4; cursor: not-allowed; }
  .reset-btn { font-size: 12px; padding: 4px 10px; color: var(--text-muted); }
  .reset-btn:disabled { opacity: 0.3; cursor: not-allowed; }
  .empty-editor { display: flex; align-items: center; justify-content: center; flex: 1; color: var(--text-muted); font-size: 13px; }
</style>
