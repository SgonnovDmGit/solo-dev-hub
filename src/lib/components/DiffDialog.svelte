<script lang="ts">
  import { diffLines } from 'diff';
  import { tStore } from '$lib/i18n';
  import type { RenderedFile } from '$lib/types';

  interface FileEntry {
    path: string;
    content: string;
    existingContent: string | null;
    shouldWrite: boolean;
  }

  interface Props {
    files: FileEntry[];
    onConfirm: (files: RenderedFile[]) => void;
    onCancel: () => void;
  }

  let { files = $bindable(), onConfirm, onCancel }: Props = $props();

  function status(entry: FileEntry): 'new' | 'changed' | 'unchanged' {
    if (entry.existingContent === null) return 'new';
    if (entry.existingContent === entry.content) return 'unchanged';
    return 'changed';
  }

  function toggleWrite(path: string) {
    files = files.map((f) => (f.path === path ? { ...f, shouldWrite: !f.shouldWrite } : f));
  }

  // Build diffLines once per entry; compute side-by-side rows.
  function sideBySide(oldText: string, newText: string): Array<{ left: string; right: string; tag: 'same' | 'removed' | 'added' }> {
    const parts = diffLines(oldText, newText);
    const rows: Array<{ left: string; right: string; tag: 'same' | 'removed' | 'added' }> = [];
    for (const p of parts) {
      const lines = p.value.split('\n');
      if (lines[lines.length - 1] === '') lines.pop();
      for (const line of lines) {
        if (p.added) rows.push({ left: '', right: line, tag: 'added' });
        else if (p.removed) rows.push({ left: line, right: '', tag: 'removed' });
        else rows.push({ left: line, right: line, tag: 'same' });
      }
    }
    return rows;
  }

  function confirm() {
    const toWrite = files.filter((f) => f.shouldWrite && status(f) !== 'unchanged');
    onConfirm(toWrite.map((f) => ({ path: f.path, content: f.content })));
  }
</script>

<div class="overlay" role="presentation" onclick={onCancel} onkeydown={(e) => e.key === 'Escape' && onCancel()}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1"
       onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === 'Escape' && onCancel()}>
    <h3 class="title">{$tStore('deploy.diffTitle' as any)}</h3>

    <div class="files">
      {#each files as entry (entry.path)}
        {@const st = status(entry)}
        <div class="file-block">
          <div class="file-header">
            <span class="file-path">{entry.path}</span>
            <span class="tag tag-{st}">
              {#if st === 'new'}{$tStore('deploy.fileNew' as any)}
              {:else if st === 'changed'}{$tStore('deploy.fileChanged' as any)}
              {:else}{$tStore('deploy.fileUnchanged' as any)}{/if}
            </span>
            <label class="write-toggle">
              <input
                type="checkbox"
                checked={entry.shouldWrite}
                disabled={st === 'unchanged'}
                onchange={() => toggleWrite(entry.path)}
              />
              {st === 'new' ? $tStore('deploy.create' as any) : $tStore('deploy.overwrite' as any)}
            </label>
          </div>

          {#if st === 'new'}
            <pre class="content-block">{entry.content}</pre>
          {:else if st === 'changed' && entry.existingContent !== null}
            <div class="sbs">
              <div class="sbs-col-head">current</div>
              <div class="sbs-col-head">new</div>
              {#each sideBySide(entry.existingContent, entry.content) as row}
                <div class="sbs-line sbs-{row.tag === 'added' ? 'empty' : row.tag}">{row.left}</div>
                <div class="sbs-line sbs-{row.tag === 'removed' ? 'empty' : row.tag}">{row.right}</div>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>

    <div class="actions">
      <button onclick={onCancel}>{$tStore('dialog.cancel')}</button>
      <button class="primary" onclick={confirm}>{$tStore('deploy.writeSelected' as any)}</button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10000;
  }
  .dialog {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 20px;
    width: 95vw;
    max-width: 1200px;
    max-height: 90vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }
  .title {
    font-size: 15px;
    font-weight: 600;
    margin: 0 0 12px 0;
  }
  .files {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .file-block {
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
  }
  .file-header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .file-path {
    font-family: monospace;
    font-size: 12px;
    color: var(--text);
  }
  .tag {
    font-size: 10px;
    padding: 1px 7px;
    border-radius: 6px;
    font-family: sans-serif;
  }
  .tag-new { background: rgba(34, 197, 94, 0.15); color: rgb(34, 197, 94); }
  .tag-changed { background: rgba(234, 179, 8, 0.15); color: rgb(234, 179, 8); }
  .tag-unchanged { background: rgba(100, 100, 100, 0.15); color: var(--text-muted); }
  .write-toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-muted);
    margin-left: auto;
    cursor: pointer;
  }
  .write-toggle input[type="checkbox"] { cursor: pointer; }
  .write-toggle:has(input:disabled) { opacity: 0.5; cursor: not-allowed; }
  .content-block {
    font-family: monospace;
    font-size: 11px;
    padding: 10px;
    margin: 0;
    background: var(--bg);
    color: var(--text);
    max-height: 400px;
    overflow: auto;
    white-space: pre;
  }
  .sbs {
    display: grid;
    grid-template-columns: 1fr 1fr;
    font-family: monospace;
    font-size: 11px;
    line-height: 1.45;
    max-height: 400px;
    overflow: auto;
  }
  .sbs-col-head {
    background: var(--bg);
    padding: 4px 10px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    color: var(--text-muted);
    position: sticky;
    top: 0;
    border-bottom: 1px solid var(--border);
  }
  .sbs-line {
    padding: 1px 10px;
    white-space: pre;
    min-height: 1.45em;
  }
  .sbs-same { color: var(--text); }
  .sbs-removed { background: rgba(239, 68, 68, 0.15); color: #ef4444; }
  .sbs-added { background: rgba(34, 197, 94, 0.15); color: rgb(34, 197, 94); }
  .sbs-empty { background: rgba(100, 100, 100, 0.05); }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }
</style>
