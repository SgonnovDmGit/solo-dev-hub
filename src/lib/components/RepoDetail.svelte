<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';
  import { allRepos, assignRepo, loadAllRepos } from '$lib/stores/repos';
  import { projects } from '$lib/stores/projects';
  import { currentScreen, selectedRepoId, deployDrillTarget } from '$lib/stores/ui';
  import { loadBugsForRepo as storeLoadBugsForRepo, clearBugs } from '$lib/stores/bugs';
  import { setRepoLocalPath, getRepoStatsSummary, deleteRepository, setDeployTarget, listTemplateLanguages, initDocsForRepo, updateRepoDescription, listRenamesForRepo, checkGitAvailableForRepo, getAutocommitBranch, setAutocommitBranch } from '$lib/api/tauri-commands';
  import type { RepoRename } from '$lib/types';
  import { deleteRepoOnGitHub, splitRepoFullName, listBranches, type BranchInfo } from '$lib/api/github';
  import { addToast } from '$lib/stores/ui';
  import { getRoleLabel, getDisplayName, type Role } from '$lib/types';
  import type { StatsSummary as StatsSummaryData } from '$lib/types';
  import { tStore, locale } from '$lib/i18n';
  import { pat } from '$lib/stores/settings';
  import BugNotes from './BugNotes.svelte';
  import EmptyState from './EmptyState.svelte';
  import StatsSummary from './StatsSummary.svelte';
  import RecentActivityFeed from './RecentActivityFeed.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import SecretsPanel from './SecretsPanel.svelte';
  import TasksTab from './TasksTab.svelte';
  import DoneTab from './DoneTab.svelte';
  import { syncTasksForRepo } from '$lib/api/tauri-commands';
  import RepoChangelogTab from './RepoChangelogTab.svelte';
  import DeployScreen from './DeployScreen.svelte';
  import UntrackGitignoredDialog from './UntrackGitignoredDialog.svelte';

  const roleKeys: Role[] = ['server', 'admin_client', 'client', 'test_client', 'landing', 'tool', 'other'];
  const roles = roleKeys.map((key) => [key, getRoleLabel(key)] as [Role, string]);

  const repo = $derived($allRepos.find((r) => r.id === $selectedRepoId) ?? null);

  // T-050 v0.21.0: inline-edit display name for local-only repos + rename history list
  const isLocalOnly = $derived(repo ? repo.github_name === null : false);
  let editingName = $state(false);
  let editNameValue = $state('');
  let renames = $state<RepoRename[]>([]);
  // T-000137: per-repo auto-commit target branch for synced cross-repo files (empty = disabled).
  let autocommitBranch = $state('');
  // B-000028: branch suggestions for the auto-commit field (datalist). Fetched
  // from GitHub — needs a PAT and a full `owner/repo` name; the field stays a
  // free-text input, so an empty list just means no autocomplete (offline / no PAT).
  let branches = $state<BranchInfo[]>([]);

  async function loadRenames() {
    if (!repo) { renames = []; return; }
    try {
      const list = await listRenamesForRepo(repo.id);
      // db.rs::list_renames_for_repo returns ASC by id (chronological — required for sync replay).
      // For UI display we want freshest first → reverse on frontend.
      renames = [...list].reverse();
    } catch (e) {
      console.warn('rename history load failed', e);
      renames = [];
    }
  }

  async function loadAutocommitBranch() {
    if (!repo) { autocommitBranch = ''; return; }
    try {
      autocommitBranch = (await getAutocommitBranch(repo.id)) ?? '';
    } catch {
      autocommitBranch = '';
    }
  }

  // B-000028: fetch the repo's GitHub branches to populate the auto-commit
  // datalist. Mirrors DeployDetail's DEPLOY_BRANCH fetch — silent on failure so
  // the free-text field keeps working without a PAT / when offline.
  async function loadBranches() {
    branches = [];
    if (!repo || !$pat || !repo.github_name || !repo.github_name.includes('/')) return;
    const { owner, repo: name } = splitRepoFullName(repo.github_name);
    try {
      branches = await listBranches($pat, owner, name);
    } catch {
      branches = [];
    }
  }

  // B-000028: load the saved auto-commit branch + branch list, then default an
  // empty auto-commit target to `dev` when the repo has one (the integration
  // branch for this workflow). Persisted so auto-commit is truly enabled, not
  // just displayed. Guarded against a repo switch mid-load.
  async function loadAutocommitState() {
    const rid = repo?.id ?? null;
    await loadAutocommitBranch();
    await loadBranches();
    if (repo?.id !== rid || !repo) return;
    if (autocommitBranch === '' && branches.some((b) => b.name === 'dev')) {
      autocommitBranch = 'dev';
      try {
        await setAutocommitBranch(repo.id, 'dev');
      } catch (err) {
        console.warn('autocommit dev-default persist failed', err);
      }
    }
  }

  $effect(() => {
    void $selectedRepoId;
    loadRenames();
    void loadAutocommitState();
  });

  function startEditName() {
    if (!isLocalOnly || !repo) return;
    editNameValue = repo.description ?? '';
    editingName = true;
  }
  function cancelEditName() { editingName = false; editNameValue = ''; }
  async function saveEditName() {
    if (!repo) return;
    const newDesc = editNameValue.trim();
    if (!newDesc || newDesc === (repo.description ?? '')) { editingName = false; return; }
    try {
      await updateRepoDescription(repo.id, newDesc);
      await loadAllRepos();
      await loadRenames();
    } finally {
      editingName = false;
    }
  }
  function fmtRenameDate(iso: string): string { return iso.slice(0, 10); }
  function autoFocus(node: HTMLInputElement) { node.focus(); node.select(); }

  function formatDate(iso: string | null): string {
    if (!iso) return '—';
    return new Date(iso).toLocaleDateString($locale === 'ru' ? 'ru-RU' : 'en-US', {
      year: 'numeric', month: 'short', day: 'numeric',
    });
  }

  async function handleProjectChange(e: Event) {
    if (!repo) return;
    const val = (e.target as HTMLSelectElement).value;
    await assignRepo(repo.id, val === '' ? null : Number(val), repo.role);
    await loadAllRepos();
  }

  async function handleRoleChange(e: Event) {
    if (!repo) return;
    const val = (e.target as HTMLSelectElement).value;
    const role = val === '' ? null : val;
    const result = await assignRepo(repo.id, repo.project_id, role);
    if (!result) {
      (e.target as HTMLSelectElement).value = repo.role ?? 'other';
    }
    await loadAllRepos();
  }

  // T-000137: save the auto-commit branch (empty → null disables auto-commit for this repo).
  async function handleAutocommitBranchChange(e: Event) {
    if (!repo) return;
    const val = (e.target as HTMLInputElement).value.trim();
    autocommitBranch = val;
    try {
      await setAutocommitBranch(repo.id, val === '' ? null : val);
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function handleSetLocalPath() {
    if (!repo) return;
    const selected = await open({ directory: true, title: $tStore('repo.selectFolder' as any) });
    if (selected) {
      await setRepoLocalPath(repo.id, selected as string);
      await loadAllRepos();
    }
  }

  async function loadBugsForRepo() {
    if (!repo) {
      clearBugs();
      return;
    }
    const hasLocalPath = Boolean(repo.local_path);
    await storeLoadBugsForRepo(repo.id, hasLocalPath);
  }

  // Reload bugs when selected repo changes. v0.16.0: DB-centric, no flush needed
  // (create/resolve/reject/update commands persist immediately via SQLite).
  $effect(() => {
    if ($selectedRepoId) {
      loadBugsForRepo();
    }
  });

  // v0.20.0: sync tasks once per repo on mount. TasksTab/DoneTab lazy-read DB
  // afterwards. Idempotent — first call marks tasks_migrated_at; subsequent
  // calls diff against DB and emit events for changes.
  $effect(() => {
    if ($selectedRepoId != null) {
      syncTasksForRepo($selectedRepoId).catch((e) => console.warn('sync_tasks failed', e));
    }
  });

  // Stats state (v0.16.0: live-computed from bugs table via VIEW; no manual reset)
  let repoSummary = $state<StatsSummaryData | null>(null);

  // B-003: delete repo state
  let showDeleteConfirm = $state(false);
  let deleteOptGitHub = $state(false);
  let deleteOptClearGit = $state(false);
  let deleteTypedName = $state('');

  // B-003: deploy target state
  let deployTargetOptions = $state<string[]>([]);

  // F-000041: gate untrack button on git CLI + .git/ availability.
  // Default false → button not rendered until backend confirms both.
  let canUntrack = $state(false);
  let showUntrackDialog = $state(false);

  // B-000015 follow-up: do NOT reset canUntrack synchronously on repo change —
  // that caused the untrack button to flash out + back in on every sidebar
  // click (and the init-docs button next to it visually jittered from the
  // flex-row reflow). Keep the previous value until the async check resolves.
  // Stale-response guard (`repo?.id === repoId`) protects against the case
  // where user clicks repo A → repo B and A's response arrives after B's.
  $effect(() => {
    if (!repo) {
      canUntrack = false;
      return;
    }
    const repoId = repo.id;
    checkGitAvailableForRepo(repoId)
      .then((v) => {
        if (repo?.id === repoId) canUntrack = v;
      })
      .catch(() => {
        if (repo?.id === repoId) canUntrack = false;
      });
  });

  // F-021: tabs in RepoDetail. T-000080: Deploy moved from separate screen to tab.
  type Tab = 'bugs' | 'tasks' | 'done' | 'changelog' | 'deploy' | 'secrets' | 'stats';
  let activeTab = $state<Tab>('bugs');

  // v1.2.0 deploy-report drill-down: when DeployReport arms deployDrillTarget
  // for this repo, jump to the Deploy tab. DeployScreen (keyed by repo.id) then
  // consumes the target to open that env's detail and clears it. We only switch
  // the tab here — clearing is DeployScreen's job so it sees the target on mount.
  $effect(() => {
    const target = $deployDrillTarget;
    if (target && repo && target.repoId === repo.id) {
      activeTab = 'deploy';
    }
  });

  $effect(() => {
    listTemplateLanguages()
      .then((langs) => {
        // `_global` — это шаблоны общих файлов (.gitignore, todo.md skeleton и т.п.),
        // не deploy target. Отфильтровываем из выпадашки.
        deployTargetOptions = langs
          .map(l => l.language_key)
          .filter(k => k !== '_global');
      })
      .catch(() => { deployTargetOptions = []; });
  });

  async function handleDeployTargetChange(e: Event) {
    if (!repo) return;
    const val = (e.target as HTMLSelectElement).value;
    await setDeployTarget(repo.id, val === '' ? null : val);
    await loadAllRepos();
  }

  async function handleInitDocs() {
    if (!repo) return;
    try {
      const created = await initDocsForRepo(repo.id);
      if (created.length === 0) {
        addToast($tStore('toast.docsAlreadyExist' as any), 'info');
      } else {
        addToast(
          $tStore('toast.docsInitialized' as any).replace('{0}', created.join(', ')),
          'success'
        );
      }
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  let deleting = $state(false);

  function openDeleteDialog() {
    deleteOptGitHub = false;
    deleteOptClearGit = false;
    deleteTypedName = '';
    showDeleteConfirm = true;
  }

  async function handleDeleteRepo() {
    if (!repo) return;
    const name = getDisplayName(repo);
    if (deleteTypedName !== name) return;

    deleting = true;
    try {
      // 1) Delete on GitHub (if requested & PAT available)
      if (deleteOptGitHub && $pat && repo.github_name && repo.github_name.includes('/')) {
        try {
          const { owner, repo: repoName } = splitRepoFullName(repo.github_name);
          await deleteRepoOnGitHub($pat, owner, repoName);
        } catch (err: any) {
          addToast($tStore('toast.githubDeleteFailed' as any).replace('{0}', String(err?.message ?? err)), 'error');
        }
      }
      // 2) DB + optional local .git cleanup
      try {
        await deleteRepository(repo.id, deleteOptClearGit, repo.local_path);
      } catch (err: any) {
        addToast($tStore('toast.repoDeleteFailed' as any).replace('{0}', name).replace('{1}', String(err)), 'error');
        return;
      }
      addToast($tStore('toast.repoDeleted' as any).replace('{0}', name), 'success');
      // 3) Refresh + navigate back
      await loadAllRepos();
      currentScreen.set({ name: 'dashboard' });
    } finally {
      deleting = false;
      showDeleteConfirm = false;
    }
  }

  $effect(() => {
    if (repo && repo.local_path) {
      getRepoStatsSummary(repo.id).then(s => { repoSummary = s; }).catch(() => { repoSummary = null; });
    } else {
      repoSummary = null;
    }
  });

</script>

<div class="repo-detail">
  {#if !repo}
    <EmptyState icon="📂" title={$tStore('repoDetail.notFound')} hint={$tStore('repoDetail.notFoundHint')} />
  {:else}
    <!-- Sticky header (C-variant: 2-row compact, no back-btn, no top-right badges) -->
    <div class="sticky-header">
      <h2 class="repo-name">
        {#if editingName && isLocalOnly}
          <input
            class="repo-name-input"
            type="text"
            bind:value={editNameValue}
            onkeydown={(e) => { if (e.key === 'Enter') saveEditName(); if (e.key === 'Escape') cancelEditName(); }}
            onblur={saveEditName}
            use:autoFocus
          />
        {:else if isLocalOnly}
          <button class="ghost repo-name-btn" onclick={startEditName} title={$tStore('repo.editName' as any)}>
            {getDisplayName(repo)}
          </button>
        {:else if repo.github_url}
          <a href={repo.github_url} target="_blank" title={$tStore('repoDetail.openOnGitHub')}>{getDisplayName(repo)}</a>
        {:else}
          {getDisplayName(repo)}
        {/if}
      </h2>

      {#if renames.length > 0}
        <div class="rename-history">
          {#each renames as r (r.id)}
            <div class="rename-entry">
              {$tStore('repo.renameHistoryPrev' as any)} <code>{r.old_canonical}</code> ({fmtRenameDate(r.renamed_at)})
            </div>
          {/each}
        </div>
      {/if}

      <!-- Row 1: lang + last-pushed + path + folder-btn, action прижата вправо -->
      <div class="meta-row">
        {#if repo.language}
          <span class="lang-badge">{repo.language}</span>
        {/if}
        <span class="meta-text">{$tStore('repoDetail.labelLastPushed')}: {formatDate(repo.last_pushed_at)}</span>
        <span class="meta-dot">·</span>
        {#if repo.local_path}
          <span class="local-path ok" title={repo.local_path}>📁 {repo.local_path}</span>
        {:else}
          <span class="local-path warn">⚠ {$tStore('repo.localPathNotFound' as any)}</span>
        {/if}
        <button class="ghost mini" onclick={handleSetLocalPath} type="button">{$tStore('repo.localPathSet' as any)}</button>
        <button class="init-docs-btn row-action" onclick={handleInitDocs} type="button" title={$tStore('repo.initDocsButton' as any)}>
          <span class="btn-icon">📚</span>
          <span class="btn-label">{$tStore('repo.initDocsButton' as any)}</span>
        </button>
        {#if canUntrack}
          <button class="untrack-gitignored-btn row-action" onclick={() => (showUntrackDialog = true)} type="button" title={$tStore('repo.untrackGitignoredButton' as any)}>
            <span class="btn-icon">🧹</span>
            <span class="btn-label">{$tStore('repo.untrackGitignoredButton' as any)}</span>
          </button>
        {/if}
      </div>

      <!-- Row 2: editable chips (Project / Role / Шаблон), Delete прижат вправо -->
      <div class="chips-row">
        <div class="chip">
          <span class="chip-label">{$tStore('repoDetail.labelProject' as any)}:</span>
          <select class="chip-select" value={repo.project_id ?? ''} onchange={handleProjectChange} title={$tStore('repoDetail.assignToProject')}>
            <option value="">{$tStore('repoDetail.unassigned')}</option>
            {#each $projects as project (project.id)}
              <option value={project.id}>{project.name}</option>
            {/each}
          </select>
        </div>

        <div class="chip">
          <span class="chip-label">{$tStore('repoDetail.labelRole' as any)}:</span>
          <select class="chip-select" value={repo.role ?? 'other'} onchange={handleRoleChange} title={$tStore('repoDetail.setRole')}>
            {#each roles as [key, label] (key)}
              <option value={key}>{label}</option>
            {/each}
          </select>
        </div>

        <div class="chip">
          <span class="chip-label">{$tStore('repo.deployTarget' as any)}:</span>
          <select class="chip-select" value={repo.deploy_target ?? ''} onchange={handleDeployTargetChange}>
            <option value="">{$tStore('repo.deployTargetNone' as any)}</option>
            {#each deployTargetOptions as lang (lang)}
              <option value={lang}>{lang}</option>
            {/each}
          </select>
        </div>

        <div class="chip">
          <span class="chip-label">{$tStore('repoDetail.labelAutocommit' as any)}:</span>
          <input
            class="chip-input"
            type="text"
            list="autocommit-branch-list"
            value={autocommitBranch}
            onchange={handleAutocommitBranchChange}
            placeholder={$tStore('repoDetail.autocommitPlaceholder' as any)}
            title={$tStore('repoDetail.autocommitHint' as any)}
          />
          {#if branches.length > 0}
            <datalist id="autocommit-branch-list">
              {#each branches as b (b.name)}
                <option value={b.name}>{b.isDefault ? `${b.name} (default)` : ''}</option>
              {/each}
            </datalist>
          {/if}
        </div>

        <button class="delete-repo-btn row-action" onclick={openDeleteDialog} type="button" disabled={deleting} title={$tStore('repo.deleteButton' as any)}>
          <span class="btn-icon">🗑</span>
          <span class="btn-label">{$tStore('repo.deleteButton' as any)}</span>
        </button>
      </div>
    </div>

    <!-- F-021: Tabs navigation. Order: Bugs → Tasks → Secrets → Stats (B-003). -->
    <div class="tabs-nav">
      <button
        class="tab-btn"
        class:active={activeTab === 'bugs'}
        onclick={() => (activeTab = 'bugs')}
        type="button"
      >{$tStore('repo.tabBugs' as any)}</button>
      <button
        class="tab-btn"
        class:active={activeTab === 'tasks'}
        onclick={() => (activeTab = 'tasks')}
        type="button"
      >{$tStore('tasks.tabTitle' as any)}</button>
      <button
        class="tab-btn"
        class:active={activeTab === 'done'}
        onclick={() => (activeTab = 'done')}
        type="button"
      >{$tStore('done.tabTitle' as any)}</button>
      <button
        class="tab-btn"
        class:active={activeTab === 'changelog'}
        onclick={() => (activeTab = 'changelog')}
        type="button"
      >{$tStore('repo.tabChangelog' as any)}</button>
      <button
        class="tab-btn"
        class:active={activeTab === 'deploy'}
        onclick={() => (activeTab = 'deploy')}
        type="button"
      >{$tStore('repo.tabDeploy' as any)}</button>
      <button
        class="tab-btn"
        class:active={activeTab === 'secrets'}
        onclick={() => (activeTab = 'secrets')}
        type="button"
      >{$tStore('repo.tabSecrets' as any)}</button>
      <button
        class="tab-btn"
        class:active={activeTab === 'stats'}
        onclick={() => (activeTab = 'stats')}
        type="button"
      >{$tStore('repo.tabStats' as any)}</button>
    </div>

    {#if activeTab === 'secrets'}
      <!-- Secrets panel — renamed from "Overview" (tab held only this). -->
      {#if repo.github_name && $pat}
        <div class="secrets-wrapper">
          <!-- B-000025: SecretsPanel keeps per-repo bulk-paste drafts internally
               (module-level map keyed by repoFullName), so no remount is needed —
               switching repos restores the previous draft instead of wiping it. -->
          <SecretsPanel repoFullName={repo.github_name} collapsible={false} />
        </div>
      {/if}
    {:else if activeTab === 'bugs'}
      {#if repo.local_path}
        <BugNotes repoRole={repo.role ?? 'other'} />
      {:else}
        <div class="bugs-blocked">
          <p>{$tStore('repo.bugsBlocked' as any)}</p>
        </div>
      {/if}
    {:else if activeTab === 'tasks'}
      {#if repo.local_path}
        <TasksTab repoId={repo.id} />
      {:else}
        <div class="bugs-blocked">
          <p>{$tStore('repo.bugsBlocked' as any)}</p>
        </div>
      {/if}
    {:else if activeTab === 'done'}
      {#if repo.local_path}
        <DoneTab repoId={repo.id} />
      {:else}
        <div class="bugs-blocked">
          <p>{$tStore('repo.bugsBlocked' as any)}</p>
        </div>
      {/if}
    {:else if activeTab === 'changelog'}
      {#if repo.local_path}
        <RepoChangelogTab repoId={repo.id} />
      {:else}
        <div class="bugs-blocked">
          <p>{$tStore('repo.bugsBlocked' as any)}</p>
        </div>
      {/if}
    {:else if activeTab === 'deploy'}
      {#if repo.github_name && repo.deploy_target}
        <div class="deploy-wrapper">
          {#key repo.id}
            <!-- B-000006: DeployScreen drill-down state (viewMode='detail',
                 selectedDeployEnvId) lives in component-local state. Without a
                 `key` on repo.id the component is reused on repo switch, so a
                 drill-down opened on Repo A keeps rendering Repo A's env after
                 the user clicks Repo B in the sidebar. Keying forces remount.
                 B-000008: parent `.repo-detail` is overflow:hidden, so the
                 Deploy tab needs its own scroll container (mirrors the
                 `.secrets-wrapper` / `.stats-section` pattern). Without this
                 wrapper, many secrets/envs are clipped below the viewport. -->
            <DeployScreen />
          {/key}
        </div>
      {:else}
        <div class="bugs-blocked">
          <p>{$tStore('repo.deployBlocked' as any)}</p>
        </div>
      {/if}
    {:else if activeTab === 'stats'}
      {#if repo.local_path}
        <div class="stats-section">
          <div class="stats-header">
            <span class="stats-title">{$tStore('stats.title' as any)}</span>
          </div>
          <StatsSummary summary={repoSummary} scope="repo" />
          <RecentActivityFeed scope="repo" scopeId={repo.id} />
        </div>
      {:else}
        <div class="bugs-blocked">
          <p>{$tStore('repo.bugsBlocked' as any)}</p>
        </div>
      {/if}
    {/if}

  {/if}
</div>

{#if showDeleteConfirm && repo}
  <ConfirmDialog
    title={$tStore('repo.deleteConfirmTitle' as any).replace('{0}', getDisplayName(repo))}
    message={$tStore('repo.deleteConfirmMessage' as any)}
    onConfirm={handleDeleteRepo}
    onCancel={() => (showDeleteConfirm = false)}
  >
    <div class="delete-options">
      <label class="opt disabled-opt">
        <input type="checkbox" checked disabled />
        {$tStore('repo.deleteOptFromApp' as any)}
      </label>
      <label class="opt">
        <input type="checkbox" bind:checked={deleteOptGitHub} disabled={!$pat} />
        {$tStore('repo.deleteOptFromGitHub' as any)}
        <span class="opt-warn">({$tStore('repo.deleteOptGitHubWarning' as any)})</span>
      </label>
      <label class="opt" class:disabled-opt={!repo.local_path}>
        <input type="checkbox" bind:checked={deleteOptClearGit} disabled={!repo.local_path} />
        {$tStore('repo.deleteOptClearGit' as any)}
      </label>
      <div class="confirm-type">
        <div class="confirm-type-label">
          {$tStore('repo.deleteTypeName' as any).replace('{0}', getDisplayName(repo))}
        </div>
        <input
          class="confirm-type-input"
          type="text"
          bind:value={deleteTypedName}
          placeholder={$tStore('repo.deleteTypePlaceholder' as any)}
        />
      </div>
    </div>
  </ConfirmDialog>
{/if}

{#if showUntrackDialog && repo}
  <UntrackGitignoredDialog
    repositoryId={repo.id}
    onClose={() => (showUntrackDialog = false)}
  />
{/if}

<style>
  .repo-detail {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* F-021: tabs nav */
  .tabs-nav {
    display: flex;
    gap: 0;
    padding: 0 24px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
  }
  .tab-btn {
    padding: 8px 16px;
    font-size: 12px;
    color: var(--text-muted);
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    cursor: pointer;
    transition: color 0.15s, border-color 0.15s;
  }
  .tab-btn:hover {
    color: var(--text);
  }
  .tab-btn.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }

  .sticky-header {
    flex-shrink: 0;
    padding: 12px 24px;
    container-type: inline-size;
    container-name: repo-header;
    border-bottom: 1px solid var(--border);
    background-color: var(--bg);
  }

  .repo-name {
    font-size: 18px;
    font-weight: 700;
    margin: 0 0 4px 0;
  }

  .repo-name a {
    color: var(--accent);
    text-decoration: none;
  }

  .repo-name a:hover {
    text-decoration: underline;
  }
  .repo-name-btn {
    background: none;
    border: none;
    padding: 0;
    margin: 0;
    font-size: 18px;
    font-weight: 700;
    color: var(--text);
    cursor: pointer;
    text-align: left;
  }
  .repo-name-btn:hover { color: var(--accent); }
  .repo-name-input {
    font-size: 18px;
    font-weight: 700;
    padding: 2px 6px;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent);
    border-radius: 4px;
  }

  .rename-history {
    margin: 4px 0 8px;
    padding: 0 0 0 4px;
  }
  .rename-entry {
    font-size: 11px;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .rename-entry code {
    font-family: monospace;
    color: var(--text);
    background: var(--surface);
    padding: 0 5px;
    border-radius: 3px;
    font-size: 11px;
  }

  .meta-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 12px;
    margin-top: 4px;
  }

  .meta-text {
    color: var(--text-muted);
  }

  .meta-dot {
    color: var(--text-muted);
    opacity: 0.5;
  }

  .lang-badge {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 10px;
    background-color: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }

  .chips-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 12px;
    padding-top: 6px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    transition: border-color 150ms, background 150ms;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .chip:hover { border-color: var(--accent); background: var(--surface-hover); }
  .chip-label {
    color: var(--text-muted);
    font-size: 11px;
    white-space: nowrap;
  }
  .chip-select {
    background: transparent;
    color: var(--text);
    border: 0;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    padding: 2px 0;
    font-family: inherit;
    outline: none;
  }
  /* T-000137: auto-commit branch text input, styled to sit inside a chip. */
  .chip-input {
    background: transparent;
    color: var(--text);
    border: 0;
    font-size: 12px;
    font-weight: 500;
    padding: 2px 0;
    font-family: inherit;
    outline: none;
    width: 96px;
  }
  .chip-input::placeholder {
    color: var(--text-muted);
    font-weight: 400;
  }

  .row-action {
    margin-left: auto;
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .btn-icon { line-height: 1; }
  .btn-label { white-space: nowrap; }

  /* Adaptive collapse: hide button labels when header gets narrow.
     Threshold tuned so icons appear before the row would wrap. */
  @container repo-header (max-width: 760px) {
    .row-action .btn-label { display: none; }
    .row-action { padding: 4px 8px; }
  }

  .local-path {
    font-size: 11px;
    font-family: monospace;
    /* B-000015: cap visible width so a deeply nested path doesn't push the
       row-action buttons (init-docs / untrack-gitignored) onto the next line.
       Full path is available on hover via the `title` attribute. */
    max-width: 40ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .local-path.ok {
    color: var(--text);
  }

  .local-path.warn {
    color: #f59e0b;
  }

  .mini {
    font-size: 10px;
    padding: 1px 5px;
  }

  .bugs-blocked {
    padding: 16px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
    border: 1px dashed var(--border);
    border-radius: 6px;
  }

  .secrets-wrapper {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 24px;
    /* T-000129 follow-up: flex column so SecretsPanel's `flex: 1` cascade
       reaches the bulk-paste textarea. Without this the .secrets-section.flat
       child has no flex parent, so its `flex: 1` is silently ignored and the
       textarea stays at rows="4". */
    display: flex;
    flex-direction: column;
  }

  /* B-000008: same overflow-scroll pattern as .secrets-wrapper / .stats-section.
     DeployDetail with many secrets needs its own scroll container — parent
     .repo-detail is overflow:hidden, so without this the bottom of the list
     is clipped below the viewport and the user can't scroll to see it. */
  .deploy-wrapper {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .stats-section {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    border-bottom: 1px solid var(--border);
    padding: 6px 24px;
  }

  .stats-header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 6px;
  }

  .stats-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }

  .init-docs-btn {
    font-size: 11px;
    padding: 2px 10px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
  }
  .init-docs-btn:hover { background: var(--surface); border-color: var(--accent); }

  .untrack-gitignored-btn {
    /* Sits immediately to the right of init-docs in meta-row. Init-docs has the
       row-action margin-left: auto and anchors the pair to the right edge;
       untrack overrides to a small fixed gap so it doesn't compete for auto. */
    margin-left: 4px;
    font-size: 11px;
    padding: 2px 10px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    white-space: nowrap;
  }
  .untrack-gitignored-btn:hover { background: var(--surface); border-color: var(--accent); }

  .delete-repo-btn {
    font-size: 11px;
    padding: 2px 8px;
    color: var(--danger, #ef4444);
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
    white-space: nowrap;
    transition: border-color 150ms;
  }
  .delete-repo-btn:hover:not(:disabled) {
    border-color: var(--danger, #ef4444);
  }
  .delete-repo-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }

  .delete-options {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 12px 0;
  }

  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
    padding: 4px 0;
  }

  .opt.disabled-opt {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .opt-warn {
    font-size: 11px;
    color: var(--text-muted);
    margin-left: 4px;
  }

  .confirm-type {
    margin-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .confirm-type-label {
    font-size: 12px;
    color: var(--text-muted);
  }

  .confirm-type-input {
    padding: 6px 8px;
    font-size: 13px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    font-family: monospace;
  }
</style>
