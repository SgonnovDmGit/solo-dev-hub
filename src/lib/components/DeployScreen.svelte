<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import { selectedRepoId, addToast, deployDrillTarget } from '$lib/stores/ui';
  import { allRepos } from '$lib/stores/repos';
  import { pat } from '$lib/stores/settings';
  import { tStore, locale } from '$lib/i18n';
  import type { DeployEnvironment } from '$lib/types';
  import {
    listDeployEnvironments, createDeployEnvironment, cloneDeployEnvironment,
    deleteDeployEnvironment,
    getRepoDeployConfig, setRepoDeployConfig,
    getTemplateFile,
    readRepoFile,
  } from '$lib/api/tauri-commands';
  import { runAutoDetect, type AutoDetectSpec } from '$lib/api/auto-detect';
  import {
    createEnvironment, deleteEnvironment as ghDeleteEnvironment,
    splitRepoFullName,
  } from '$lib/api/github';
  import DeployDetail from './DeployDetail.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';

  const repo = $derived($allRepos.find((r) => r.id === $selectedRepoId) ?? null);

  // B-fix (v1.3.0): snapshot this instance's repo id ONCE at mount. The block is
  // wrapped in {#key repo.id} (one repo per instance), so the id is stable for the
  // instance's lifetime. Must NOT be reactive — `repo` derives from the global
  // $selectedRepoId, so a reactive ownRepoId would re-track the selection and
  // defeat the leak guard. `untrack` reads it once without establishing a dep
  // (also silences the state_referenced_locally false-positive).
  const ownRepoId = untrack(() => repo?.id ?? null);

  type ViewMode = 'list' | 'detail';
  let viewMode = $state<ViewMode>('list');
  let selectedDeployEnvId = $state<number | null>(null);

  // Master view state
  let environments = $state<DeployEnvironment[]>([]);
  let loading = $state(false);

  // ── T-000103 Task 4: shared (repo-wide) image config ─────────────────────────
  // DeployScreen owns `repoConfig` as the single source of truth for
  // placeholder values that render into the repo-wide Dockerfile (scope: "repo"
  // in meta.json). Passed down to DeployDetail as a prop for cross-source
  // empty-required validation (Task 5).
  //
  // Persistence: per-key autosave on `blur`. Mirrors DeploySecretsTable's
  // optimistic-update pattern (B-000009) — no full reload after save, which
  // would replace the state map and cause focus loss when the user tabs.
  interface RepoScopePlaceholder {
    key: string;
    label: string;
    description: string;
    default: string;
    auto_detect?: AutoDetectSpec;
  }
  let repoConfig = $state<Record<string, string>>({});
  let repoScopePlaceholders = $state<RepoScopePlaceholder[]>([]);
  let sharedSectionExpanded = $state(true);
  // Guard key includes deploy_target so template swap re-loads the placeholder
  // set (D6: deploy_repo_config blob preserved across swaps, but the visible
  // input set changes with the new template's meta.json).
  let lastLoadedKey = $state<string | null>(null);

  function extractLocalized(v: any, fallback: string): string {
    if (v == null) return fallback;
    if (typeof v === 'string') return v;
    if (typeof v === 'object') {
      const loc = $locale;
      return v[loc] ?? v.en ?? v.ru ?? fallback;
    }
    return fallback;
  }

  async function loadRepoScopePlaceholders(deployTarget: string | null | undefined): Promise<RepoScopePlaceholder[]> {
    if (!deployTarget) return [];
    try {
      const metaFile = await getTemplateFile(deployTarget, 'meta.json');
      if (!metaFile) return [];
      const meta = JSON.parse(metaFile.content);
      const placeholders = meta?.placeholders;
      if (!placeholders || typeof placeholders !== 'object') return [];
      const out: RepoScopePlaceholder[] = [];
      for (const [key, spec] of Object.entries(placeholders) as [string, any][]) {
        if (spec?.scope === 'repo') {
          out.push({
            key,
            label: extractLocalized(spec?.label, key),
            description: extractLocalized(spec?.description, ''),
            default: typeof spec?.default === 'string' ? spec.default : '',
            auto_detect: spec?.auto_detect && typeof spec.auto_detect === 'object'
              ? (spec.auto_detect as AutoDetectSpec)
              : undefined,
          });
        }
      }
      return out;
    } catch (err) {
      console.warn('Failed to load repo-scope placeholders', err);
      return [];
    }
  }

  async function loadRepoConfig(repoId: number, deployTarget: string | null | undefined) {
    try {
      const [config, scopePlaceholders] = await Promise.all([
        getRepoDeployConfig(repoId),
        loadRepoScopePlaceholders(deployTarget),
      ]);
      repoConfig = config ?? {};
      repoScopePlaceholders = scopePlaceholders;

      // T-000110: run auto_detect for any repo-scope placeholder that has an
      // `auto_detect` spec AND no stored value. Fills the input and persists
      // on first detection — user can override freely afterwards. Re-detect
      // only happens if the user clears the field.
      const detectedChanges = await runAutoDetectForEmpty(repoId, scopePlaceholders);
      if (detectedChanges) {
        try {
          await setRepoDeployConfig(repoId, { ...repoConfig });
        } catch (err) {
          console.warn('Auto-detect persist failed', err);
        }
      }

      // Default state: expanded if any repo-scope placeholder is empty/unset;
      // collapsed if all have values. Computed once on load — user toggles
      // freely thereafter for this session.
      if (scopePlaceholders.length > 0) {
        const anyEmpty = scopePlaceholders.some((p) => {
          const v = (repoConfig[p.key] ?? '').trim();
          return v === '';
        });
        sharedSectionExpanded = anyEmpty;
      } else {
        sharedSectionExpanded = false;
      }
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function runAutoDetectForEmpty(
    repoId: number,
    scopePlaceholders: RepoScopePlaceholder[],
  ): Promise<boolean> {
    let changed = false;
    for (const p of scopePlaceholders) {
      if (!p.auto_detect) continue;
      if ((repoConfig[p.key] ?? '').trim() !== '') continue;
      const detected = await runAutoDetect(
        p.auto_detect,
        (path) => readRepoFile(repoId, path),
      );
      if (detected !== null && detected !== '') {
        repoConfig[p.key] = detected;
        changed = true;
      }
    }
    return changed;
  }

  // Reactive load whenever repo.id or deploy_target changes. Guards with
  // lastLoadedKey so we don't re-trigger on incidental reactivity (e.g. repo
  // object identity changes via $allRepos refresh without actual change).
  $effect(() => {
    if (!repo) return;
    const rid = repo.id;
    const target = repo.deploy_target ?? '';
    const key = `${rid}:${target}`;
    if (lastLoadedKey === key) return;
    lastLoadedKey = key;
    void loadRepoConfig(rid, repo.deploy_target);
  });

  async function saveRepoConfigKey(_key: string) {
    if (ownRepoId == null) return;
    // Guard: if the global selection has already moved to another repo (blur
    // firing during teardown of a repo switch), do NOT write this repo's config
    // — it would leak into the now-selected repo.
    if (get(selectedRepoId) !== ownRepoId) return;
    try {
      await setRepoDeployConfig(ownRepoId, { ...repoConfig });
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

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

  // v1.2.0 deploy-report drill-down: if DeployReport armed a target for this
  // repo, open that env's detail directly. One-shot — cleared after consuming.
  // DeployScreen is keyed by repo.id in RepoDetail, so this fires per repo open.
  onMount(() => {
    const target = get(deployDrillTarget);
    if (target && repo && target.repoId === repo.id) {
      openDetail(target.deployEnvId);
      deployDrillTarget.set(null);
    }
  });

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
      repoConfig={repoConfig}
      onBack={backToList}
    />
  {:else}
    {#if repoScopePlaceholders.length > 0}
      <section class="shared-config" class:collapsed={!sharedSectionExpanded}>
        <button
          class="shared-config-header"
          type="button"
          onclick={() => sharedSectionExpanded = !sharedSectionExpanded}
          aria-expanded={sharedSectionExpanded}
        >
          <span class="chevron" class:expanded={sharedSectionExpanded}>▸</span>
          <span class="shared-config-title">{$tStore('deploy.sharedImageConfig' as any) || 'Shared image config'}</span>
          <span class="shared-config-subtitle">{$tStore('deploy.sharedImageConfigSubtitle' as any) || 'Applied to Dockerfile; same across all envs'}</span>
        </button>
        {#if sharedSectionExpanded}
          <div class="shared-config-body">
            {#each repoScopePlaceholders as p (p.key)}
              {@const inputId = `repo-config-${p.key}`}
              <div class="field" title={p.description}>
                <label for={inputId}>{p.label}:</label>
                <input
                  id={inputId}
                  type="text"
                  value={repoConfig[p.key] ?? ''}
                  placeholder={p.default}
                  oninput={(e) => { repoConfig[p.key] = (e.currentTarget as HTMLInputElement).value; }}
                  onblur={() => saveRepoConfigKey(p.key)}
                />
              </div>
            {/each}
          </div>
        {/if}
      </section>
    {/if}

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
  .env-row:hover { background: var(--surface-hover); }
  .env-row td { padding: 0.5rem; border-bottom: 1px solid var(--border); }
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
    background: var(--surface);
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
  .cancel-btn:hover:not(:disabled) { background: var(--surface-hover); border-color: var(--text-muted); }
  .cancel-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  /* T-000103 Task 4: shared (repo-wide) image config section above env list */
  .shared-config {
    margin-bottom: 1rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--surface);
  }
  .shared-config-header {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    padding: 0.6rem 0.75rem;
    background: transparent;
    border: 0;
    cursor: pointer;
    text-align: left;
    color: var(--text);
    font-family: inherit;
  }
  .shared-config-header:hover {
    background: var(--surface-hover);
  }
  .shared-config.collapsed .shared-config-header {
    border-bottom: 0;
  }
  .chevron {
    display: inline-block;
    transition: transform 0.15s ease;
    color: var(--text-muted);
    font-size: 0.85em;
    flex-shrink: 0;
    width: 0.85em;
  }
  .chevron.expanded {
    transform: rotate(90deg);
  }
  .shared-config-title {
    font-weight: 600;
    font-size: 0.95em;
  }
  .shared-config-subtitle {
    color: var(--text-muted);
    font-size: 0.85em;
  }
  .shared-config-body {
    padding: 0.4rem 0.75rem 0.75rem 0.75rem;
    border-top: 1px solid var(--border);
  }
  .shared-config-body .field {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 0.4rem 0;
  }
  .shared-config-body .field label {
    min-width: 11rem;
    text-align: right;
    flex-shrink: 0;
    font-size: 0.9em;
    color: var(--text);
  }
  .shared-config-body .field input {
    flex: 1;
    padding: 0.4rem;
    box-sizing: border-box;
  }
</style>
