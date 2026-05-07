<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { projects, addProject, loadProjects } from '$lib/stores/projects';
  import { allRepos, isSyncing, syncFromGitHub, assignRepo, loadAllRepos } from '$lib/stores/repos';
  import { pat } from '$lib/stores/settings';
  import { currentScreen, selectedRepoId, selectedProjectId, addToast } from '$lib/stores/ui';
  import { ROLE_ICONS, getDisplayName } from '$lib/types';
  import type { Role, Repository } from '$lib/types';
  import { tStore } from '$lib/i18n';
  import {
    createLocalRepository, getSetting, setSetting,
    reorderProject, reorderRepo, rebalanceRepoGroup, autoSortAll,
  } from '$lib/api/tauri-commands';
  import EmptyState from './EmptyState.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';

  // Collapsible state per project id. Default: all collapsed on first launch; persisted in settings.
  let collapsed = $state<Record<number, boolean>>({});
  let unassignedCollapsed = $state(true);

  // v0.19.0: persisted sidebar layout state
  let sidebarCollapsed = $state(false);
  let sidebarWidth = $state(320); // px, used in full mode

  function clampWidth(w: number): number {
    return Math.max(200, Math.min(500, w));
  }

  // debounced persist helper
  let persistTimer: ReturnType<typeof setTimeout> | null = null;
  function persistLayoutDebounced() {
    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => {
      setSetting('sidebar_width', String(sidebarWidth)).catch((e) => console.warn(e));
      setSetting('sidebar_collapsed', String(sidebarCollapsed)).catch((e) => console.warn(e));
    }, 300);
  }

  onMount(async () => {
    try {
      const stored = await getSetting('sidebar_collapsed_projects');
      if (stored) collapsed = JSON.parse(stored);
      const storedUnassigned = await getSetting('sidebar_unassigned_collapsed');
      if (storedUnassigned !== null) unassignedCollapsed = storedUnassigned === 'true';
      // v0.19.0
      const storedWidth = await getSetting('sidebar_width');
      if (storedWidth !== null) {
        const w = parseInt(storedWidth, 10);
        if (!Number.isNaN(w)) sidebarWidth = clampWidth(w);
      }
      const storedCollapsed = await getSetting('sidebar_collapsed');
      if (storedCollapsed !== null) sidebarCollapsed = storedCollapsed === 'true';
    } catch (err) {
      console.warn('sidebar collapse load failed', err);
    }
  });

  async function persistCollapsed() {
    try {
      await setSetting('sidebar_collapsed_projects', JSON.stringify(collapsed));
      await setSetting('sidebar_unassigned_collapsed', String(unassignedCollapsed));
    } catch (err) {
      console.warn('sidebar collapse persist failed', err);
    }
  }

  $effect(() => {
    // Any project not yet in collapsed record → default collapsed=true
    let changed = false;
    for (const p of $projects) {
      if (collapsed[p.id] === undefined) {
        collapsed[p.id] = true;
        changed = true;
      }
    }
    if (changed) persistCollapsed();
  });

  // F-025: Rust ORDER BY sort_order ASC, (name|github_name) ASC — no frontend sort needed.
  const sortedProjects = $derived($projects);
  // v0.19.0: drag-resize state
  let isResizing = $state(false);
  let resizeStartX = 0;
  let resizeStartWidth = 0;
  let resizePreviewWidth = $state(320);
  let rafId: number | null = null;

  const effectiveWidth = $derived(
    isResizing
      ? (resizePreviewWidth < 160 ? 52 : Math.max(200, Math.min(500, resizePreviewWidth)))
      : (sidebarCollapsed ? 52 : sidebarWidth)
  );

  function onResizePointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    if (draggedRepoId !== null) return; // don't start resize while a repo drag is in flight
    e.preventDefault();
    isResizing = true;
    resizeStartX = e.clientX;
    resizeStartWidth = sidebarCollapsed ? 52 : sidebarWidth;
    resizePreviewWidth = resizeStartWidth;
    // Add window-level listeners — pointer may travel outside handle
    window.addEventListener('pointermove', onResizePointerMove);
    window.addEventListener('pointerup', onResizePointerUp);
    window.addEventListener('pointercancel', onResizePointerUp);
  }

  function onResizePointerMove(e: PointerEvent) {
    if (!isResizing) return;
    if (rafId !== null) cancelAnimationFrame(rafId);
    rafId = requestAnimationFrame(() => {
      const delta = e.clientX - resizeStartX;
      const raw = resizeStartWidth + delta;
      resizePreviewWidth = Math.max(0, Math.min(500, raw));
      rafId = null;
    });
  }

  function onResizePointerUp() {
    if (!isResizing) return;
    isResizing = false;
    window.removeEventListener('pointermove', onResizePointerMove);
    window.removeEventListener('pointerup', onResizePointerUp);
    window.removeEventListener('pointercancel', onResizePointerUp);
    if (rafId !== null) { cancelAnimationFrame(rafId); rafId = null; }

    // Commit decision based on final preview width
    if (resizePreviewWidth < 160) {
      // Snap to collapsed; sidebarWidth retains last non-collapsed value
      if (showNewProjectForm) cancelNewProject();
      if (showLocalFolderForm) cancelLocalFolder();
      sidebarCollapsed = true;
    } else {
      sidebarCollapsed = false;
      sidebarWidth = clampWidth(resizePreviewWidth);
    }
    persistLayoutDebounced();
  }

  // New project form state
  let showNewProjectForm = $state(false);
  let newProjectName = $state('');
  let newProjectDesc = $state('');
  let newProjectType = $state<'standard' | 'microservice'>('standard');
  let isCreating = $state(false);


  function toggleProject(id: number) {
    collapsed[id] = !collapsed[id];
    persistCollapsed();
  }

  function toggleUnassigned() {
    unassignedCollapsed = !unassignedCollapsed;
    persistCollapsed();
  }

  function expandAll() {
    for (const p of sortedProjects) collapsed[p.id] = false;
    unassignedCollapsed = false;
    persistCollapsed();
  }

  function collapseAll() {
    for (const p of sortedProjects) collapsed[p.id] = true;
    unassignedCollapsed = true;
    persistCollapsed();
  }

  function openProject(projectId: number) {
    selectedProjectId.set(projectId);
    currentScreen.set({ name: 'project' });
  }

  function selectRepo(id: number) {
    selectedRepoId.set(id);
    currentScreen.set({ name: 'repo-detail' });
  }

  async function handleSync() {
    if ($isSyncing) return;
    if (!$pat) return;
    await syncFromGitHub($pat);
  }

  async function handleCreateProject() {
    if (!newProjectName.trim()) return;
    isCreating = true;
    const created = await addProject(newProjectName.trim(), newProjectDesc.trim() || undefined, newProjectType);
    if (created) {
      newProjectName = '';
      newProjectDesc = '';
      newProjectType = 'standard';
      showNewProjectForm = false;
    }
    isCreating = false;
  }

  function cancelNewProject() {
    showNewProjectForm = false;
    newProjectName = '';
    newProjectDesc = '';
    newProjectType = 'standard';
  }

  let showLocalFolderForm = $state(false);
  let localFolderName = $state('');
  let localFolderPath = $state('');
  let isCreatingLocal = $state(false);

  async function pickLocalFolder() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({ directory: true, multiple: false });
      if (selected) {
        localFolderPath = selected as string;
        if (!localFolderName) {
          localFolderName = localFolderPath.split(/[/\\]/).pop() ?? '';
        }
      }
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function handleCreateLocalRepo() {
    if (!localFolderName.trim() || !localFolderPath.trim()) return;
    isCreatingLocal = true;
    try {
      await createLocalRepository(localFolderPath.trim(), localFolderName.trim());
      await loadAllRepos();
      addToast(`${localFolderName}: created`, 'success');
      localFolderName = '';
      localFolderPath = '';
      showLocalFolderForm = false;
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      isCreatingLocal = false;
    }
  }

  function cancelLocalFolder() {
    showLocalFolderForm = false;
    localFolderName = '';
    localFolderPath = '';
  }

  // v0.19.0: collapsed mode helpers
  function projectInitials(project: { name: string; project_type: string }): string {
    const cleaned = project.name.replace(/[^A-Za-zА-Яа-я0-9]/g, '');
    if (project.project_type === 'microservice') {
      // Microservice: ⚙ + 1 char
      const c = cleaned.slice(0, 1).toUpperCase();
      return `⚙${c}`;
    }
    // Standard: first 2 chars
    return cleaned.slice(0, 2).toUpperCase() || '??';
  }

  function projectColor(project: { project_type: string }): string {
    return project.project_type === 'microservice' ? '#3b82f6' : '#475569';
  }

  function toggleSidebarCollapsed() {
    // Auto-close any open forms on transition (per spec)
    if (showNewProjectForm) cancelNewProject();
    if (showLocalFolderForm) cancelLocalFolder();
    sidebarCollapsed = !sidebarCollapsed;
    persistLayoutDebounced();
  }

  function clickProjectInCollapsed(projectId: number) {
    selectedProjectId.set(projectId);
    currentScreen.set({ name: 'project' });
    // One-shot force-expand the clicked project (other projects keep their state per spec)
    if (collapsed[projectId] !== false) {
      collapsed[projectId] = false;
      persistCollapsed();
    }
    sidebarCollapsed = false;
    persistLayoutDebounced();
    // Scroll into view after re-render
    setTimeout(() => {
      document.querySelector(`[data-project-id="${projectId}"]`)
        ?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }, 80);
  }

  function clickUnassignedInCollapsed() {
    unassignedCollapsed = false;
    persistCollapsed();
    sidebarCollapsed = false;
    persistLayoutDebounced();
    setTimeout(() => {
      document.querySelector('.project-group.unassigned')
        ?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }, 80);
  }

  // F-025 v2: single ▲▼ pair in the header acts on the currently-open item.
  // Target = selectedRepoId if set, else selectedProjectId. Nothing selected → buttons disabled.
  // Per-row ▲▼ removed — their hover/focus dance was worse than the extra click to select first.
  const reorderTarget = $derived<
    | { kind: 'repo'; id: number; name: string }
    | { kind: 'project'; id: number; name: string }
    | null
  >(
    $selectedRepoId != null
      ? (() => {
          const r = $allRepos.find((x) => x.id === $selectedRepoId);
          return r ? { kind: 'repo', id: r.id, name: getDisplayName(r) } : null;
        })()
      : $selectedProjectId != null
      ? (() => {
          const p = $projects.find((x) => x.id === $selectedProjectId);
          return p ? { kind: 'project', id: p.id, name: p.name } : null;
        })()
      : null
  );

  // "Just moved" highlight: after clicking ▲/▼, the item that was moved lights up
  // as if hovered, so the user can see where it went (critical for wrap-around on
  // first/last). Cleared when the mouse enters a *different* row — so normal
  // browsing dismisses it, but sitting still / keyboard use preserves it.
  let justMoved = $state<{ kind: 'repo' | 'project'; id: number } | null>(null);

  async function handleHeaderReorder(direction: 'up' | 'down') {
    if (!reorderTarget) return;
    const snapshot = { kind: reorderTarget.kind, id: reorderTarget.id };
    try {
      if (snapshot.kind === 'repo') {
        await reorderRepo(snapshot.id, direction);
        await loadAllRepos();
      } else {
        await reorderProject(snapshot.id, direction);
        await loadProjects();
      }
      justMoved = snapshot;
      await tick();
      // Scroll moved item into view in case it wrapped to the opposite end.
      const sel = snapshot.kind === 'repo'
        ? `[data-repo-id="${snapshot.id}"]`
        : `[data-project-id="${snapshot.id}"]`;
      (document.querySelector(sel) as HTMLElement | null)
        ?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function clearJustMovedIfElsewhere(kind: 'repo' | 'project', id: number) {
    if (justMoved && (justMoved.kind !== kind || justMoved.id !== id)) {
      justMoved = null;
    }
  }

  function reorderTooltip(direction: 'up' | 'down'): string {
    if (!reorderTarget) return $tStore('sidebar.reorderNoSelection' as any);
    const key = direction === 'up' ? 'sidebar.reorderUpNamed' : 'sidebar.reorderDownNamed';
    return $tStore(key as any).replace('{0}', reorderTarget.name);
  }

  // F-025: Auto-sort — confirm dialog + reset sort_order to initial formula.
  let showAutoSortConfirm = $state(false);
  async function handleAutoSortConfirm() {
    showAutoSortConfirm = false;
    try {
      await autoSortAll();
      await Promise.all([loadProjects(), loadAllRepos()]);
      addToast($tStore('toast.sortRestored' as any), 'success');
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function getRoleIcon(role: string | null): string {
    if (!role) return '';
    return ROLE_ICONS[role as Role] ?? '';
  }

  function getReposForProject(projectId: number) {
    return $allRepos.filter((r) => r.project_id === projectId);
  }

  const unassignedRepos = $derived($allRepos.filter((r) => r.project_id === null));

  // Drag-and-drop state
  let draggedRepoId = $state<number | null>(null);
  let dragOverProjectId = $state<number | null>(null);

  // Pointer-based drag (HTML5 drag unreliable in Tauri WebView2)
  let isDragging = $state(false);
  let dragStartX = 0;
  let dragStartY = 0;
  let dragLabel = $state('');
  let ghostX = $state(0);
  let ghostY = $state(0);
  const DRAG_THRESHOLD = 8;

  function handlePointerDown(e: PointerEvent, repoId: number) {
    if (e.button !== 0) return;
    if (isResizing) return; // don't start repo drag while resize is in flight
    e.preventDefault(); // prevent text selection
    draggedRepoId = repoId;
    isDragging = false;
    const repo = $allRepos.find(r => r.id === repoId);
    dragLabel = repo ? getDisplayName(repo) : '';
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    // Do NOT setPointerCapture — it breaks elementFromPoint
  }

  function handlePointerMove(e: PointerEvent) {
    if (draggedRepoId === null) return;

    // Check threshold before activating drag
    if (!isDragging) {
      const dx = Math.abs(e.clientX - dragStartX);
      const dy = Math.abs(e.clientY - dragStartY);
      if (dx < DRAG_THRESHOLD && dy < DRAG_THRESHOLD) return;
      isDragging = true;
    }

    ghostX = e.clientX + 12;
    ghostY = e.clientY - 8;

    // Find which project group we're hovering over
    const el = document.elementFromPoint(e.clientX, e.clientY);
    if (!el) { dragOverProjectId = null; return; }
    const group = el.closest('[data-project-id]') as HTMLElement | null;
    if (group) {
      const pid = group.dataset.projectId;
      dragOverProjectId = pid === 'null' ? -1 : Number(pid);
    } else {
      dragOverProjectId = null;
    }
  }

  async function handlePointerUp(e: PointerEvent) {
    if (draggedRepoId === null) return;
    const wasDragging = isDragging;
    const repoId = draggedRepoId;
    draggedRepoId = null;
    isDragging = false;
    dragOverProjectId = null;

    if (!wasDragging) return; // was a click, not a drag

    // Determine drop target
    const el = document.elementFromPoint(e.clientX, e.clientY);
    if (!el) return;
    const group = el.closest('[data-project-id]') as HTMLElement | null;
    if (!group) return;

    const pid = group.dataset.projectId;
    const targetProjectId = pid === 'null' ? null : Number(pid);

    const repo = $allRepos.find(r => r.id === repoId);
    if (!repo) return;

    if (repo.project_id !== targetProjectId) {
      // Cross-project move — Rust assign_repository places it at group end (F-025).
      await assignRepo(repoId, targetProjectId, repo.role);
      await loadAllRepos();
      return;
    }

    // F-025: same-project drop → reorder within group. Compute target index by cursor Y.
    const siblings = $allRepos.filter((r) => r.project_id === targetProjectId);
    if (siblings.length <= 1) return;
    const orderedIds: number[] = [];
    const items = Array.from(group.querySelectorAll<HTMLElement>('[data-repo-id]'));
    let inserted = false;
    for (const item of items) {
      const itemRepoId = Number(item.dataset.repoId);
      if (itemRepoId === repoId) continue; // remove from list, reinsert below
      if (!inserted) {
        const rect = item.getBoundingClientRect();
        if (e.clientY < rect.top + rect.height / 2) {
          orderedIds.push(repoId);
          inserted = true;
        }
      }
      orderedIds.push(itemRepoId);
    }
    if (!inserted) orderedIds.push(repoId);
    try {
      await rebalanceRepoGroup(orderedIds);
      await loadAllRepos();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }
</script>

<aside
  class="sidebar"
  class:collapsed={sidebarCollapsed}
  class:resizing={isResizing}
  class:no-select={isDragging}
  style="width: {effectiveWidth}px; min-width: {effectiveWidth}px;"
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  oncontextmenu={(e) => e.preventDefault()}
>
  {#if sidebarCollapsed}
    <div class="collapsed-strip">
      <button
        class="ghost collapse-toggle"
        onclick={toggleSidebarCollapsed}
        title={$tStore('sidebar.toggleCollapse' as any)}
        aria-label={$tStore('sidebar.toggleCollapse' as any)}
      >▶</button>
      {#each sortedProjects as project (project.id)}
        <button
          class="project-icon"
          class:selected={$selectedProjectId === project.id}
          style="background: {projectColor(project)}"
          onclick={() => clickProjectInCollapsed(project.id)}
          title={project.name}
        >
          {projectInitials(project)}
        </button>
      {/each}
      {#if unassignedRepos.length > 0}
        <button
          class="unassigned-badge"
          onclick={clickUnassignedInCollapsed}
          title={$tStore('sidebar.unassignedBadge' as any).replace('{count}', String(unassignedRepos.length))}
        >❓{unassignedRepos.length}</button>
      {/if}
    </div>
  {:else}
  <div class="sidebar-header">
    <div class="title-group">
      <button
        class="ghost compact collapse-toggle-full"
        onclick={toggleSidebarCollapsed}
        title={$tStore('sidebar.toggleCollapse' as any)}
        aria-label={$tStore('sidebar.toggleCollapse' as any)}
      >◀</button>
      <span class="sidebar-title">{$tStore('sidebar.projects')}</span>
      <button
        class="ghost compact"
        onclick={expandAll}
        title={$tStore('sidebar.expandAll' as any)}
        disabled={$projects.length === 0}
      >
        ⊞
      </button>
      <button
        class="ghost compact"
        onclick={collapseAll}
        title={$tStore('sidebar.collapseAll' as any)}
        disabled={$projects.length === 0}
      >
        ⊟
      </button>
      <button
        class="ghost compact"
        onclick={() => (showAutoSortConfirm = true)}
        title={$tStore('sidebar.autoSort' as any)}
        disabled={$projects.length === 0}
      >
        🔤
      </button>
      <button
        class="ghost compact"
        onclick={() => handleHeaderReorder('up')}
        title={reorderTooltip('up')}
        disabled={!reorderTarget}
        aria-label={reorderTooltip('up')}
      >
        ▲
      </button>
      <button
        class="ghost compact"
        onclick={() => handleHeaderReorder('down')}
        title={reorderTooltip('down')}
        disabled={!reorderTarget}
        aria-label={reorderTooltip('down')}
      >
        ▼
      </button>
    </div>
    <div class="header-actions">
      <button
        class="ghost"
        onclick={() => (showNewProjectForm = !showNewProjectForm)}
        title={$tStore('sidebar.createProject')}
        disabled={showNewProjectForm || showLocalFolderForm}
      >
        +
      </button>
      <button
        class="ghost"
        onclick={() => (showLocalFolderForm = !showLocalFolderForm)}
        title={$tStore('sidebar.addLocalFolder' as any)}
        disabled={showLocalFolderForm || showNewProjectForm}
      >
        📁
      </button>
      <button
        class="ghost sync-btn"
        onclick={handleSync}
        disabled={$isSyncing || !$pat}
        title={!$pat ? $tStore('sidebar.syncNoPat') : $tStore('sidebar.syncTooltip')}
      >
        {#if $isSyncing}
          <span class="spinner" aria-hidden="true">⟳</span>
        {:else}
          ⟳
        {/if}
      </button>
    </div>
  </div>

  {#if showNewProjectForm}
    <div class="new-project-form">
      <input
        type="text"
        bind:value={newProjectName}
        placeholder={$tStore('sidebar.projectName')}
        onkeydown={(e) => {
          if (e.key === 'Enter') handleCreateProject();
          if (e.key === 'Escape') cancelNewProject();
        }}
      />
      <input
        type="text"
        bind:value={newProjectDesc}
        placeholder={$tStore('sidebar.descriptionOptional')}
        onkeydown={(e) => {
          if (e.key === 'Enter') handleCreateProject();
          if (e.key === 'Escape') cancelNewProject();
        }}
      />
      <label class="type-row">
        <span class="type-row-label">{$tStore('sidebar.projectTypeLabel' as any)}:</span>
        <select bind:value={newProjectType}>
          <option value="standard">📁 {$tStore('project.typeStandard' as any)}</option>
          <option value="microservice">⚙ {$tStore('project.typeMicroservice' as any)}</option>
        </select>
      </label>
      <div class="form-actions">
        <button onclick={cancelNewProject} type="button">{$tStore('sidebar.cancel')}</button>
        <button
          class="primary"
          onclick={handleCreateProject}
          disabled={isCreating || !newProjectName.trim()}
          type="button"
        >
          {isCreating ? $tStore('sidebar.creating') : $tStore('sidebar.create')}
        </button>
      </div>
    </div>
  {/if}

  {#if showLocalFolderForm}
    <div class="new-project-form">
      <input
        type="text"
        bind:value={localFolderName}
        placeholder={$tStore('sidebar.localFolderNamePlaceholder' as any)}
        onkeydown={(e) => {
          if (e.key === 'Enter') handleCreateLocalRepo();
          if (e.key === 'Escape') cancelLocalFolder();
        }}
      />
      <div class="path-row">
        <input
          type="text"
          bind:value={localFolderPath}
          placeholder={$tStore('sidebar.localFolderPathPlaceholder' as any)}
          readonly
        />
        <button class="ghost" onclick={pickLocalFolder} type="button">…</button>
      </div>
      <div class="form-actions">
        <button onclick={cancelLocalFolder} type="button">{$tStore('sidebar.cancel')}</button>
        <button
          class="primary"
          onclick={handleCreateLocalRepo}
          disabled={isCreatingLocal || !localFolderName.trim() || !localFolderPath.trim()}
          type="button"
        >
          {isCreatingLocal ? $tStore('sidebar.creating') : $tStore('sidebar.create')}
        </button>
      </div>
    </div>
  {/if}

  <div class="project-list">
    {#if $projects.length === 0 && !showNewProjectForm}
      <EmptyState
        icon="📁"
        title={$tStore('sidebar.noProjects')}
        hint={$tStore('sidebar.noProjectsHint')}
      />
    {:else}
      {#each sortedProjects as project (project.id)}
        {@const repos = getReposForProject(project.id)}
        <div
          class="project-group"
          class:drag-over={dragOverProjectId === project.id}
          data-project-id={project.id}
          role="group"
        >
          <div
            class="project-row project-header"
            class:just-moved={justMoved?.kind === 'project' && justMoved.id === project.id}
            onmouseenter={() => clearJustMovedIfElsewhere('project', project.id)}
            role="presentation"
          >
            <button
              class="ghost collapse-btn"
              onclick={() => toggleProject(project.id)}
              title={collapsed[project.id] ? $tStore('sidebar.expand') : $tStore('sidebar.collapse')}
            >
              <span class="chevron" class:rotated={collapsed[project.id]}>▾</span>
            </button>
            <button
              class="ghost project-name-btn"
              onclick={() => openProject(project.id)}
              title={project.description ?? project.name}
            >
              {#if project.project_type === 'microservice'}
                <span class="type-icon" title={$tStore('project.typeMicroservice' as any)}>⚙</span>
              {/if}
              {project.name}
              {#if repos.length > 0}
                <span class="repo-count">{repos.length}</span>
              {/if}
            </button>
          </div>

          {#if !collapsed[project.id]}
            <ul class="repo-list">
              {#if repos.length === 0}
                <li class="repo-empty">{$tStore('sidebar.noReposAssigned')}</li>
              {:else}
                {#each repos as repo (repo.id)}
                  <li
                    role="listitem"
                    class="repo-draggable"
                    class:dragging={draggedRepoId === repo.id}
                    data-repo-id={repo.id}
                    onpointerdown={(e) => handlePointerDown(e, repo.id)}
                  >
                    <div
                      class="repo-item"
                      class:just-moved={justMoved?.kind === 'repo' && justMoved.id === repo.id}
                      onclick={() => { if (!isDragging) selectRepo(repo.id); }}
                      onkeydown={(e) => e.key === 'Enter' && selectRepo(repo.id)}
                      onmouseenter={() => clearJustMovedIfElsewhere('repo', repo.id)}
                      title={repo.description ?? repo.github_name}
                      role="button"
                      tabindex="0"
                    >
                      {#if repo.role}
                        <span class="role-icon" title={$tStore('sidebar.roleTooltip').replace('{0}', repo.role)}>{getRoleIcon(repo.role)}</span>
                      {/if}
                      <span class="repo-name">{getDisplayName(repo)}</span>
                    </div>
                  </li>
                {/each}
              {/if}
            </ul>
          {/if}
        </div>
      {/each}

      <!-- Unassigned group -->
      {#if unassignedRepos.length > 0}
        <div
          class="project-group unassigned"
          class:drag-over={dragOverProjectId === -1}
          data-project-id="null"
          role="group"
        >
          <div class="project-row unassigned-header">
            <button
              class="ghost collapse-btn"
              onclick={toggleUnassigned}
              title={unassignedCollapsed ? $tStore('sidebar.expand') : $tStore('sidebar.collapse')}
            >
              <span class="chevron" class:rotated={unassignedCollapsed}>▾</span>
            </button>
            <span class="project-name muted">
              {$tStore('sidebar.unassigned')}
              <span class="repo-count">{unassignedRepos.length}</span>
            </span>
          </div>

          {#if !unassignedCollapsed}
            <ul class="repo-list">
              {#each unassignedRepos as repo (repo.id)}
                <li
                  role="listitem"
                  class="repo-draggable"
                  class:dragging={draggedRepoId === repo.id}
                  onpointerdown={(e) => handlePointerDown(e, repo.id)}
                  onpointermove={handlePointerMove}
                  onpointerup={handlePointerUp}
                >
                  <div
                    class="repo-item"
                    onclick={() => { if (!isDragging) selectRepo(repo.id); }}
                    onkeydown={(e) => e.key === 'Enter' && selectRepo(repo.id)}
                    title={repo.description ?? repo.github_name}
                    role="button"
                    tabindex="0"
                  >
                    <span class="repo-name">{getDisplayName(repo)}</span>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
  {#if isDragging}
    <div class="drag-ghost" style="left: {ghostX}px; top: {ghostY}px;">
      📦 {dragLabel}
    </div>
  {/if}
  {/if}
  <div
    class="resize-handle"
    onpointerdown={onResizePointerDown}
    role="separator"
    aria-orientation="vertical"
  ></div>
</aside>

{#if showAutoSortConfirm}
  <ConfirmDialog
    title={$tStore('sidebar.autoSortConfirmTitle' as any)}
    message={$tStore('sidebar.autoSortConfirmMessage' as any)}
    onConfirm={handleAutoSortConfirm}
    onCancel={() => (showAutoSortConfirm = false)}
  />
{/if}

<style>
  .sidebar {
    background-color: var(--sidebar-bg);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    height: 100%;
    position: relative; /* needed for drag handle in Stage E */
    transition: width 0.15s ease, min-width 0.15s ease;
  }

  .sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 10px 10px 14px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }

  .sidebar-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-muted);
  }

  .title-group {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .ghost.compact {
    padding: 0 4px;
    font-size: 12px;
    line-height: 1;
  }

  .header-actions {
    display: flex;
    gap: 2px;
    align-items: center;
  }

  .header-actions button {
    font-size: 13px;
    padding: 3px 7px;
  }

  .sync-btn {
    font-size: 12px;
  }

  .spinner {
    display: inline-block;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .new-project-form {
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
    background-color: var(--surface);
    flex-shrink: 0;
  }

  .new-project-form input {
    font-size: 12px;
    padding: 5px 8px;
  }

  .type-row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-muted);
  }

  .type-row select {
    flex: 1;
    font-size: 12px;
    padding: 4px 6px;
  }

  .type-row-label {
    white-space: nowrap;
  }

  .type-icon {
    font-size: 11px;
    margin-right: 3px;
    opacity: 0.8;
  }

  .path-row {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .path-row input { flex: 1; }
  .path-row button { flex-shrink: 0; padding: 3px 6px; }

  .form-actions {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
  }

  .form-actions button {
    font-size: 12px;
    padding: 4px 10px;
  }

  .project-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }

  .project-group {
    margin-bottom: 2px;
  }

  .project-group.unassigned {
    margin-top: 8px;
    border-top: 1px solid var(--border);
    padding-top: 4px;
  }

  .project-row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 6px 2px 4px;
    border-radius: 3px;
  }

  /* F-025 v2: post-reorder highlight — mirrors the hover appearance of each row type
     so the user can see where the moved item landed (esp. after wrap-around). */
  .project-row.just-moved {
    background-color: var(--surface-hover);
  }
  .project-row.just-moved .project-name-btn {
    color: var(--accent);
  }


  .collapse-btn {
    padding: 2px 4px;
    font-size: 12px;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  .chevron {
    display: inline-block;
    transition: transform 0.15s;
    line-height: 1;
  }

  .chevron.rotated {
    transform: rotate(-90deg);
  }

  .project-name-btn {
    flex: 1;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 5px;
    cursor: pointer;
    text-align: left;
    padding: 2px 4px;
    border-radius: 3px;
  }

  .project-name-btn:hover {
    color: var(--accent);
  }

  .project-name {
    flex: 1;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: flex;
    align-items: center;
    gap: 5px;
    cursor: default;
  }

  .project-name.muted {
    color: var(--text-muted);
    font-weight: 500;
  }

  .repo-count {
    font-size: 10px;
    font-weight: 500;
    background-color: var(--surface);
    color: var(--text-muted);
    padding: 0 5px;
    border-radius: 8px;
    border: 1px solid var(--border);
    min-width: 18px;
    text-align: center;
    flex-shrink: 0;
  }

  .repo-list {
    list-style: none;
    padding: 2px 0 4px 18px;
  }

  .repo-item {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 3px 8px;
    font-size: 12px;
    color: var(--text-muted);
    border-radius: 3px;
    text-align: left;
    font-weight: 400;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    cursor: pointer;
    background: none;
    border: none;
  }

  .repo-item:hover,
  .repo-item.just-moved {
    color: var(--text);
    background-color: var(--surface-hover);
  }

  .role-icon {
    font-size: 11px;
    flex-shrink: 0;
    cursor: help;
  }

  .repo-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
    min-width: 0;
  }

  .repo-empty {
    font-size: 11px;
    color: var(--text-muted);
    padding: 3px 8px;
    opacity: 0.6;
    font-style: italic;
  }

  .repo-draggable {
    cursor: grab;
    list-style: none;
    touch-action: none;
  }

  .repo-draggable.dragging {
    cursor: grabbing;
    opacity: 0.5;
  }

  .project-header,
  .unassigned-header {
    border-radius: 4px;
    transition: outline 0.1s, background-color 0.1s;
  }

  .drag-over {
    outline: 2px dashed var(--accent);
    outline-offset: -2px;
    background-color: color-mix(in srgb, var(--accent) 10%, transparent);
  }

  .no-select {
    user-select: none;
    -webkit-user-select: none;
  }

  .drag-ghost {
    position: fixed;
    pointer-events: none;
    z-index: 9999;
    font-size: 11px;
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 2px 8px;
    white-space: nowrap;
    opacity: 0.9;
    box-shadow: 0 2px 8px rgba(0,0,0,0.3);
  }

  /* v0.19.0: Collapsed mode strip */
  .collapsed-strip {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 8px 4px;
    gap: 6px;
    height: 100%;
    overflow-y: auto;
  }
  .collapse-toggle {
    font-size: 14px;
    padding: 4px 6px;
    margin-bottom: 4px;
    color: var(--text-muted);
  }
  .collapse-toggle:hover { color: var(--text); }
  .project-icon {
    width: 36px;
    height: 28px;
    border: 2px solid transparent;
    border-radius: 4px;
    color: white;
    font-weight: 700;
    font-size: 11px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: var(--font-mono, monospace);
    transition: filter 0.1s, border-color 0.1s;
    flex-shrink: 0;
  }
  .project-icon:hover { filter: brightness(1.15); }
  .project-icon.selected { border-color: var(--accent); }
  .unassigned-badge {
    margin-top: auto;
    padding: 4px 6px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-muted);
    border-radius: 4px;
    font-size: 11px;
    cursor: pointer;
  }
  .unassigned-badge:hover { color: var(--text); background: var(--surface-hover); }

  .resize-handle {
    position: absolute;
    top: 0;
    right: 0;
    width: 4px;
    height: 100%;
    cursor: col-resize;
    z-index: 10;
    user-select: none;
  }
  .resize-handle:hover,
  .sidebar.resizing .resize-handle {
    background: var(--accent);
    opacity: 0.5;
  }
</style>
