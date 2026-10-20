<script lang="ts">
  import { formatSyncIssues } from '$lib/utils/sync-issues';
  import { selectedProjectId, currentScreen } from '$lib/stores/ui';
  import { addToast } from '$lib/stores/ui';
  import { allRepos } from '$lib/stores/repos';
  import { projects } from '$lib/stores/projects';
  import { tStore } from '$lib/i18n';
  import { syncProject, listProjectRequirements, confirmRequirement, listCodexEdges, listPeerChannels } from '$lib/api/tauri-commands';
  import { noteManualSync } from '$lib/stores/autosync';
  import { refreshAfterSync } from '$lib/stores/shared';
  import { toastRulesFindings } from '$lib/utils/rules-report';
  import { getDisplayName, type RequirementInfo, type CodexEdgeInfo, type PeerChannelInfo } from '$lib/types';

  let requirements = $state<RequirementInfo[]>([]);
  let codexEdges = $state<CodexEdgeInfo[]>([]);
  // T-000192/193: InOutBox channels between equals.
  let peerChannels = $state<PeerChannelInfo[]>([]);
  let syncing = $state(false);
  let loading = $state(true);

  const projectId = $derived($selectedProjectId);
  // T-000123: surface the current project in the header so the user knows
  // which scope this sync is acting on (was generic "Synchronize" only).
  const projectName = $derived(
    $projects.find((p) => p.id === projectId)?.name ?? ''
  );

  async function loadRequirements() {
    if (!projectId) return;
    loading = true;
    try {
      // T-000173/193: load REQ pairs + codex broadcast edges + peer channels.
      [requirements, codexEdges, peerChannels] = await Promise.all([
        listProjectRequirements(projectId),
        listCodexEdges(projectId),
        listPeerChannels(projectId),
      ]);
    } catch (e) {
      addToast(String(e), 'error');
    } finally {
      loading = false;
    }
  }

  // P8 review-fix: reactive on projectId so navigating to SyncScreen for a
  // different project (without unmount) reloads its requirements instead
  // of showing stale data from the previous project. Mirrors the pattern
  // used in TasksTab and RepoChangelogTab.
  $effect(() => {
    if (projectId) loadRequirements();
  });

  async function handleSync() {
    if (!projectId) return;
    syncing = true;
    // T-000136: reset the auto-sync elapsed clock so the next timed run waits a
    // full interval after this manual sync.
    noteManualSync();
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
        // T-000198: one translated line per issue under a counted header.
        const header = $tStore('sync.errors').replace('{0}', String(result.errors.length));
        addToast(`${header}\n${formatSyncIssues(result.errors)}`, 'warning');
      }
      // F-000039: report REQ pairs auto-closed via .impl.md acknowledgement.
      if (result.auto_closed.length > 0) {
        addToast($tStore('sync.autoClosed').replace('{0}', String(result.auto_closed.length)), 'info');
      }
      // T-000137: report repos where synced cross-repo files were auto-committed.
      if (result.committed.length > 0) {
        addToast($tStore('sync.committed').replace('{0}', String(result.committed.length)), 'info');
      }
      // T-000205 / T-000207: a rule taken back out of a repo's flagged copy now
      // applies to the whole portfolio, and a lost marker means an edit never
      // arrived. Nothing verifies the approval behind a flag, so both are always
      // named.
      toastRulesFindings({ taken: result.rules_taken, lost: result.rules_lost, superseded: [] });
      await loadRequirements();
    } catch (e) {
      addToast(String(e), 'error');
    } finally {
      syncing = false;
      // F-000044: statuses of shared repos are settled by the sync, and it may
      // have linked a contributor's repo to a project.
      void refreshAfterSync();
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
    const sourceRepoId = findRepoId(req.source_repo);
    const targetRepoId = findRepoId(req.target_repo);
    if (sourceRepoId === null || targetRepoId === null) {
      addToast(`Cannot resolve repo for ${req.filename}: ${req.source_repo} → ${req.target_repo}`, 'error');
      return;
    }
    try {
      // C1 review-fix: pass both source AND target IDs. Earlier the backend
      // iterated all connected microservices and deleted from any whose
      // filename matched — collided NNNs across MSes erased sibling pairs.
      await confirmRequirement(projectId, req.filename, sourceRepoId, targetRepoId);
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

  // T-000173: codex broadcast edges, split by direction and grouped by codex.
  const codexIncoming = $derived(codexEdges.filter((e) => e.direction === 'incoming'));
  const codexOutgoing = $derived(codexEdges.filter((e) => e.direction === 'outgoing'));

  function groupByCodex(list: CodexEdgeInfo[]): [string, CodexEdgeInfo[]][] {
    const groups = new Map<string, CodexEdgeInfo[]>();
    for (const e of list) {
      const arr = groups.get(e.codex_name);
      if (arr) arr.push(e);
      else groups.set(e.codex_name, [e]);
    }
    return [...groups.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }

  function codexStatusLabel(status: string): string {
    return status === 'synced' ? $tStore('sync.codexSynced') : $tStore('sync.codexPending');
  }

  function codexStatusClass(status: string): string {
    return status === 'synced' ? 'badge-responded' : 'badge-new';
  }

  // T-000193: peer-channel badge — same three-state vocabulary as the codex rows.
  function peerStatusLabel(status: string): string {
    if (status === 'synced') return $tStore('sync.peerSynced');
    if (status === 'unresolved') return $tStore('sync.peerUnresolved');
    return $tStore('sync.peerPending');
  }

  function peerStatusClass(status: string): string {
    if (status === 'synced') return 'badge-responded';
    if (status === 'unresolved') return '';
    return 'badge-new';
  }
</script>

<div class="sync-screen">
  <div class="header">
    <button class="ghost back-btn" onclick={goBack}>{$tStore('sync.back')}</button>
    <h2>{$tStore('sync.title')}{projectName ? ` — ${projectName}` : ''}</h2>
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
  {:else if requirements.length === 0 && codexEdges.length === 0 && peerChannels.length === 0}
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
                    {#if req.has_impl}
                      <span class="badge badge-impl" title={$tStore('sync.willAutoClose')}>&#8987; impl</span>
                    {/if}
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
                    {#if req.has_impl}
                      <span class="badge badge-impl" title={$tStore('sync.willAutoClose')}>&#8987; impl</span>
                    {/if}
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

    {@render codexSection(codexIncoming, 'sync.codexIncoming')}
    {@render codexSection(codexOutgoing, 'sync.codexOutgoing')}

    {#if peerChannels.length > 0}
      <div class="section">
        <div class="section-label"><span class="codex-emoji">&#129309;</span> {$tStore('sync.peerChannels')}</div>
        <div class="req-list">
          {#each peerChannels as p (p.self_repo_id + '::' + p.peer_repo_id)}
            <div class="req-row">
              <div class="req-info">
                <span class="req-filename">{p.self_repo} &harr; {p.peer_repo}{p.peer_project ? ` (${p.peer_project})` : ''}</span>
                <span class="inbox">&#8627; {p.folder}</span>
                {#if !p.registry_present}
                  <span class="inbox no-registry">{$tStore('sync.peerNoRegistry')}</span>
                {/if}
              </div>
              <div class="req-actions">
                <span class="filecount">
                  {$tStore('sync.peerCounts')
                    .replace('{0}', String(p.outbox_count))
                    .replace('{1}', String(p.inbox_count))
                    .replace('{2}', String(p.pact_count))}
                </span>
                <span class="badge {peerStatusClass(p.status)}">{peerStatusLabel(p.status)}</span>
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  {/if}
</div>

{#snippet codexSection(list: CodexEdgeInfo[], labelKey: string)}
  {#if list.length > 0}
    <div class="section">
      <div class="section-label"><span class="codex-emoji">&#128218;</span> {$tStore(labelKey as any)}</div>
      {#each groupByCodex(list) as [codexName, edges] (codexName)}
        <div class="source-group">
          <div class="source-header">&#128218; {codexName}</div>
          <div class="req-list">
            {#each edges as e (e.codex_name + '::' + e.consumer_name + '::' + e.lane)}
              <div class="req-row">
                <div class="req-info">
                  <span class="req-filename">{e.source_repo} &rarr; {e.target_repo}</span>
                  <span class="inbox">&#8627; {e.inbox}</span>
                </div>
                <div class="req-actions">
                  <span class="filecount">{$tStore('sync.codexFiles').replace('{0}', String(e.file_count))}</span>
                  <span class="badge badge-lane">{e.lane}</span>
                  <span class="badge {codexStatusClass(e.status)}">{codexStatusLabel(e.status)}</span>
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
{/snippet}

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

  /* T-000193: peer channel hint when the ledger has not been created yet */
  .no-registry {
    color: var(--warning, #b8860b);
  }

  /* T-000173: codex broadcast section */
  .codex-emoji {
    font-size: 12px;
  }

  .inbox {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
    opacity: 0.85;
  }

  .filecount {
    font-size: 11px;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .badge-lane {
    background-color: rgba(168, 85, 247, 0.15);
    border-color: rgba(168, 85, 247, 0.5);
    color: rgb(168, 85, 247);
    font-family: monospace;
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

  /* F-000039: .impl.md present → pair will auto-close on next sync. */
  .badge-impl {
    background-color: rgba(168, 85, 247, 0.15);
    border-color: rgba(168, 85, 247, 0.5);
    color: rgb(168, 85, 247);
    font-variant-numeric: tabular-nums;
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
