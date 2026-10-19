<script lang="ts">
  // F-000044 / T-000223 (frame 3): confirm switching shared mode on. The body
  // scrolls when the content is long (many blocking .gitignore lines); the
  // title and the buttons stay pinned.
  import { tStore } from '$lib/i18n';
  import type { IgnoreLine } from '$lib/types';

  interface Props {
    repoName: string;
    /** The owner's branch (the repo's auto-commit branch). */
    devBranch: string;
    /** `true` public, `false` private, `null` unknown (no warning). */
    isPublic: boolean | null;
    blockingLines: IgnoreLine[];
    busy?: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  }

  let { repoName, devBranch, isPublic, blockingLines, busy = false, onConfirm, onCancel }: Props = $props();

  const hasBlocking = $derived(blockingLines.length > 0);
</script>

<div
  class="overlay"
  role="presentation"
  onclick={() => !busy && onCancel()}
  onkeydown={(e) => e.key === 'Escape' && !busy && onCancel()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="shared-enable-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && !busy && onCancel()}
  >
    <h3 id="shared-enable-title" class="title">
      {$tStore('shared.enable.title' as any).replace('{0}', repoName)}
    </h3>

    <div class="body">
      <p class="message">{$tStore('shared.enable.body' as any)}</p>
      <p class="branch">
        {$tStore('shared.enable.devBranch' as any).replace('{0}', '')}<code>{devBranch}</code>
      </p>

      {#if isPublic === true}
        <div class="banner warning">{$tStore('shared.enable.public' as any)}</div>
      {/if}

      {#if hasBlocking}
        <div class="banner caution">
          <div class="banner-title">{$tStore('shared.enable.blockingTitle' as any)}</div>
          <ul class="lines">
            {#each blockingLines as l (l.line_no)}
              <li><span class="no">{l.line_no}</span><span class="text">{l.text}</span></li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>

    <div class="actions">
      <button type="button" onclick={onCancel} disabled={busy}>{$tStore('dialog.cancel')}</button>
      <button type="button" class="primary" onclick={onConfirm} disabled={busy}>
        {busy
          ? $tStore('shared.enable.working' as any)
          : hasBlocking
            ? $tStore('shared.enable.confirmRemove' as any)
            : $tStore('shared.enable.confirm' as any)}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background-color: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10000;
  }

  .dialog {
    background-color: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 24px;
    width: min(520px, calc(100vw - 48px));
    max-height: min(90vh, 640px);
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }

  .title {
    font-size: 15px;
    font-weight: 600;
    margin: 0 0 10px;
    flex-shrink: 0;
  }

  /* The only scrolling part: the footer stays in view with any content length. */
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .message {
    font-size: 13px;
    color: var(--text-muted);
    margin: 0 0 10px;
    line-height: 1.5;
  }

  .branch {
    font-size: 13px;
    margin: 0 0 12px;
  }

  .branch code {
    font-family: monospace;
    color: var(--text);
  }

  .banner {
    font-size: 12px;
    padding: 8px 10px;
    border-radius: 4px;
    margin-bottom: 10px;
    line-height: 1.4;
  }

  .banner.warning {
    background: color-mix(in srgb, var(--toast-warning) 14%, transparent);
    color: var(--toast-warning);
    border: 1px solid color-mix(in srgb, var(--toast-warning) 45%, transparent);
  }

  .banner.caution {
    border: 1px solid var(--border);
    color: var(--text);
  }

  .banner-title {
    margin-bottom: 6px;
    color: var(--text-muted);
  }

  .lines {
    list-style: none;
    margin: 0;
    padding: 0;
    font-family: monospace;
    font-size: 12px;
  }

  .lines li {
    display: flex;
    gap: 12px;
    padding: 1px 0;
  }

  .lines .no {
    min-width: 3ch;
    text-align: right;
    color: var(--text-muted);
  }

  .lines .text {
    color: var(--text);
    word-break: break-all;
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 14px;
    flex-shrink: 0;
  }

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }

  .primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
