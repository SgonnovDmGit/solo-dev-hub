<script lang="ts">
  import type { TopHotProject } from '$lib/types';
  import { tStore } from '$lib/i18n';

  interface Props {
    projects: TopHotProject[];
  }

  let { projects }: Props = $props();
</script>

{#if projects.length > 0}
  <div class="tophot">
    <div class="tophot-title">{$tStore('dashboard.topHotTitle' as any)}</div>
    <div class="tophot-list">
      {#each projects as p, i (p.project_id)}
        <div class="tophot-item">
          <div class="tophot-rank">{i + 1}</div>
          <div>
            <div class="tophot-name">{p.name}</div>
            <div class="tophot-meta">
              {#if p.critical > 0}
                <span class="crit">{p.critical} crit</span> /
              {:else}
                {p.critical} crit /
              {/if}
              {p.major} maj /
              {p.active} act
            </div>
          </div>
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .tophot {
    background: linear-gradient(135deg, rgba(239, 68, 68, 0.08), rgba(239, 68, 68, 0.02));
    border: 1px solid rgba(239, 68, 68, 0.2);
    border-radius: 5px;
    padding: 8px 12px;
    margin-bottom: 14px;
  }
  .tophot-title {
    font-size: 11px; font-weight: 600; color: var(--text);
    margin-bottom: 6px;
  }
  .tophot-title::before { content: "🔥 "; }
  .tophot-list { display: flex; gap: 16px; font-size: 11px; flex-wrap: wrap; }
  .tophot-item { display: flex; align-items: center; gap: 8px; }
  .tophot-rank {
    width: 16px; height: 16px; background: rgba(239, 68, 68, 0.2);
    border-radius: 50%; color: #ef4444;
    font-weight: 700; font-size: 9px;
    display: flex; align-items: center; justify-content: center;
  }
  .tophot-name { font-weight: 600; color: var(--text); }
  .tophot-meta { color: var(--text-muted); font-size: 10px; }
  .tophot-meta .crit { color: #ef4444; font-weight: 600; }
</style>
