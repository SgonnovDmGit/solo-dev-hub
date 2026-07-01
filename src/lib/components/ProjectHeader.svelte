<script lang="ts">
  import { editProject, removeProject, loadProjects } from '$lib/stores/projects';
  import { updateProjectType, setProjectAutoSync } from '$lib/api/tauri-commands';
  import { selectedProjectId, currentScreen, addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import type { Project } from '$lib/types';
  import ConfirmDialog from './ConfirmDialog.svelte';

  interface Props {
    project: Project;
    hasParents: boolean;
  }
  let { project, hasParents }: Props = $props();

  // F-012: type-change is blocked ONLY when this project is a microservice connected
  // to parents (standard projects referencing it). Repos and own-connected microservices
  // don't matter — user can freely reshape a project.
  const canChangeType = $derived(!hasParents);

  // Inline edit state for name
  let editingName = $state(false);
  let editNameValue = $state('');

  // Inline edit state for description
  let editingDesc = $state(false);
  let editDescValue = $state('');

  // Delete confirm
  let showDeleteConfirm = $state(false);

  // Type-change confirm
  let showTypeChangeConfirm = $state(false);
  let pendingNewType = $state<'standard' | 'microservice'>('standard');

  function startEditName() {
    editNameValue = project.name;
    editingName = true;
  }

  function cancelEditName() {
    editingName = false;
    editNameValue = '';
  }

  async function saveEditName() {
    if (!editNameValue.trim()) return;
    await editProject(project.id, editNameValue.trim(), project.description ?? undefined);
    editingName = false;
  }

  function startEditDesc() {
    editDescValue = project.description ?? '';
    editingDesc = true;
  }

  function cancelEditDesc() {
    editingDesc = false;
    editDescValue = '';
  }

  async function saveEditDesc() {
    await editProject(project.id, project.name, editDescValue.trim() || undefined);
    editingDesc = false;
  }

  // B-002: same pattern as BugItem — autoFocus + Enter-save, Shift+Enter for newline
  function handleDescKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { cancelEditDesc(); return; }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); saveEditDesc(); }
  }

  function autoFocus(node: HTMLElement) {
    node.focus();
  }

  // B-003: select-based type change — open ConfirmDialog with chosen value
  function handleTypeSelectChange(e: Event) {
    const newType = (e.target as HTMLSelectElement).value as 'standard' | 'microservice';
    if (newType === project.project_type) return;
    pendingNewType = newType;
    showTypeChangeConfirm = true;
    // Revert select immediately; will re-render to new value after confirm
    (e.target as HTMLSelectElement).value = project.project_type;
  }

  async function confirmTypeChange() {
    try {
      await updateProjectType(project.id, pendingNewType);
      addToast($tStore('toast.projectTypeChanged' as any), 'success');
      await loadProjects();
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      showTypeChangeConfirm = false;
    }
  }

  // T-000136: toggle background auto-sync opt-in for this project. Reload the
  // projects store so the flag on `project.auto_sync_enabled` reflects the DB.
  async function handleAutoSyncToggle(e: Event) {
    const enabled = (e.currentTarget as HTMLInputElement).checked;
    try {
      await setProjectAutoSync(project.id, enabled);
      await loadProjects();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function handleDeleteProject() {
    const success = await removeProject(project.id);
    if (success) {
      selectedProjectId.set(null);
      currentScreen.set({ name: 'dashboard' });
    }
    showDeleteConfirm = false;
  }

  function formatDate(iso: string | null | undefined): string {
    if (!iso) return '—';
    return new Date(iso).toLocaleDateString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
    });
  }
</script>

