<script lang="ts">
  import { confirmBug, editBug, removeBug, rejectBugWithComment } from '$lib/stores/bugs';
  import { type BugView } from '$lib/types';
  import { tStore } from '$lib/i18n';
  import ConfirmDialog from './ConfirmDialog.svelte';

  function autoFocus(node: HTMLElement) {
    node.focus();
  }

  interface Props {
    bug: BugView;
    repoRole: string;
  }

  let { bug, repoRole }: Props = $props();

  let editingDesc = $state(false);
  let editingComment = $state(false);
  let descDraft = $state('');
  let commentDraft = $state('');
  let showConfirmDelete = $state(false);
  let showRejectPrompt = $state(false);
  let rejectComment = $state('');

  const severityColors: Record<string, string> = {
    critical: '#ef4444',
    major: '#f97316',
    medium: '#eab308',
    minor: '#6b7280',
  };

  const statusColors: Record<string, string> = {
    'created': '#6b7280',       // grey — just filed, no work yet
    'in-progress': '#3b82f6',   // blue — LLM/dev actively working
    'testing': '#f97316',       // orange — awaiting user verification
    'rejected': '#ef4444',      // red — user rejected, needs retry
    'confirmed': '#22c55e',     // green — terminal success
  };

  const severities = ['critical', 'major', 'medium', 'minor'] as const;

  const categoriesByRole: Record<string, string[]> = {
    client: ['ui_ux', 'ux_flow', 'logic', 'integration', 'other'],
    admin_client: ['ui_ux', 'ux_flow', 'logic', 'integration', 'other'],
    test_client: ['ui_ux', 'ux_flow', 'logic', 'integration', 'other'],
    landing: ['ui_ux', 'ux_flow', 'logic', 'integration', 'other'],
    server: ['logic', 'database', 'performance', 'security', 'integration', 'other'],
    tool: ['ui_ux', 'ux_flow', 'logic', 'database', 'performance', 'other'],
  };
  const defaultCategories = ['ui_ux', 'ux_flow', 'logic', 'backend', 'network', 'database', 'security', 'performance', 'integration', 'other'];

  const categories = $derived(categoriesByRole[repoRole] ?? defaultCategories);

  const dotColor = $derived(severityColors[bug.severity] ?? '#6b7280');
  const statusColor = $derived(statusColors[bug.status] ?? '#6b7280');
  const isTesting = $derived(bug.status === 'testing');
  const isConfirmed = $derived(bug.status === 'confirmed');
  // Hard-delete only allowed for accidental creations (status 'created').
  // Beyond that, confirmed-status is the archive path.
  const canDelete = $derived(bug.status === 'created');

  // --- Description edit ---
  function startEditDesc() {
    if (isConfirmed) return;
    descDraft = bug.description ?? '';
    editingDesc = true;
  }

  async function saveDesc() {
    editingDesc = false;
    if (descDraft !== bug.description) {
      await editBug(bug.id, descDraft || undefined);
    }
  }

  function handleDescKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { editingDesc = false; }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); saveDesc(); }
  }

  // --- Comment edit ---
  function startEditComment() {
    if (isConfirmed) return;
    commentDraft = bug.comment ?? '';
    editingComment = true;
  }

  async function saveComment() {
    editingComment = false;
    const newComment = commentDraft.trim();
    const oldComment = (bug.comment ?? '').trim();
    if (newComment !== oldComment) {
      // Empty string = clear (backend maps to NULL).
      await editBug(bug.id, undefined, undefined, undefined, newComment);
    }
  }

  function handleCommentKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { editingComment = false; }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); saveComment(); }
  }

  // --- Field changes ---
  async function handleSeverityChange(e: Event) {
    await editBug(bug.id, undefined, (e.target as HTMLSelectElement).value);
  }

  async function handleCategoryChange(e: Event) {
    await editBug(bug.id, undefined, undefined, (e.target as HTMLSelectElement).value);
  }

  // --- Status transitions ---
  async function handleConfirm() {
    await confirmBug(bug.id);
  }

  function handleReject() {
    showRejectPrompt = true;
    rejectComment = '';
  }

  async function handleRejectConfirm() {
    showRejectPrompt = false;
    await rejectBugWithComment(bug.id, rejectComment.trim() || undefined);
  }

  // --- Delete ---
  function handleDelete() {
    if (!bug.description) {
      // No description = silent delete of blank placeholder row.
      removeBug(bug.id);
    } else {
      showConfirmDelete = true;
    }
  }

  async function handleDeleteConfirm() {
    showConfirmDelete = false;
    await removeBug(bug.id);
  }
