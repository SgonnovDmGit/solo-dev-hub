<script lang="ts">
  import { tStore } from '$lib/i18n';

  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    message: string;
    onConfirm: () => void;
    onCancel: () => void;
    children?: Snippet;
  }

  let { title, message, onConfirm, onCancel, children }: Props = $props();
</script>

<div
  class="overlay"
  role="presentation"
  onclick={onCancel}
  onkeydown={(e) => e.key === 'Escape' && onCancel()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="dialog-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && onCancel()}
  >
    <h3 id="dialog-title" class="title">{title}</h3>
    <p class="message">{message}</p>
    {#if children}
      {@render children()}
    {/if}
    <div class="actions">
      <button onclick={onCancel}>{$tStore('dialog.cancel')}</button>
      <button class="danger" onclick={onConfirm}>{$tStore('dialog.confirm')}</button>
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
    min-width: 500px;
    max-width: 480px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  }

  .title {
    font-size: 15px;
    font-weight: 600;
    margin-bottom: 10px;
  }

  .message {
    font-size: 13px;
    color: var(--text-muted);
    margin-bottom: 20px;
    line-height: 1.5;
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
</style>
