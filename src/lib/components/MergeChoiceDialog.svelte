<script lang="ts">
  import type { Repository } from '$lib/types';
  import { tStore, tf } from '$lib/i18n';

  interface Props {
    githubName: string;
    candidates: Repository[];
    onResolve: (localId: number) => void;
    onCreateNew: () => void;
    onSkip: () => void;
  }

  let { githubName, candidates, onResolve, onCreateNew, onSkip }: Props = $props();

  let choice = $state<string>('create-new');

  function apply() {
    if (choice === 'create-new') {
      onCreateNew();
    } else {
      const localId = Number(choice);
      if (!Number.isNaN(localId)) onResolve(localId);
    }
  }
</script>

<div
  class="overlay"
  role="presentation"
  onclick={onSkip}
  onkeydown={(e) => e.key === 'Escape' && onSkip()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="merge-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && onSkip()}
  >
    <h3 id="merge-title" class="title">{$tStore('merge.title' as any)}</h3>
    <p class="message">{tf('merge.description' as any, githubName)}</p>

    <div class="choices">
      {#each candidates as c (c.id)}
        <label class="choice">
          <input type="radio" name="merge-choice" value={String(c.id)} bind:group={choice} />
          <span class="choice-label">
            <span class="choice-title">
              {$tStore('merge.chooseExisting' as any).replace('{0}', c.description ?? `#${c.id}`)}
            </span>
            <span class="choice-path">{c.local_path ?? ''}</span>
          </span>
        </label>
      {/each}
      <label class="choice">
        <input type="radio" name="merge-choice" value="create-new" bind:group={choice} />
        <span class="choice-label">
          <span class="choice-title">{$tStore('merge.createNew' as any)}</span>
        </span>
      </label>
    </div>

    <div class="actions">
      <button onclick={onSkip} type="button">{$tStore('merge.skip' as any)}</button>
      <button class="primary" onclick={apply} type="button">{$tStore('merge.apply' as any)}</button>
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
    min-width: 520px;
    max-width: 640px;
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
    margin-bottom: 16px;
    line-height: 1.5;
  }

  .choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 20px;
  }

  .choice {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 4px;
    cursor: pointer;
  }

  .choice:hover {
    background-color: var(--border);
  }

  .choice-label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }

  .choice-title {
    font-size: 13px;
    font-weight: 500;
  }

  .choice-path {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
    word-break: break-all;
  }

  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
</style>