<div class="header">
  <div class="title-row">
    {#if editingName}
      <input
        class="name-input"
        type="text"
        bind:value={editNameValue}
        onkeydown={(e) => {
          if (e.key === 'Enter') saveEditName();
          if (e.key === 'Escape') cancelEditName();
        }}
        onblur={saveEditName}
      />
    {:else}
      <button class="ghost project-title" onclick={startEditName} title={$tStore('project.editName')}>
        {project.name}
      </button>
    {/if}
    <div class="header-actions">
      <button class="icon-btn" onclick={startEditName} title={$tStore('project.editName')} aria-label="edit">✏</button>
      <button
        class="icon-btn danger"
        onclick={() => (showDeleteConfirm = true)}
        disabled={project.project_type === 'microservice' && hasParents}
        title={project.project_type === 'microservice' && hasParents ? $tStore('project.deleteBlockedHasParents' as any) : $tStore('project.deleteProject')}
        aria-label="delete"
      >⌫</button>
    </div>
  </div>

  <div class="meta">
    <span class="type-label">{$tStore('project.typeLabel' as any)}:</span>
    <!-- Обёртка span с title: browsers не показывают title на disabled-элементах надёжно -->
    <span
      class="type-select-wrapper"
      title={canChangeType ? '' : $tStore('project.changeTypeDisabled' as any)}
    >
      <select
        class="type-select"
        value={project.project_type}
        onchange={handleTypeSelectChange}
        disabled={!canChangeType}
      >
        <option value="standard">📁 {$tStore('project.typeStandard' as any)}</option>
        <option value="microservice">⚙ {$tStore('project.typeMicroservice' as any)}</option>
      </select>
      {#if !canChangeType}
        <span class="type-lock-icon" aria-hidden="true">🔒</span>
      {/if}
    </span>
    <span class="meta-sep">•</span>
    {$tStore('project.created')}: {formatDate(project.created_at)}
    <span class="meta-sep">•</span>
    <label class="autosync-toggle" title={$tStore('project.autoSync' as any)}>
      <input
        type="checkbox"
        checked={project.auto_sync_enabled}
        onchange={handleAutoSyncToggle}
      />
      {$tStore('project.autoSync' as any)}
    </label>
  </div>
</div>

<div class="description-section">
  <div class="section-label">{$tStore('project.description')}</div>
  {#if editingDesc}
    <textarea
      class="desc-input"
      bind:value={editDescValue}
      placeholder={$tStore('project.descriptionPlaceholder')}
      onkeydown={handleDescKeydown}
      onblur={saveEditDesc}
      rows="2"
      use:autoFocus
    ></textarea>
  {:else}
    <div
      class="description-text"
      class:placeholder={!project.description}
      onclick={startEditDesc}
      role="button"
      tabindex="0"
      title={$tStore('project.editName')}
      onkeydown={(e) => e.key === 'Enter' && startEditDesc()}
    >
      {project.description || $tStore('project.descriptionPlaceholder')}
    </div>
  {/if}
</div>

{#if showDeleteConfirm && project}
  <ConfirmDialog
    title={$tStore('project.deleteConfirmTitle')}
    message={$tStore('project.deleteConfirmMessage').replace('{0}', project.name)}
    onConfirm={handleDeleteProject}
    onCancel={() => (showDeleteConfirm = false)}
  />
{/if}

{#if showTypeChangeConfirm && project}
  <ConfirmDialog
    title={$tStore('project.changeType' as any)}
    message={$tStore('project.changeTypeConfirm' as any).replace('{0}', pendingNewType === 'microservice' ? $tStore('project.typeMicroservice' as any) : $tStore('project.typeStandard' as any))}
    onConfirm={confirmTypeChange}
    onCancel={() => (showTypeChangeConfirm = false)}
  />
{/if}

<style>
  .header {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex-shrink: 0;
  }

  .title-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .project-title {
    font-size: 22px;
    font-weight: 700;
    color: var(--text);
    cursor: pointer;
    border-radius: 4px;
    padding: 2px 6px;
    margin: 0 -6px;
    transition: background-color 0.1s;
  }

  .project-title:hover {
    background-color: var(--surface);
  }

  .name-input {
    font-size: 22px;
    font-weight: 700;
    background-color: var(--surface);
    border: 1px solid var(--accent);
    border-radius: 4px;
    padding: 2px 6px;
    color: var(--text);
    width: 100%;
    max-width: 500px;
  }

  .meta {
    font-size: 12px;
    color: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }

  .type-label {
    font-size: 11px;
    color: var(--text-muted);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  /* Match the style of role select in repo-table — minimal, use system defaults */
  .type-select {
    font-size: 12px;
    padding: 3px 6px;
    min-width: 140px;
  }

  .type-select:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  /* Wrapper needed because browsers don't always show title on disabled elements */
  .type-select-wrapper {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .type-lock-icon {
    font-size: 11px;
    opacity: 0.7;
    cursor: help;
  }

  .meta-sep {
    color: var(--border);
  }

  .autosync-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: var(--text-muted);
    cursor: pointer;
  }

  .autosync-toggle input {
    cursor: pointer;
    margin: 0;
  }

  .description-section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    flex-shrink: 0;
  }

  .section-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
  }

  .description-text {
    font-size: 13px;
    color: var(--text);
    cursor: pointer;
    border-radius: 4px;
    padding: 6px 8px;
    border: 1px solid transparent;
    transition: border-color 0.1s, background-color 0.1s;
    min-height: 32px;
    line-height: 1.5;
  }

  .description-text:hover {
    background-color: var(--surface);
    border-color: var(--border);
  }

  .description-text.placeholder {
    color: var(--text-muted);
    font-style: italic;
  }

  .desc-input {
    font-size: 13px;
    padding: 6px 8px;
    resize: vertical;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background-color: var(--surface);
    color: var(--text);
    width: 100%;
    font-family: inherit;
    line-height: 1.5;
    min-height: 60px;
  }

  .header-actions {
    margin-left: auto;
    display: flex;
    gap: 4px;
  }
  .icon-btn {
    background: none;
    border: none;
    padding: 4px 8px;
    cursor: pointer;
    font-size: 14px;
    color: var(--text-muted);
    border-radius: 4px;
  }
  .icon-btn:hover { color: var(--accent); background: var(--surface-hover); }
  .icon-btn.danger:hover { color: #f87171; background: rgba(248, 113, 113, 0.1); }
  .icon-btn[disabled] { opacity: 0.4; cursor: not-allowed; }
</style>
