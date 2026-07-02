<script lang="ts">
  import { navigateTo } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
</script>

<div class="reports-screen">
  <header class="screen-head">
    <h2>{$tStore('reports.title' as any)}</h2>
    <p class="subtitle">{$tStore('reports.tooltip' as any)}</p>
  </header>

  <div class="cards">
    <button class="report-card" onclick={() => navigateTo('deploy_report')}>
      <span class="card-icon" aria-hidden="true">🚀</span>
      <span class="card-text">
        <span class="card-title">{$tStore('reports.deployCardTitle' as any)}</span>
        <span class="card-desc">{$tStore('reports.deployCardDesc' as any)}</span>
      </span>
    </button>

    <button class="report-card" onclick={() => navigateTo('secret_audit')}>
      <span class="card-icon" aria-hidden="true">🔑</span>
      <span class="card-text">
        <span class="card-title">{$tStore('reports.auditCardTitle' as any)}</span>
        <span class="card-desc">{$tStore('reports.auditCardDesc' as any)}</span>
      </span>
    </button>
  </div>
</div>

<style>
  .reports-screen { padding: 1rem; }
  .screen-head { margin-bottom: 1rem; }
  .screen-head h2 { margin: 0 0 0.25rem 0; font-size: 1.2rem; }
  .subtitle { color: var(--text-muted); font-size: 0.9em; margin: 0; }

  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(20rem, 1fr));
    gap: 1rem;
    align-items: start;
  }

  .report-card {
    display: flex;
    align-items: flex-start;
    gap: 0.9rem;
    width: 100%;
    text-align: left;
    padding: 1.1rem;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
    font-family: inherit;
    transition: border-color 0.12s ease, background 0.12s ease;
  }
  .report-card:hover { background: var(--surface-hover); border-color: var(--accent); }
  .report-card:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }

  .card-icon { font-size: 1.8rem; line-height: 1; flex-shrink: 0; }
  .card-text { display: flex; flex-direction: column; gap: 0.3rem; min-width: 0; }
  .card-title { font-weight: 600; font-size: 1.05rem; }
  .card-desc { color: var(--text-muted); font-size: 0.88em; line-height: 1.4; }
</style>
