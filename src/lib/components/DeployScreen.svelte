<script lang="ts">
  import { selectedRepoId, currentScreen, addToast } from '$lib/stores/ui';
  import { allRepos } from '$lib/stores/repos';
  import { pat } from '$lib/stores/settings';
  import { tStore } from '$lib/i18n';
  import { getDisplayName } from '$lib/types';
  import type { DeployEnvironment } from '$lib/types';
  import {
    listDeployEnvironments, createDeployEnvironment, cloneDeployEnvironment,
    deleteDeployEnvironment,
  } from '$lib/api/tauri-commands';
  import {
    createEnvironment, deleteEnvironment as ghDeleteEnvironment,
    splitRepoFullName,
  } from '$lib/api/github';
  import DeployDetail from './DeployDetail.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';

  const repo = $derived($allRepos.find((r) => r.id === $selectedRepoId) ?? null);

  type ViewMode = 'list' | 'detail';
  let viewMode = $state<ViewMode>('list');
  let selectedDeployEnvId = $state<number | null>(null);

  // Master view state
  let environments = $state<DeployEnvironment[]>([]);
  let loading = $state(false);

  // + New deployment form state
  let showNewForm = $state(false);
  let newName = $state('');
  let cloneSource = $state<DeployEnvironment | null>(null);
  let creating = $state(false);

  // Delete confirm
  let envToDelete = $state<DeployEnvironment | null>(null);

  async function loadEnvironments() {
    if (!repo) return;
    loading = true;
    try {
      environments = await listDeployEnvironments(repo.id);
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      loading = false;
    }
  }

  function openDetail(id: number) {
    selectedDeployEnvId = id;
    viewMode = 'detail';
  }

  function backToList() {
    selectedDeployEnvId = null;
    viewMode = 'list';
    loadEnvironments();
  }

  function startNewForm() {
    showNewForm = true;
    newName = '';
    cloneSource = null;
  }

  function cancelNewForm() {
    showNewForm = false;
    cloneSource = null;
  }

  function startNewFormCopyFrom(source: DeployEnvironment) {
    showNewForm = true;
    newName = '';
    cloneSource = source;
  }

  async function handleCreate() {
    if (!repo || !repo.github_name || !$pat) {
      addToast($tStore('deploy.githubRequired' as any), 'error');
      return;
    }
    const trimmedName = newName.trim();
    if (!trimmedName) return;
    creating = true;
    try {
      const { owner, repo: repoName } = splitRepoFullName(repo.github_name);
      let created: DeployEnvironment;
      if (cloneSource === null) {
        created = await createDeployEnvironment({
          repository_id: repo.id,
          name: trimmedName,
          workflow_name: `Deploy ${trimmedName}`,
          image_tag: trimmedName,
          compose_service: '',
          domain: '',
          deploy_branch: '',
          extras: {},
        });
      } else {
        created = await cloneDeployEnvironment(cloneSource.id, trimmedName);
      }
      // Create GitHub environment
      await createEnvironment($pat, owner, repoName, trimmedName);
      showNewForm = false;
      addToast(($tStore('deploy.createdToast' as any) || 'Deployment "{0}" created').replace('{0}', trimmedName), 'success');
      await loadEnvironments();
      openDetail(created.id);
    } catch (err: any) {
      addToast(String(err?.message ?? err), 'error');
    } finally {
      creating = false;
    }
  }

  async function handleDelete() {
    if (!envToDelete || !repo || !repo.github_name || !$pat) return;
    const env = envToDelete;
    envToDelete = null;
    try {
      const { owner, repo: repoName } = splitRepoFullName(repo.github_name);
      // GitHub delete first (cascades secrets) — if offline, DB stays consistent
      await ghDeleteEnvironment($pat, owner, repoName, env.name);
      await deleteDeployEnvironment(env.id);
      addToast(($tStore('deploy.deletedToast' as any) || 'Deployment "{0}" deleted').replace('{0}', env.name), 'success');
      await loadEnvironments();
    } catch (err: any) {
      addToast(String(err?.message ?? err), 'error');
    }
  }

  function back() {
    currentScreen.set({ name: 'repo-detail' });
  }

  $effect(() => {
    if (repo && viewMode === 'list') {
      loadEnvironments();
    }
  });
</script>