</script>

<div class="bug-item" class:resolved={isConfirmed}>
  <!-- Row 1: [✓ if testing]  #N  🔴severity  category  date  [confirmed_at if confirmed]  🗑 (if created) -->
  <div class="row-1">
    <div class="action-slot">
      {#if isTesting}
        <button class="action-btn confirm-btn" onclick={handleConfirm} title={$tStore('bugItem.confirmTooltip' as any)} type="button">✓</button>
      {:else if isConfirmed}
        <span class="confirmed-mark" title={$tStore('bugItem.confirmedBadge' as any)}>✓</span>
      {/if}
    </div>

    <span class="num">#{bug.id}</span>

    <span class="dot" style="background-color: {dotColor}"></span>
    <select class="sel" value={bug.severity} onchange={handleSeverityChange} disabled={isConfirmed}>
      {#each severities as s (s)}
        <option value={s}>{$tStore(`severity.${s}` as any)}</option>
      {/each}
    </select>

    <select class="sel" value={bug.category} onchange={handleCategoryChange} disabled={isConfirmed}>
      {#each categories as c (c)}
        <option value={c}>{$tStore(`category.${c}` as any)}</option>
      {/each}
    </select>

    <span class="date">{bug.date}</span>

    <span
      class="status-badge"
      style="background-color: {statusColor}20; color: {statusColor}; border-color: {statusColor}80;"
      title={$tStore(`status.${bug.status}` as any)}
    >{$tStore(`status.${bug.status}` as any)}</span>

    <span class="spacer"></span>

    <span class="attempts" title={$tStore('bugItem.attemptsTooltip' as any)}>
      {$tStore('bugItem.attemptsLabel' as any)}: {bug.fix_attempts}
    </span>

    {#if isConfirmed && bug.confirmed_at}
      <span class="confirmed-at">{bug.confirmed_at}</span>
    {/if}

    {#if canDelete}
      <button class="del-btn" onclick={handleDelete} title={$tStore('bugItem.delete')} type="button">🗑</button>
    {/if}
  </div>

  <!-- Row 2: [✗ if testing] + Description -->
  <div class="row-2">
    <div class="action-slot">
      {#if isTesting}
        <button class="action-btn reject-btn" onclick={handleReject} title={$tStore('bugItem.rejectTooltip' as any)} type="button">✗</button>
      {/if}
    </div>
    <div class="desc-area">
    {#if editingDesc}
      <textarea
        class="edit-area"
        bind:value={descDraft}
        onblur={saveDesc}
        onkeydown={handleDescKeydown}
        placeholder={$tStore('bugItem.clickToAddNotes')}
        rows="2"
        use:autoFocus
      ></textarea>
      {#if descDraft.includes('|')}
        <span class="pipe-hint">{$tStore('bug.pipeHint' as any)}</span>
      {/if}
    {:else}
      <button class="text-btn" onclick={startEditDesc} type="button" disabled={isConfirmed}>
        {#if bug.description}
          {bug.description}
        {:else}
          <span class="placeholder">{$tStore('bugItem.clickToAddNotes')}</span>
        {/if}
      </button>
    {/if}
    </div>
  </div>

  <!-- Row 3: Comment (only when fix_attempts > 0) -->
  {#if bug.fix_attempts > 0}
    <div class="row-3">
      {#if editingComment}
        <textarea
          class="edit-area comment-area"
          bind:value={commentDraft}
          onblur={saveComment}
          onkeydown={handleCommentKeydown}
          placeholder={$tStore('bugItem.commentPlaceholder' as any)}
          rows="1"
          use:autoFocus
        ></textarea>
      {:else if bug.comment}
        <button class="comment-btn" onclick={startEditComment} type="button" disabled={isConfirmed}>
          💬 {bug.comment}
        </button>
      {:else if !isConfirmed}
        <button class="add-comment-btn" onclick={startEditComment} type="button">+ {$tStore('bugItem.addComment' as any)}</button>
      {/if}
    </div>
  {/if}
</div>

{#if showConfirmDelete}
  <ConfirmDialog
    title={$tStore('bugItem.deleteConfirmTitle')}
    message={$tStore('bugItem.deleteConfirmMessage').replace('{0}', bug.description?.slice(0, 50) ?? '')}
    onConfirm={handleDeleteConfirm}
    onCancel={() => (showConfirmDelete = false)}
  />
{/if}

{#if showRejectPrompt}
  <ConfirmDialog
    title={$tStore('bugItem.rejectConfirmTitle' as any)}
    message={$tStore('bugItem.rejectConfirmMessage' as any)}
    onConfirm={handleRejectConfirm}
    onCancel={() => (showRejectPrompt = false)}
  >
    <textarea
      class="reject-textarea"
      bind:value={rejectComment}
      placeholder={$tStore('bugItem.commentPlaceholder' as any)}
      rows="2"
    ></textarea>
  </ConfirmDialog>
{/if}

<style>
  .bug-item {
    padding: 6px 0;
    border-bottom: 1px solid var(--border);
  }
  .bug-item:last-child { border-bottom: none; }
  .bug-item.resolved {
    opacity: 0.75;
    border-left: 3px solid #22c55e;
    padding-left: 8px;
    background-color: rgba(34, 197, 94, 0.06);
  }

  .row-1 {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .action-slot {
    width: 20px;
    display: flex;
    flex-shrink: 0;
    justify-content: center;
  }

  .action-btn {
    background: none;
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 4px;
    font-size: 12px;
    cursor: pointer;
    line-height: 1.5;
    flex-shrink: 0;
  }

  .confirm-btn { color: #22c55e; border-color: #22c55e; }
  .confirm-btn:hover { background: rgba(34,197,94,0.15); }

  .reject-btn { color: #ef4444; border-color: #ef4444; }
  .reject-btn:hover { background: rgba(239,68,68,0.15); }

  .confirmed-mark {
    color: #22c55e;
    font-size: 12px;
    font-weight: bold;
  }

  .num {
    font-size: 10px;
    color: var(--text-muted);
    font-family: monospace;
    flex-shrink: 0;
  }

  .status-badge {
    font-size: 9px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 10px;
    border: 1px solid;
    text-transform: lowercase;
    letter-spacing: 0.02em;
    line-height: 1.5;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .dot {
    width: 7px; height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }

  .sel {
    font-size: 10px;
    padding: 0 2px;
    height: 20px;
    max-width: 90px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 3px;
  }

  .sel:disabled { opacity: 0.6; cursor: not-allowed; }

  .date {
    font-size: 10px;
    color: var(--text-muted);
    font-family: monospace;
  }

  .confirmed-at {
    font-size: 10px;
    color: #22c55e;
    font-family: monospace;
    margin-left: 4px;
  }

  .spacer { flex: 1; }

  .attempts {
    font-size: 10px;
    color: var(--text-muted);
    font-family: monospace;
    min-width: 14px;
    text-align: center;
  }

  .del-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 12px;
    cursor: pointer;
    padding: 0 2px;
    flex-shrink: 0;
  }
  .del-btn:hover { color: #ef4444; }

  .row-2 { margin-top: 2px; display: flex; align-items: flex-start; gap: 5px; }
  .desc-area { flex: 1; min-width: 0; }
  .row-3 { margin-top: 1px; padding-left: 25px; }

  .text-btn {
    background: none;
    border: none;
    padding: 2px 4px;
    font-size: 12px;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    width: 100%;
    white-space: pre-wrap;
    word-break: break-word;
    line-height: 1.4;
  }
  .text-btn:hover:not(:disabled) { background: var(--surface); border-radius: 3px; }
  .text-btn:disabled { cursor: default; }

  .placeholder { opacity: 0.4; font-style: italic; color: var(--text-muted); }

  .edit-area {
    width: 100%;
    font-size: 12px;
    padding: 4px 6px;
    resize: vertical;
    line-height: 1.4;
  }

  .pipe-hint {
    display: block;
    margin-top: 2px;
    font-size: 10px;
    color: rgb(234, 179, 8);
    opacity: 0.85;
  }

  .comment-btn {
    background: none;
    border: none;
    padding: 2px 4px;
    font-size: 11px;
    color: var(--text-muted);
    cursor: pointer;
    text-align: left;
    width: 100%;
    font-style: italic;
    line-height: 1.4;
    /* B-000014 round 2: long comments without spaces (URLs, code) used to
       overflow the bug-list horizontally → page-level horizontal scroll.
       Match .text-btn (description) wrapping behavior. */
    white-space: pre-wrap;
    word-break: break-word;
  }
  .comment-btn:hover:not(:disabled) { background: var(--surface); border-radius: 3px; }
  .comment-btn:disabled { cursor: default; }

  .comment-area { font-size: 11px; }

  .add-comment-btn {
    background: none;
    border: none;
    padding: 0 4px;
    font-size: 10px;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0.5;
  }
  .add-comment-btn:hover { opacity: 1; color: var(--accent); }

  .reject-textarea {
    width: 100%;
    font-size: 12px;
    padding: 4px 6px;
    margin-top: 8px;
    resize: vertical;
  }
</style>
