<script lang="ts">
  interface Props {
    icon: string;
    title: string;
    hint: string;
  }

  let { icon, title, hint }: Props = $props();
</script>

<div class="empty-state">
  <span class="icon" aria-hidden="true">{icon}</span>
  <p class="title">{title}</p>
  <p class="hint">{hint}</p>
</div>

<style>
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px 20px;
    text-align: center;
    color: var(--text-muted);
  }

  .icon {
    font-size: 36px;
    line-height: 1;
  }

  .title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text-muted);
  }

  .hint {
    font-size: 12px;
    color: var(--text-muted);
    opacity: 0.7;
    max-width: 240px;
    line-height: 1.4;
  }
</style>