<div class="deploy-screen">
  {#if !repo}
    <div class="empty"><p>{$tStore('repoDetail.notFound')}</p></div>
  {:else if viewMode === 'detail' && selectedDeployEnvId != null}
    <DeployDetail
      deployEnvId={selectedDeployEnvId}
      onBack={backToList}
    />
  {:else}
    <div class="header">
      <button class="ghost back-btn" onclick={back} type="button">
        {$tStore('deploy.back' as any)}
      </button>
      <h2>{($tStore('deploy.deploymentsTitle' as any) || 'Deployments: {0}').replace('{0}', getDisplayName(repo))}</h2>
    </div>

    {#if loading}
      <p class="loading">{$tStore('common.loading' as any)}</p>
    {:else if environments.length === 0 && !showNewForm}
      <p class="empty-note">{$tStore('deploy.noDeployments' as any) || 'No deployments yet. Click + to create.'}</p>
    {:else}
      <table class="env-list">
        <thead>
          <tr>
            <th class="copy-col"></th>
            <th>{$tStore('deploy.columnName' as any) || 'Name'}</th>
            <th>{$tStore('deploy.columnBranch' as any) || 'Branch'}</th>
            <th>{$tStore('deploy.columnDomain' as any) || 'Domain'}</th>
            <th>{$tStore('deploy.columnUpdated' as any) || 'Updated'}</th>
            <th class="actions-col"></th>
          </tr>
        </thead>
        <tbody>
          {#each environments as env (env.id)}
            <tr class="env-row" onclick={() => openDetail(env.id)}>
              <td class="copy-cell" onclick={(e) => e.stopPropagation()}>
                <button class="icon copy-btn" onclick={() => startNewFormCopyFrom(env)} aria-label="Duplicate" title="Duplicate">⎘</button>
              </td>
              <td class="name">{env.name}</td>
              <td>{env.deploy_branch || '—'}</td>
              <td>{env.domain || '—'}</td>
              <td class="updated">{env.updated_at.slice(0, 10)}</td>
              <td class="actions" onclick={(e) => e.stopPropagation()}>
                <button class="icon delete" onclick={() => envToDelete = env} aria-label="Delete">✕</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}

    {#if showNewForm}
      <div class="new-form">
        {#if cloneSource}
          <span class="form-prompt">
            {$tStore('deploy.copyAsPrompt' as any) || 'Copy'} <strong>{cloneSource.name}</strong> {$tStore('deploy.copyAsConnector' as any) || 'as:'}
          </span>
        {:else}
          <span class="form-prompt">{$tStore('deploy.newAsPrompt' as any) || 'New deployment:'}</span>
        {/if}
        <input type="text" bind:value={newName} placeholder={$tStore('deploy.namePlaceholder' as any) || 'name (prod, test, …)'} />
        <button class="primary" onclick={handleCreate} disabled={creating || !newName.trim()}>
          {$tStore('deploy.create' as any) || 'Create'}
        </button>
        <button class="cancel-btn" onclick={cancelNewForm} disabled={creating}>{$tStore('common.cancel' as any)}</button>
      </div>
    {:else}
      <button class="primary new-btn" onclick={startNewForm}>
        {$tStore('deploy.newDeployment' as any) || '+ New deployment'}
      </button>
    {/if}
  {/if}
</div>

{#if envToDelete}
  <ConfirmDialog
    title={$tStore('deploy.deleteConfirmTitle' as any) || 'Delete deployment?'}
    message={($tStore('deploy.deleteConfirmBody' as any) || 'This will delete the GitHub environment "{0}" and all its secrets. The workflow file deploy-{0}.yml will be removed on next Generate. Continue?').replace(/\{0\}/g, envToDelete.name)}
    onConfirm={handleDelete}
    onCancel={() => envToDelete = null}
  />
{/if}

<style>
  .deploy-screen { padding: 1rem; }
  .env-list { width: 100%; border-collapse: collapse; margin: 1rem 0; }
  .env-list th { text-align: left; padding: 0.5rem; font-weight: 600; border-bottom: 1px solid var(--border); }
  .env-row { cursor: pointer; }
  .env-row:hover { background: var(--hover-bg); }
  .env-row td { padding: 0.5rem; border-bottom: 1px solid var(--border-light); }
  .env-row .name { font-weight: 600; }
  .env-row .updated { color: var(--text-muted); font-size: 0.85em; }
  .env-row .actions { text-align: right; }
  .env-row .actions button.icon { opacity: 0; transition: opacity 0.15s; background: transparent; border: 0; cursor: pointer; }
  .env-row:hover .actions button.icon { opacity: 1; }
  .copy-col, .actions-col { width: 2.5rem; }
  .copy-cell { text-align: center; padding: 0; }
  .copy-cell .copy-btn {
    background: transparent;
    border: 0;
    cursor: pointer;
    padding: 0.3rem 0.5rem;
    color: var(--text-muted);
    font-size: 1rem;
  }
  .copy-cell .copy-btn:hover { color: var(--text); }
  .new-form {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    padding: 0.6rem 0.75rem;
    background: var(--hover-bg);
    border-radius: 4px;
    flex-wrap: wrap;
    margin-top: 0.5rem;
  }
  .new-form input { padding: 0.4rem 0.6rem; flex: 1; min-width: 12rem; }
  .form-prompt { color: var(--text-muted); font-size: 0.95em; }
  .form-prompt strong { color: var(--text); }
  .new-btn { margin-top: 0.5rem; }
  .empty-note { color: var(--text-muted); }
  .cancel-btn {
    padding: 0.45rem 1rem;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    border-radius: 4px;
    font-weight: 500;
  }
  .cancel-btn:hover:not(:disabled) { background: var(--hover-bg); border-color: var(--text-muted); }
  .cancel-btn:disabled { opacity: 0.4; cursor: not-allowed; }
</style>
