<script lang="ts">
  import { toasts, dismissToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
</script>

{#if $toasts.length > 0}
  <div class="toast-container" role="status" aria-live="polite">
    {#each $toasts as toast (toast.id)}
      <button
        class="toast toast-{toast.type}"
        onclick={() => dismissToast(toast.id)}
        title={$tStore('toast.clickToDismiss')}
      >
        <span class="toast-message">{toast.message}</span>
        <span class="toast-close" aria-hidden="true">×</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  .toast-container {
    position: fixed;
    bottom: 16px;
    right: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 9999;
    max-width: 380px;
  }

  .toast {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
    padding: 10px 14px;
    border-radius: 4px;
    border: none;
    cursor: pointer;
    font-size: 13px;
    font-family: inherit;
    font-weight: 400;
    line-height: 1.4;
    text-align: left;
    width: 100%;
    min-width: 240px;
    color: #fff;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
  }

  .toast:hover {
    opacity: 0.9;
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

  .toast-close {
    font-size: 16px;
    line-height: 1;
    opacity: 0.8;
    flex-shrink: 0;
  }
</style>
