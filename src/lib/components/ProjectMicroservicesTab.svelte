<script lang="ts">
  import { selectedProjectId, currentScreen, addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import {
    connectMicroservice, disconnectMicroservice, listProjectMicroservices,
    listMicroserviceProjects, serverRepoOfMicroservice,
    connectCodex, listConnectedCodices, listCodexProjects,
    listPeerChannels, addPeerLink, removePeerLink,
  } from '$lib/api/tauri-commands';
  import { allRepos } from '$lib/stores/repos';
  import { getDisplayName, type Project, type Repository, type PeerChannelInfo } from '$lib/types';

  interface Props {
    project: Project;
    parents: Project[];
  }
  let { project, parents }: Props = $props();

  // F-012: list of all microservice-type projects (for connection dropdown)
  let allMicroserviceProjects = $state<Project[]>([]);

  // Connected microservice-project IDs for current project
  let connectedMicroserviceIds = $state<number[]>([]);

  // T-000168: Codices — connectable codex projects + connected ids
  let allCodexProjects = $state<Project[]>([]);
  let connectedCodexIds = $state<number[]>([]);

  // Cached server-repo info per microservice-project (for inline display)
  let msServerRepoCache = $state<Record<number, { repo: Repository | null; error: string | null }>>({});

  async function loadMicroservices() {
    if (!project) return;
    try {
      connectedMicroserviceIds = await listProjectMicroservices(project.id);
      allMicroserviceProjects = await listMicroserviceProjects();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  // T-000168: load codex projects + this project's connected codices.
  async function loadCodices() {
    if (!project) return;
    try {
      connectedCodexIds = await listConnectedCodices(project.id);
      allCodexProjects = await listCodexProjects();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function toggleCodex(codexProjectId: number) {
    if (!project) return;
    try {
      if (connectedCodexIds.includes(codexProjectId)) {
        // Disconnect reuses the generic edge-delete command.
        await disconnectMicroservice(project.id, codexProjectId);
      } else {
        await connectCodex(project.id, codexProjectId);
      }
      await loadCodices();
    } catch (err: any) {
      const msg = String(err);
      if (msg.toLowerCase().includes('cycle')) {
        addToast($tStore('toast.cycleDetected' as any), 'error');
      } else {
        addToast(msg, 'error');
      }
    }
  }

  // T-000193: peer channels of this project — derived pairs (same role, read
  // only) and explicit links (unlinkable). The same list the sync screen shows.
  let peerChannels = $state<PeerChannelInfo[]>([]);
  let linkSelfId = $state<number | null>(null);
  let linkPeerId = $state<number | null>(null);

  async function loadPeers() {
    if (!project) return;
    try {
      peerChannels = await listPeerChannels(project.id);
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  const myRepos = $derived($allRepos.filter((r) => r.project_id === project?.id));
  const linkCandidates = $derived($allRepos.filter((r) => r.id !== linkSelfId));

  async function createLink() {
    if (!linkSelfId || !linkPeerId) return;
    try {
      await addPeerLink(linkSelfId, linkPeerId);
      linkPeerId = null;
      await loadPeers();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function dropLink(selfId: number, peerId: number) {
    try {
      await removePeerLink(selfId, peerId);
      await loadPeers();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function loadServerRepoFor(msProjectId: number) {
    try {
      const repo = await serverRepoOfMicroservice(msProjectId);
      msServerRepoCache = { ...msServerRepoCache, [msProjectId]: { repo, error: null } };
    } catch (err: any) {
      msServerRepoCache = { ...msServerRepoCache, [msProjectId]: { repo: null, error: String(err) } };
    }
  }

  $effect(() => {
    if (project) { loadMicroservices(); loadCodices(); loadPeers(); }
  });

  // Default the "my repo" side to this project's first repo so the picker is
  // usable without a first click.
  $effect(() => {
    if (linkSelfId === null && myRepos.length > 0) linkSelfId = myRepos[0].id;
  });

  $effect(() => {
    // Load server-repo info for every listed microservice-project
    for (const ms of allMicroserviceProjects) {
      if (!(ms.id in msServerRepoCache)) {
        loadServerRepoFor(ms.id);
      }
    }
  });

  async function toggleMicroservice(msProjectId: number) {
    if (!project) return;
    try {
      if (connectedMicroserviceIds.includes(msProjectId)) {
        await disconnectMicroservice(project.id, msProjectId);
      } else {
        await connectMicroservice(project.id, msProjectId);
      }
      await loadMicroservices();
    } catch (err: any) {
      const msg = String(err);
      if (msg.toLowerCase().includes('cycle')) {
        addToast($tStore('toast.cycleDetected' as any), 'error');
      } else {
        addToast(msg, 'error');
      }
    }
  }

  function openParentProject(parentId: number) {
    selectedProjectId.set(parentId);
    currentScreen.set({ name: 'project' });
  }
</script>

<!-- Microservices section (connecting other microservice-projects to this one) -->
<div class="microservices-section">
  <div class="section-label">{$tStore('project.microservices')} <span class="tab-count">{allMicroserviceProjects.filter(ms => ms.id !== project.id).length}</span></div>

  {#if allMicroserviceProjects.filter(ms => ms.id !== project.id).length === 0}
    <div class="empty-repos">
      <div class="empty-title">{$tStore('project.noMicroservices')}</div>
      <div class="empty-hint">{$tStore('project.noMicroservicesHint')}</div>
    </div>
  {:else}
    <div class="microservice-list">
      {#each allMicroserviceProjects.filter(ms => ms.id !== project.id) as ms (ms.id)}
        {@const isConnected = connectedMicroserviceIds.includes(ms.id)}
        {@const cached = msServerRepoCache[ms.id]}
        <div class="microservice-row">
          <button class="ghost repo-link" onclick={() => { selectedProjectId.set(ms.id); currentScreen.set({ name: 'project' }); }}>
            <span class="ms-icon">⚙</span>
            {ms.name}
          </button>
          <span class="ms-server-info">
            {#if cached?.repo}
              → {cached.repo.github_name}
            {:else if cached?.error}
              {#if cached.error.includes('no server-repo')}
                {$tStore('project.microserviceNoServer' as any)}
              {:else if cached.error.includes('server-repos')}
                {$tStore('project.microserviceMultipleServers' as any)}
              {:else}
                {cached.error}
              {/if}
            {/if}
          </span>
          <button
            class="toggle-btn"
            class:connected={isConnected}
            onclick={() => toggleMicroservice(ms.id)}
          >
            {isConnected ? $tStore('project.connected') : $tStore('project.disconnected')}
          </button>
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- T-000168: Codices section (connecting codex-projects — practice svods — to this one) -->
<div class="microservices-section">
  <div class="section-label">📚 {$tStore('project.codices' as any)} <span class="tab-count">{allCodexProjects.filter(cx => cx.id !== project.id).length}</span></div>

  {#if allCodexProjects.filter(cx => cx.id !== project.id).length === 0}
    <div class="empty-repos">
      <div class="empty-title">{$tStore('project.noCodices' as any)}</div>
      <div class="empty-hint">{$tStore('project.noCodicesHint' as any)}</div>
    </div>
  {:else}
    <div class="microservice-list">
      {#each allCodexProjects.filter(cx => cx.id !== project.id) as cx (cx.id)}
        {@const isConnected = connectedCodexIds.includes(cx.id)}
        <div class="microservice-row">
          <button class="ghost repo-link" onclick={() => { selectedProjectId.set(cx.id); currentScreen.set({ name: 'project' }); }}>
            <span class="ms-icon">📚</span>
            {cx.name}
          </button>
          <span class="ms-server-info">→ docs/codex/{cx.name}/</span>
          <button
            class="toggle-btn"
            class:connected={isConnected}
            onclick={() => toggleCodex(cx.id)}
          >
            {isConnected ? $tStore('project.connected') : $tStore('project.disconnected')}
          </button>
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- T-000193: peer channels (InOutBox) — pairs of equals, derived or explicit -->
<div class="microservices-section">
  <div class="section-label">🤝 {$tStore('project.peerChannels' as any)} <span class="tab-count">{peerChannels.length}</span></div>

  {#if peerChannels.length === 0}
    <div class="empty-repos">
      <div class="empty-title">{$tStore('project.noPeerChannels' as any)}</div>
      <div class="empty-hint">{$tStore('project.noPeerChannelsHint' as any)}</div>
    </div>
  {:else}
    <div class="microservice-list">
      {#each peerChannels as p (p.self_repo_id + '::' + p.peer_repo_id)}
        <div class="microservice-row">
          <span class="peer-pair">{p.self_repo} ↔ {p.peer_repo}{p.peer_project ? ` (${p.peer_project})` : ''}</span>
          <span class="ms-server-info">
            {p.origin === 'explicit'
              ? $tStore('project.peerExplicit' as any)
              : $tStore('project.peerDerived' as any).replace('{0}', p.role)}
          </span>
          {#if p.origin === 'explicit'}
            <button class="toggle-btn connected" onclick={() => dropLink(p.self_repo_id, p.peer_repo_id)}>
              {$tStore('project.peerUnlink' as any)}
            </button>
          {:else}
            <span class="peer-auto">{$tStore('project.peerAuto' as any)}</span>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <div class="peer-linker">
    <select bind:value={linkSelfId} aria-label={$tStore('project.peerLinkSelf' as any)}>
      {#each myRepos as r (r.id)}
        <option value={r.id}>{getDisplayName(r)}</option>
      {/each}
    </select>
    <span class="peer-arrow">↔</span>
    <select bind:value={linkPeerId} aria-label={$tStore('project.peerLinkPeer' as any)}>
      <option value={null}>{$tStore('project.peerLinkPeer' as any)}</option>
      {#each linkCandidates as r (r.id)}
        <option value={r.id}>{getDisplayName(r)}</option>
      {/each}
    </select>
    <button class="toggle-btn" disabled={!linkSelfId || !linkPeerId} onclick={createLink}>
      {$tStore('project.peerLinkAdd' as any)}
    </button>
  </div>
  <div class="peer-hint">{$tStore('project.peerLinkHint' as any)}</div>
</div>

<!-- F-012 / T-000168: microservice → parents ("Connected to"); codex → consumers -->
{#if project.project_type === 'microservice' || project.project_type === 'codex'}
  {@const isCodex = project.project_type === 'codex'}
  <div class="microservices-section">
    <div class="section-label">
      {isCodex ? $tStore('project.codexConsumers' as any) : $tStore('project.connectedParents' as any)}
      <span class="tab-count">{parents.length}</span>
    </div>
    {#if parents.length === 0}
      <div class="empty-repos">
        <div class="empty-title">{isCodex ? $tStore('project.codexConsumersEmpty' as any) : $tStore('project.connectedParentsEmpty' as any)}</div>
      </div>
    {:else}
      <div class="microservice-list">
        {#each parents as parent (parent.id)}
          <div class="microservice-row">
            <button class="ghost repo-link" onclick={() => openParentProject(parent.id)}>
              📁 {parent.name}
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  /* T-000193: peer channels */
  .peer-pair {
    font-weight: 500;
  }
  .peer-auto {
    color: var(--text-muted);
    font-size: 12px;
  }
  .peer-linker {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    flex-wrap: wrap;
  }
  .peer-arrow {
    color: var(--text-muted);
  }
  .peer-hint {
    margin-top: 6px;
    color: var(--text-muted);
    font-size: 12px;
  }

  /* Shared rules (duplicated — also used by ProjectDetail's remaining markup) */
  .empty-repos {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 32px 16px;
    color: var(--text-muted);
  }

  .empty-title {
    font-size: 14px;
    font-weight: 600;
  }

  .empty-hint {
    font-size: 12px;
    opacity: 0.7;
    text-align: center;
  }

  .repo-link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .repo-link:hover {
    text-decoration: underline;
  }

  .tab-count {
    font-size: 12px;
    color: var(--text-muted);
    background: var(--surface);
    padding: 2px 8px;
    border-radius: 10px;
    border: 1px solid var(--border);
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }

  /* MS-tab-only rules */
  .ms-server-info {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
    flex: 1;
    text-align: center;
  }

  .microservices-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    padding-top: 12px;
  }

  .microservice-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .microservice-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 5px 8px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background-color: var(--bg);
  }

  .microservice-row:hover {
    background-color: var(--surface);
  }

  .ms-icon {
    font-size: 12px;
    opacity: 0.7;
  }

  .toggle-btn {
    font-size: 11px;
    padding: 3px 10px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background-color: var(--surface);
    color: var(--text-muted);
    cursor: pointer;
    transition: background-color 0.15s, color 0.15s, border-color 0.15s;
    white-space: nowrap;
  }

  .toggle-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .toggle-btn.connected {
    background-color: rgba(34, 197, 94, 0.15);
    border-color: rgba(34, 197, 94, 0.5);
    color: rgb(34, 197, 94);
  }

  .toggle-btn.connected:hover {
    background-color: rgba(34, 197, 94, 0.25);
    border-color: rgb(34, 197, 94);
  }
</style>
