<script lang="ts">
  import { projects } from '$lib/stores/projects';
  import { allRepos, assignRepo, loadAllRepos } from '$lib/stores/repos';
  import { selectedProjectId, selectedRepoId, currentScreen } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import { getRoleLabel, getDisplayName, ROLE_ICONS, type Role } from '$lib/types';
  import type { StatsSummary as StatsSummaryData } from '$lib/types';
  import ProjectHeader from './ProjectHeader.svelte';
  import StatsSummary from './StatsSummary.svelte';
  import RecentActivityFeed from './RecentActivityFeed.svelte';
  import ProjectGraph from './ProjectGraph.svelte';
  import {
    connectMicroservice, disconnectMicroservice, listProjectMicroservices, getProjectStatsSummary,
    listMicroserviceProjects, listParentsOfMicroservice, serverRepoOfMicroservice,
  } from '$lib/api/tauri-commands';
  import type { Project, Repository } from '$lib/types';
  import { addToast } from '$lib/stores/ui';

  const roleKeys: Role[] = ['server', 'admin_client', 'client', 'test_client', 'landing', 'tool', 'other'];
  const roles = roleKeys.map((key) => [key, getRoleLabel(key)] as [Role, string]);

  // Find the current project reactively
  const project = $derived($projects.find((p) => p.id === $selectedProjectId) ?? null);
  const rolePriority: Record<string, number> = {
    server: 0, admin_client: 1, client: 2, test_client: 3,
    landing: 4, tool: 5,
  };
  const projectRepos = $derived(
    project
      ? [...$allRepos.filter((r) => r.project_id === project.id)].sort((a, b) => {
          const pa = rolePriority[a.role ?? ''] ?? 99;
          const pb = rolePriority[b.role ?? ''] ?? 99;
          if (pa !== pb) return pa - pb;
          return (a.github_name ?? a.description ?? '').localeCompare(b.github_name ?? b.description ?? '');
        })
      : []
  );

  const reposLookup = $derived.by(() => {
    const map = new Map<number, Repository>();
    for (const r of $allRepos ?? []) map.set(r.id, r);
    return map;
  });

  // F-012: list of all microservice-type projects (for connection dropdown)
  let allMicroserviceProjects = $state<Project[]>([]);

  // Connected microservice-project IDs for current project
  let connectedMicroserviceIds = $state<number[]>([]);

  // Parent-projects of current microservice-project (if applicable)
  let parentsOfMicroservice = $state<Project[]>([]);

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

  async function loadParents() {
    if (!project || project.project_type !== 'microservice') {
      parentsOfMicroservice = [];
      return;
    }
    try {
      parentsOfMicroservice = await listParentsOfMicroservice(project.id);
    } catch (err) {
      parentsOfMicroservice = [];
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
    if (project) {
      loadMicroservices();
      loadParents();
    }
  });

  $effect(() => {
    // Load server-repo info for every listed microservice-project
    for (const ms of allMicroserviceProjects) {
      if (!(ms.id in msServerRepoCache)) {
        loadServerRepoFor(ms.id);
      }
    }
  });

  // Project stats
  let projectSummary = $state<StatsSummaryData | null>(null);

  $effect(() => {
    if (project) {
      getProjectStatsSummary(project.id).then(s => { projectSummary = s; }).catch(() => { projectSummary = null; });
    } else {
      projectSummary = null;
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

  // F-013/T-055: tabs (mirror RepoDetail pattern)
  type ProjectTab = 'repos' | 'microservices' | 'graph' | 'stats';
  let activeTab = $state<ProjectTab>('repos');

  // Reset to default tab when switching projects
  $effect(() => {
    void $selectedProjectId;
    activeTab = 'repos';
  });

  async function handleUnassignRepo(repoId: number) {
    await assignRepo(repoId, null, null);
    await loadAllRepos();
  }

  async function handleRoleChange(repoId: number, value: string) {
    const role = value === '' ? null : value;
    if (!project) return;
    await assignRepo(repoId, project.id, role);
    await loadAllRepos();
  }

  function openSync() {
    currentScreen.set({ name: 'sync' });
  }

  function openRepo(id: number) {
    selectedRepoId.set(id);
    currentScreen.set({ name: 'repo-detail' });
  }

  function getRoleIcon(role: string | null): string {
    if (!role) return '';
    return ROLE_ICONS[role as Role] ?? '';
  }
</script>

<div class="project-detail" oncontextmenu={(e) => e.preventDefault()} role="application">
  {#if !project}
    <div class="not-found">
      <p>{$tStore('project.notFound')}</p>
    </div>
  {:else}
    <ProjectHeader {project} hasParents={parentsOfMicroservice.length > 0} />

    <div class="project-tabs">
      <button class="tab-btn" class:active={activeTab === 'repos'} onclick={() => (activeTab = 'repos')}>
        {$tStore('project.tabRepos' as any)}
      </button>
      <button class="tab-btn" class:active={activeTab === 'microservices'} onclick={() => (activeTab = 'microservices')}>
        {$tStore('project.tabMicroservices' as any)}
      </button>
      <button class="tab-btn" class:active={activeTab === 'graph'} onclick={() => (activeTab = 'graph')}>
        {$tStore('project.tabGraph' as any)}
      </button>
      <button class="tab-btn" class:active={activeTab === 'stats'} onclick={() => (activeTab = 'stats')}>
        {$tStore('project.tabStats' as any)}
      </button>
    </div>

    {#if activeTab === 'repos'}
    <div class="repos-section">
      <div class="tab-toolbar">
        <span class="tab-count">{projectRepos.length}</span>
        <button class="sync-nav-btn" onclick={openSync} title={$tStore('sync.openSync')}>
          🔄 {$tStore('sync.openSync')}
        </button>
      </div>

      {#if projectRepos.length === 0}
        <div class="empty-repos">
          <div class="empty-icon">📦</div>
          <div class="empty-title">{$tStore('project.noRepos')}</div>
          <div class="empty-hint">{$tStore('project.noReposHint')}</div>
        </div>
      {:else}
        <div class="repo-table-wrapper">
          <table class="repo-table">
            <thead>
              <tr>
                <th>{$tStore('project.colRepo' as any)}</th>
                <th>{$tStore('project.colLang' as any)}</th>
                <th>{$tStore('project.setRole')}</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {#each projectRepos as repo (repo.id)}
                <tr>
                  <td>
                    <button class="repo-link" onclick={() => openRepo(repo.id)}>
                      {#if repo.role}
                        <span class="role-icon" title={repo.role}>{getRoleIcon(repo.role)}</span>
                      {/if}
                      {getDisplayName(repo)}
                    </button>
                  </td>
                  <td>
                    {#if repo.language}
                      <span class="lang-badge">{repo.language}</span>
                    {:else}
                      <span class="muted">—</span>
                    {/if}
                  </td>
                  <td>
                    <select
                      value={repo.role ?? ''}
                      onchange={(e) =>
                        handleRoleChange(repo.id, (e.target as HTMLSelectElement).value)}
                      title={$tStore('project.setRole')}
                    >
                      {#each roles as [key, label] (key)}
                        <option value={key}>{label}</option>
                      {/each}
                    </select>
                  </td>
                  <td>
                    <button
                      class="ghost unassign-btn"
                      onclick={() => handleUnassignRepo(repo.id)}
                      title={$tStore('project.unassignRepo')}
                    >
                      ×
                    </button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>
    {/if}

    {#if activeTab === 'microservices'}
    <!-- Microservices section (connecting other microservice-projects to this one) -->
    <div class="microservices-section">
      <div class="tab-toolbar">
        <span class="tab-count">{allMicroserviceProjects.filter(ms => ms.id !== project.id).length}</span>
      </div>

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

    <!-- F-012: for microservice-projects, show connected parents -->
    {#if project.project_type === 'microservice'}
      <div class="microservices-section">
        <div class="section-label">{$tStore('project.connectedParents' as any)} <span class="tab-count">{parentsOfMicroservice.length}</span></div>
        {#if parentsOfMicroservice.length === 0}
          <div class="empty-repos">
            <div class="empty-title">{$tStore('project.connectedParentsEmpty' as any)}</div>
          </div>
        {:else}
          <div class="microservice-list">
            {#each parentsOfMicroservice as parent (parent.id)}
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
    {/if}

    {#if activeTab === 'graph'}
      <ProjectGraph projectId={project.id} />
    {/if}

    {#if activeTab === 'stats'}
      <StatsSummary summary={projectSummary} scope="project" {reposLookup} />
      <RecentActivityFeed scope="project" scopeId={project.id} />
    {/if}
  {/if}
</div>

<style>
  .project-detail {
    padding: 24px 28px;
    height: 100%;
    overflow-y: auto;
  }

  .project-detail > * + * {
    margin-top: 20px;
  }

  .not-found {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding-top: 60px;
    color: var(--text-muted);
  }

  .sync-nav-btn {
    font-size: 12px;
    padding: 4px 12px;
    border-radius: 4px;
    border: 1px solid var(--accent);
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    white-space: nowrap;
  }

  .sync-nav-btn:hover {
    background-color: var(--accent);
    color: white;
  }

  .ms-server-info {
    font-size: 11px;
    color: var(--text-muted);
    font-family: monospace;
    flex: 1;
    text-align: center;
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }

  .repos-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1;
    overflow: hidden;
  }

  .empty-repos {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 32px 16px;
    color: var(--text-muted);
  }

  .empty-icon {
    font-size: 28px;
    opacity: 0.5;
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

  .repo-table-wrapper {
    overflow-y: auto;
    flex: 1;
  }

  .repo-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }

  .repo-table thead th {
    text-align: left;
    padding: 6px 10px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
  }

  .repo-table tbody tr {
    border-bottom: 1px solid var(--border);
  }

  .repo-table tbody tr:hover {
    background-color: var(--surface);
  }

  .repo-table td {
    padding: 7px 10px;
    vertical-align: middle;
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

  .role-icon {
    font-size: 11px;
    flex-shrink: 0;
  }

  .lang-badge {
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 10px;
    background-color: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-muted);
    white-space: nowrap;
  }

  .muted {
    color: var(--text-muted);
    opacity: 0.5;
  }

  .unassign-btn {
    font-size: 14px;
    color: var(--text-muted);
    padding: 2px 6px;
    opacity: 0.5;
    transition: opacity 0.1s;
  }

  .unassign-btn:hover {
    opacity: 1;
    color: var(--danger) !important;
  }

  select {
    font-size: 12px;
    padding: 3px 6px;
    min-width: 110px;
  }

  .project-tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    margin: 16px 0 12px;
    flex-shrink: 0;
  }
  .tab-toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 8px;
  }
  .tab-count {
    font-size: 12px;
    color: var(--text-muted);
    background: var(--surface);
    padding: 2px 8px;
    border-radius: 10px;
    border: 1px solid var(--border);
  }
  .tab-btn {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    padding: 8px 12px;
    font-size: 13px;
    color: var(--text-muted);
    cursor: pointer;
  }
  .tab-btn:hover { color: var(--text); }
  .tab-btn.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
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
