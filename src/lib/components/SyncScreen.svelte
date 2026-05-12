<script lang="ts">
  import { selectedProjectId, currentScreen } from '$lib/stores/ui';
  import { addToast } from '$lib/stores/ui';
  import { allRepos } from '$lib/stores/repos';
  import { tStore } from '$lib/i18n';
  import { syncProject, listProjectRequirements, confirmRequirement } from '$lib/api/tauri-commands';
  import { getDisplayName, type RequirementInfo } from '$lib/types';
  import { onMount } from 'svelte';

  let requirements = $state<RequirementInfo[]>([]);
  let syncing = $state(false);
  let loading = $state(true);

  const projectId = $derived($selectedProjectId);

  async function loadRequirements() {
    if (!projectId) return;
    loading = true;
    try {
      requirements = await listProjectRequirements(projectId);
    } catch (e) {
      addToast(String(e), 'error');
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    loadRequirements();
  });

  async function handleSync() {
    if (!projectId) return;
    syncing = true;
    try {
      const result = await syncProject(projectId);
      const msgKey = result.migrated > 0 ? 'sync.syncCompleteFull' : 'sync.syncComplete';
      let msg = $tStore(msgKey as any)
        .replace('{0}', String(result.copied))
        .replace('{1}', String(result.responses));
      if (result.migrated > 0) {
        msg = msg.replace('{2}', String(result.migrated));
      }
      addToast(msg, 'success');
      if (result.errors.length > 0) {
        addToast($tStore('sync.errors').replace('{0}', result.errors.join('; ')), 'warning');
      }
      await loadRequirements();
    } catch (e) {
      addToast(String(e), 'error');
    } finally {
      syncing = false;
    }
  }

  function findRepoId(name: string): number | null {
    // `name` here is RequirementInfo.source_repo, which Rust populates from
    // Repository::display_name() — the last segment of github_name (e.g.
    // "web-app-client", not "owner/web-app-client"). Matching against
    // r.github_name directly would fail for every GitHub-backed repo.
    const repo = $allRepos.find(r => getDisplayName(r) === name);
    return repo ? repo.id : null;
  }

  async function handleConfirm(req: RequirementInfo) {
    if (!projectId) return;
    const repoId = findRepoId(req.source_repo);
    if (repoId === null) return;
    try {
      await confirmRequirement(projectId, req.filename, repoId);
      addToast(`${req.filename}: confirmed`, 'success');
      // B-000016: localize update — backend deleted the file pair, this RequirementInfo no longer
      // exists. Filtering keeps surrounding DOM stable so scroll position is preserved.
      requirements = requirements.filter(r =>
        !(r.filename === req.filename
          && r.source_repo === req.source_repo
          && r.target_repo === req.target_repo)
      );
    } catch (e) {
      addToast(String(e), 'error');
    }
  }

  function goBack() {
    currentScreen.set({ name: 'project' });
  }

  const clientToServer = $derived(requirements.filter(r => r.direction === 'client_to_server'));
  const serverToClient = $derived(requirements.filter(r => r.direction === 'server_to_client'));
  const serverToMicroservice = $derived(requirements.filter(r => r.direction === 'server_to_microservice'));
  // B-000020: microservice → parent server — объединяем api.md и handlers.md потоки в одну
  // секцию "Микросервис → Сервер" (зеркальная серверной "Server → Client" после B-000019).
  const microserviceToServer = $derived(
    requirements.filter(r =>
      r.direction === 'microservice_to_server_api' || r.direction === 'microservice_to_server_handlers'
    )
  );

  function groupBySourceRepo(list: RequirementInfo[]): [string, RequirementInfo[]][] {
    const groups = new Map<string, RequirementInfo[]>();
    for (const r of list) {
      const arr = groups.get(r.source_repo);
      if (arr) arr.push(r);
      else groups.set(r.source_repo, [r]);
    }
    return [...groups.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }

  // B-000019/B-000020: shared-file directions (server→clients, ms→parents) — один файл идёт
  // нескольким targets. Группируем по (filename, status) чтобы одинаково-статусные targets
  // схлопывались в одну строку, а смешанные статусы оставались отдельными.
  type FilenameGroup = { filename: string; status: string; targets: string[] };
  function aggregateByFilename(list: RequirementInfo[]): FilenameGroup[] {
    const map = new Map<string, FilenameGroup>();
    for (const r of list) {
      const key = `${r.filename}::${r.status}`;
      const existing = map.get(key);
      if (existing) existing.targets.push(r.target_repo);
      else map.set(key, { filename: r.filename, status: r.status, targets: [r.target_repo] });
    }
    return [...map.values()].sort((a, b) =>
      a.filename.localeCompare(b.filename) || a.status.localeCompare(b.status)
    );
  }

  function statusLabel(status: string): string {
    if (status === 'new') return $tStore('sync.statusNew');
    if (status === 'sent') return $tStore('sync.statusSent');
    if (status === 'responded') return $tStore('sync.statusResponded');
    return status;
  }

  function statusClass(status: string): string {
    if (status === 'new') return 'badge-new';
    if (status === 'sent') return 'badge-sent';
    if (status === 'responded') return 'badge-responded';
    return '';
  }
</script>

<div class="sync-screen">
  <div class="header">
    <button class="ghost back-btn" onclick={goBack}>{$tStore('sync.back')}</button>
    <h2>{$tStore('sync.title')}</h2>
    <button
      class="sync-btn"
      onclick={handleSync}
      disabled={syncing}
    >
      {syncing ? $tStore('sync.syncing') : $tStore('sync.syncButton')}
    </button>
  </div>

  {#if loading}
    <div class="loading">{$tStore('sync.syncing')}</div>
  {:else if requirements.length === 0}
    <div class="empty">
      <div class="empty-title">{$tStore('sync.noRequirements')}</div>
      <div class="empty-hint">{$tStore('sync.noRequirementsHint')}</div>
    </div>
  {:else}
    {#if clientToServer.length > 0}
      <div class="section">
        <div class="section-label">{$tStore('sync.clientToServer')}</div>
        {#each groupBySourceRepo(clientToServer) as [sourceRepo, reqs] (sourceRepo)}
          <div class="source-group">
            <div class="source-header">{sourceRepo}</div>
            <div class="req-list">
              {#each reqs as req (req.filename + req.source_repo)}
                <div class="req-row">
                  <div class="req-info">
                    <span class="req-filename">{req.filename}</span>
                    <span class="req-repos">{req.source_repo} &rarr; {req.target_repo}</span>
                  </div>
                  <div class="req-actions">
                    <span class="badge {statusClass(req.status)}">{statusLabel(req.status)}</span>
                    {#if req.status === 'responded'}
                      <button class="action-btn confirm-btn" onclick={() => handleConfirm(req)} title={$tStore('sync.confirm')}>
                        &#10003;
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#if serverToClient.length > 0}
      <div class="section">
        <div class="section-label">{$tStore('sync.serverToClient' as any)}</div>
        {#each groupBySourceRepo(serverToClient) as [sourceRepo, reqs] (sourceRepo)}
          <div class="source-group">
            <div class="source-header">{sourceRepo}</div>
            <div class="req-list">
              {#each aggregateByFilename(reqs) as group (group.filename + '::' + group.status)}
                <div class="req-row">
                  <div class="req-info">
                    <span class="req-filename">{group.filename}</span>
                    <span class="req-repos">{sourceRepo} &rarr; {group.targets.join(', ')}</span>
                  </div>
                  <div class="req-actions">
                    {#if group.targets.length > 1}
                      <span class="target-counter">×{group.targets.length}</span>
                    {/if}
                    <span class="badge {statusClass(group.status)}">{statusLabel(group.status)}</span>
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#if serverToMicroservice.length > 0}
      <div class="section">
        <div class="section-label">{$tStore('sync.serverToMicroservice')}</div>
        {#each groupBySourceRepo(serverToMicroservice) as [sourceRepo, reqs] (sourceRepo)}
          <div class="source-group">
            <div class="source-header">{sourceRepo}</div>
            <div class="req-list">
              {#each reqs as req (req.filename + req.target_repo)}
                <div class="req-row">
                  <div class="req-info">
                    <span class="req-filename">{req.filename}</span>
                    <span class="req-repos">{req.source_repo} &rarr; {req.target_repo}</span>
                  </div>
                  <div class="req-actions">
                    {#if req.is_reverse_lookup}
                      <span class="reverse-hint" title="Reverse-lookup view — подтверждать должен sender (parent server) из своего SyncScreen.">↩</span>
                    {/if}
                    <span class="badge {statusClass(req.status)}">{statusLabel(req.status)}</span>
                    {#if req.status === 'responded' && !req.is_reverse_lookup}
                      <button class="action-btn confirm-btn" onclick={() => handleConfirm(req)} title={$tStore('sync.confirm')}>
                        &#10003;
                      </button>
                    {/if}
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}

    {#if microserviceToServer.length > 0}
      <div class="section">
        <div class="section-label">{$tStore('sync.microserviceToServer' as any)}</div>
        {#each groupBySourceRepo(microserviceToServer) as [sourceRepo, reqs] (sourceRepo)}
          <div class="source-group">
            <div class="source-header">{sourceRepo}</div>
            <div class="req-list">
              {#each aggregateByFilename(reqs) as group (group.filename + '::' + group.status)}
                <div class="req-row">
                  <div class="req-info">
                    <span class="req-filename">{group.filename}</span>
                    <span class="req-repos">{sourceRepo} &rarr; {group.targets.join(', ')}</span>
                  </div>
                  <div class="req-actions">
                    {#if group.targets.length > 1}
                      <span class="target-counter">×{group.targets.length}</span>
                    {/if}
                    <span class="badge {statusClass(group.status)}">{statusLabel(group.status)}</span>
                  </div>
                </div>
              {/each}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .sync-screen {
    padding: 24px 28px;
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: auto;
    gap: 20px;
  }

  .header {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-shrink: 0;
  }

  .back-btn {
    font-size: 12px;
    padding: 3px 8px;
    color: var(--text-muted);
  }

  .back-btn:hover {
    color: var(--accent);
  }

  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 700;
    flex: 1;
  }

  .sync-btn {
    padding: 6px 16px;
    font-size: 13px;
    border-radius: 4px;
    border: 1px solid var(--accent);
    background-color: var(--accent);
    color: white;
    cursor: pointer;
    white-space: nowrap;
  }

  .sync-btn:hover {
    opacity: 0.9;
  }

  .sync-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .loading {
    text-align: center;
    color: var(--text-muted);
    padding: 40px;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 60px 16px;
    color: var(--text-muted);
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
  }

  .empty-hint {
    font-size: 12px;
    opacity: 0.7;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .source-group {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-left: 8px;
    border-left: 2px solid var(--border);
  }

  .source-header {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
    font-family: monospace;
    padding: 2px 0;
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }

  .req-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .req-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background-color: var(--bg);
  }

  .req-row:hover {
    background-color: var(--surface);
  }

  .req-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .req-filename {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
  }

  .req-repos {
    font-size: 11px;
    color: var(--text-muted);
  }

  .req-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .badge {
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 10px;
    border: 1px solid var(--border);
    white-space: nowrap;
  }

  .target-counter {
    font-size: 11px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .reverse-hint {
    font-size: 12px;
    color: var(--text-muted);
    opacity: 0.7;
    cursor: help;
  }

  .badge-new {
    background-color: rgba(59, 130, 246, 0.15);
    border-color: rgba(59, 130, 246, 0.5);
    color: rgb(59, 130, 246);
  }

  .badge-sent {
    background-color: rgba(234, 179, 8, 0.15);
    border-color: rgba(234, 179, 8, 0.5);
    color: rgb(234, 179, 8);
  }

  .badge-responded {
    background-color: rgba(34, 197, 94, 0.15);
    border-color: rgba(34, 197, 94, 0.5);
    color: rgb(34, 197, 94);
  }

  .action-btn {
    width: 28px;
    height: 28px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--surface);
    cursor: pointer;
    font-size: 14px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .confirm-btn {
    color: rgb(34, 197, 94);
  }

  .confirm-btn:hover {
    background-color: rgba(34, 197, 94, 0.15);
    border-color: rgb(34, 197, 94);
  }

</style>
