<script lang="ts">
  import { selectedProjectId, currentScreen, addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import {
    connectMicroservice, disconnectMicroservice, listProjectMicroservices,
    listMicroserviceProjects, serverRepoOfMicroservice,
  } from '$lib/api/tauri-commands';
  import type { Project, Repository } from '$lib/types';

  interface Props {
    project: Project;
    parents: Project[];
  }
  let { project, parents }: Props = $props();

  // F-012: list of all microservice-type projects (for connection dropdown)
  let allMicroserviceProjects = $state<Project[]>([]);

  // Connected microservice-project IDs for current project
  let connectedMicroserviceIds = $state<number[]>([]);

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

  async function loadServerRepoFor(msProjectId: number) {
    try {
      const repo = await serverRepoOfMicroservice(msProjectId);
      msServerRepoCache = { ...msServerRepoCache, [msProjectId]: { repo, error: null } };
    } catch (err: any) {
      msServerRepoCache = { ...msServerRepoCache, [msProjectId]: { repo: null, error: String(err) } };
    }
  }

  $effect(() => {
    if (project) loadMicroservices();
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
    <div class="section-label">{$tStore('project.connectedParents' as any)} <span class="tab-count">{parents.length}</span></div>
    {#if parents.length === 0}
      <div class="empty-repos">
        <div class="empty-title">{$tStore('project.connectedParentsEmpty' as any)}</div>
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
