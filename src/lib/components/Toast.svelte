<script lang="ts">
  import { toasts, dismissToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';

  // B-000027: brief per-toast "copied" feedback on the copy button.
  let copiedId = $state<number | null>(null);

  async function copyToast(id: number, message: string) {
    try {
      await navigator.clipboard.writeText(message);
      copiedId = id;
      setTimeout(() => {
        if (copiedId === id) copiedId = null;
      }, 1500);
    } catch {
      // Clipboard may be unavailable (permissions/context) — ignore silently.
    }
  }
</script>

{#if $toasts.length > 0}
  <div class="toast-container" role="status" aria-live="polite">
    {#each $toasts as toast (toast.id)}
      <!-- B-000027: every toast has selectable text, a copy button and a close
           button; clicking the text never dismisses. Error/warning toasts also
           stretch full-width (see .toast-persistent) for long messages. -->
      <div
        class="toast toast-{toast.type}"
        class:toast-persistent={toast.type === 'error' || toast.type === 'warning'}
      >
        <span class="toast-message selectable">{toast.message}</span>
        <div class="toast-tools">
          <button
            class="toast-tool"
            onclick={() => copyToast(toast.id, toast.message)}
            title={copiedId === toast.id ? $tStore('toast.copied') : $tStore('toast.copy')}
            aria-label={$tStore('toast.copy')}
          >
            {copiedId === toast.id ? '✓' : '⧉'}
          </button>
          <button
            class="toast-tool"
            onclick={() => dismissToast(toast.id)}
            title={$tStore('toast.dismiss')}
            aria-label={$tStore('toast.dismiss')}
          >
            ×
          </button>
        </div>
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    /* B-000027: container spans the full width at the bottom. Other
       toasts hug the right (flex-end); error/warning toasts override to
       stretch full-width (see .toast-persistent). */
    position: fixed;
    bottom: 16px;
    left: 16px;
    right: 16px;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    z-index: 9999;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 14px;
    border-radius: 4px;
    border: none;
    cursor: default;
    font-size: 13px;
    font-family: inherit;
    font-weight: 400;
    line-height: 1.4;
    text-align: left;
    max-width: 460px;
    min-width: 240px;
    color: #fff;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
  }

  /* B-000027: persistent errors/warnings span the full bottom width, so long
     messages get horizontal room instead of wrapping in a narrow 460px column. */
  .toast-persistent {
    align-self: stretch;
    max-width: none;
  }

  .toast-success {
    background-color: var(--toast-success);
  }

  .toast-error {
    background-color: var(--toast-error);
  }

  .toast-info {
    background-color: var(--toast-info);
  }

  .toast-warning {
    background-color: var(--toast-warning);
  }

  .toast-message {
    flex: 1;
  }

  /* B-000027: long messages are readable (scroll) and selectable (copy by hand). */
  .toast-message.selectable {
    user-select: text;
    cursor: text;
    max-height: 40vh;
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .toast-tools {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  .toast-tool {
    background: transparent;
    border: none;
    color: #fff;
    opacity: 0.85;
    cursor: pointer;
    font-size: 15px;
    line-height: 1;
    padding: 2px 5px;
    border-radius: 3px;
    font-family: inherit;
  }
  .toast-tool:hover {
    opacity: 1;
    background: rgba(255, 255, 255, 0.18);
  }

</style>
