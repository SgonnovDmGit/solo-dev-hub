<script lang="ts">
  import { pat } from '$lib/stores/settings';
  import { addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import {
    listRepoSecrets, getRepoPublicKey, createOrUpdateRepoSecret,
    deleteRepoSecret, splitRepoFullName, type RepoSecret,
  } from '$lib/api/github';
  import { encryptSecret } from '$lib/api/secrets-crypto';
  import { allRepos } from '$lib/stores/repos';
  import {
    listDeployEnvironments, deleteDeploySecret, recordSecretEvent,
  } from '$lib/api/tauri-commands';
  import ConfirmDialog from './ConfirmDialog.svelte';

  interface Props {
    repoFullName?: string;
    busy?: boolean;
  }

  let { repoFullName, busy = false }: Props = $props();

  // State
  let existingSecrets = $state<RepoSecret[]>([]);
  let loading = $state(false); // fetch-spinner flag — gates the {#if loading} branch
  let deleting = $state(false); // this panel's bulk-delete in flight

  // Existing secrets: checkboxes + inline new values
  let selectedSecrets = $state<Set<string>>(new Set());
  let secretValues = $state<Record<string, string>>({});
  let savingSecret = $state<string | null>(null);

  // B-000009: cache the repo's public key once and reuse for per-row autosaves.
  // Without caching, each onblur would re-fetch the key (1 extra API roundtrip).
  let repoPublicKey = $state<{ key: string; key_id: string } | null>(null);

  // Confirm dialogs
  let showDeleteConfirm = $state(false);

  const hasSelectedSecrets = $derived(selectedSecrets.size > 0);

  // Load existing secrets when the repo/pat is available.
  $effect(() => {
    if (repoFullName && $pat) {
      loadSecrets();
    }
  });

  async function loadSecrets(): Promise<RepoSecret[]> {
    if (!repoFullName || !$pat) return [];
    loading = true;
    try {
      const { owner, repo } = splitRepoFullName(repoFullName);
      existingSecrets = await listRepoSecrets($pat, owner, repo);
      selectedSecrets = new Set();
      secretValues = {};
      // Invalidate cached key on reload (repo could have changed).
      repoPublicKey = null;
      return existingSecrets;
    } catch (err: any) {
      if (err?.status === 403 || err?.status === 401) {
        addToast($tStore('secrets.permissionError' as any), 'error');
      }
      existingSecrets = [];
      return [];
    } finally {
      loading = false;
    }
  }

  // Public API: parent triggers a refresh and receives the fresh list.
  export async function reload(): Promise<RepoSecret[]> {
    return loadSecrets();
  }

  /// B-000009: per-row autosave for existing secrets. Mirrors DeployTable's
  /// `saveValue` — fires on textarea blur, pushes the single secret to GitHub,
  /// optimistically updates `existingSecrets[i].updated_at` without re-fetching
  /// the full list (which would re-render the keyed `{#each}` and steal focus
  /// from whichever textarea the user just tabbed into).
  async function saveExistingSecret(name: string) {
    if (!repoFullName || !$pat) return;
    const value = (secretValues[name] ?? '').trim();
    if (!value) return;
    if (savingSecret === name) return;
    savingSecret = name;
    try {
      const { owner, repo } = splitRepoFullName(repoFullName);
      if (!repoPublicKey) {
        repoPublicKey = await getRepoPublicKey($pat, owner, repo);
      }
      const encrypted = await encryptSecret(repoPublicKey.key, value);
      await createOrUpdateRepoSecret($pat, owner, repo, name, encrypted, repoPublicKey.key_id);
      const repoIdForEvent = $allRepos.find((r) => r.github_name === repoFullName)?.id;
      if (repoIdForEvent !== undefined) {
        await recordSecretEvent(repoIdForEvent, 'set', name).catch((e) => console.warn('record_secret_event failed:', e));
      }
      // Optimistic update — bump updated_at locally, no full reload.
      const nowIso = new Date().toISOString();
      existingSecrets = existingSecrets.map((s) =>
        s.name === name ? { ...s, updated_at: nowIso } : s,
      );
      secretValues[name] = '';
      addToast($tStore('secrets.savedOne' as any).replace('{0}', name), 'success');
    } catch (err: any) {
      if (err?.status === 403 || err?.status === 401) {
        addToast($tStore('secrets.permissionError' as any), 'error');
      } else {
        addToast($tStore('secrets.pushFailed' as any).replace('{0}', String(err?.message ?? err)), 'error');
      }
    } finally {
      savingSecret = null;
    }
  }

  function toggleSecret(name: string) {
    const next = new Set(selectedSecrets);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    selectedSecrets = next;
  }

  function selectAllSecrets() {
    selectedSecrets = new Set(existingSecrets.map(s => s.name));
  }

  function deselectAllSecrets() {
    selectedSecrets = new Set();
  }

  async function handleDeleteSelected() {
    if (!repoFullName || !$pat) return;
    showDeleteConfirm = false;
    deleting = true;
    const names = [...selectedSecrets];
    const succeeded: string[] = [];
    let failures = 0;
    const repoIdForCleanup = $allRepos.find(r => r.github_name === repoFullName)?.id;
    let envIdsForCleanup: number[] = [];
    if (repoIdForCleanup !== undefined) {
      try {
        const envs = await listDeployEnvironments(repoIdForCleanup);
        envIdsForCleanup = envs.map(e => e.id);
      } catch {}
    }
    const { owner, repo } = splitRepoFullName(repoFullName);
    for (const name of names) {
      try {
        await deleteRepoSecret($pat, owner, repo, name);
        succeeded.push(name);
        // v0.18.0: also remove from all deploy_secrets rows (DB-level cleanup)
        for (const envId of envIdsForCleanup) {
          try { await deleteDeploySecret(envId, name); } catch {}
        }
        // v0.20.0: record secret event for timeline
        if (repoIdForCleanup !== undefined) {
          await recordSecretEvent(repoIdForCleanup, 'delete', name).catch((e) => console.warn('record_secret_event failed:', e));
        }
      } catch {
        failures++;
      }
    }
    // B-000003: GitHub `list secrets` endpoint has eventual consistency — a
    // refetch immediately after DELETE may still return the deleted entries
    // for several seconds. Keep a denylist of just-deleted names and filter
    // them out of any refetch result so the UI stays consistent without
    // depending on GH propagation.
    const deletedSet = new Set(succeeded);
    existingSecrets = existingSecrets.filter((s) => !deletedSet.has(s.name));
    selectedSecrets = new Set();
    secretValues = {};
    try {
      const fresh = await listRepoSecrets($pat, owner, repo);
      existingSecrets = fresh.filter((s) => !deletedSet.has(s.name));
    } catch {
      // Optimistic local state already applied; reconcile happens on next refresh.
    }
    deleting = false;
    if (failures === 0) {
      addToast($tStore('secrets.deletedCount' as any).replace('{0}', String(names.length)), 'success');
    } else {
      addToast($tStore('secrets.pushPartialFail' as any).replace('{0}', String(failures)).replace('{1}', String(names.length)), 'error');
    }
  }
</script>

<!-- Existing secrets list -->
<div class="existing-secrets">
  <div class="sub-label-row">
    <span class="sub-label">{$tStore('secrets.existingSecrets' as any)}</span>
    {#if !loading && existingSecrets.length > 0}
      <button class="ghost mini" onclick={selectAllSecrets} type="button">{$tStore('secrets.selectAll' as any)}</button>
      <button class="ghost mini" onclick={deselectAllSecrets} type="button">{$tStore('secrets.deselectAll' as any)}</button>
    {/if}
    <span class="spacer"></span>
    <button class="ghost mini refresh-btn"
            onclick={loadSecrets}
            disabled={loading}
            title={$tStore('secrets.refresh' as any) || 'Refresh from GitHub'}
            type="button">{loading ? '⟳' : '↻'}</button>
  </div>
  {#if loading}
    <div class="loading-text">...</div>
  {:else if existingSecrets.length === 0}
    <div class="no-secrets">{$tStore('secrets.noSecrets' as any)}</div>
  {:else}
    <div class="secret-list">
      {#each existingSecrets as secret (secret.name)}
        <div class="secret-row">
          <label class="secret-check">
            <input
              type="checkbox"
              checked={selectedSecrets.has(secret.name)}
              onchange={() => toggleSecret(secret.name)}
            />
            <span class="secret-name">{secret.name}</span>
          </label>
          <textarea
            class="secret-value-input"
            rows="1"
            spellcheck="false"
            autocomplete="off"
            placeholder={$tStore('secrets.newValue' as any)}
            value={secretValues[secret.name] ?? ''}
            oninput={(e) => { secretValues[secret.name] = (e.target as HTMLTextAreaElement).value; }}
            onblur={() => saveExistingSecret(secret.name)}
            disabled={busy || deleting || savingSecret === secret.name}
          ></textarea>
        </div>
      {/each}
    </div>
    <!-- Bulk actions — delete only; update is per-row autosave on blur (B-000009). -->
    {#if hasSelectedSecrets}
      <div class="bulk-actions">
        <button
          class="bulk-btn delete"
          onclick={() => (showDeleteConfirm = true)}
          disabled={busy || deleting}
          type="button"
        >
          {$tStore('secrets.deleteSelected' as any)} ({selectedSecrets.size})
        </button>
      </div>
    {/if}
  {/if}
</div>

<!-- Delete selected confirmation -->
{#if showDeleteConfirm}
  <ConfirmDialog
    title={$tStore('secrets.deleteSelectedConfirm' as any).replace('{0}', String(selectedSecrets.size))}
    message={[...selectedSecrets].join(', ')}
    onConfirm={handleDeleteSelected}
    onCancel={() => (showDeleteConfirm = false)}
  />
{/if}

<style>
  /* T-000129 follow-up: keep existing-secrets at natural height so
     .new-secrets (in the parent) gets the remaining flex space. Without
     flex-shrink:0 the existing-list gets compressed when many secrets are
     present. Self-owned here since Svelte scoped styles don't cross the
     component boundary from SecretsPanel. Harmless in non-flat mode. */
  .existing-secrets {
    flex-shrink: 0;
  }

  .sub-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-muted);
    margin-bottom: 6px;
  }

  .sub-label-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }

  .sub-label-row .sub-label {
    margin-bottom: 0;
  }

  .sub-label-row .spacer {
    flex: 1;
  }

  .sub-label-row .mini {
    font-size: 11px;
    color: var(--accent);
    padding: 0 4px;
  }

  .refresh-btn {
    font-size: 14px;
    padding: 0 6px;
    color: var(--text-muted);
    line-height: 1;
  }

  .refresh-btn:hover:not(:disabled) {
    color: var(--accent);
  }

  .refresh-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .loading-text {
    font-size: 12px;
    color: var(--text-muted);
  }

  .no-secrets {
    font-size: 12px;
    color: var(--text-muted);
    font-style: italic;
  }

  .secret-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .secret-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: 4px;
    border: 1px solid var(--border);
    background-color: var(--bg);
  }

  .secret-row:hover {
    background-color: var(--surface);
  }

  .secret-check {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    min-width: 180px;
    flex-shrink: 0;
  }

  .secret-check input[type="checkbox"] {
    cursor: pointer;
  }

  .secret-name {
    font-family: monospace;
    font-size: 12px;
    color: var(--text);
  }

  .secret-value-input {
    flex: 1;
    font-size: 12px;
    font-family: monospace;
    padding: 3px 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 3px;
    color: var(--text);
    min-width: 0;
    resize: vertical;
    /* Mask like a password — user sees dots, but textarea accepts multi-line paste */
    -webkit-text-security: disc;
    line-height: 1.5;
    min-height: 22px;
    max-height: 180px;
    overflow: auto;
    white-space: pre;
  }

  .secret-value-input:focus {
    -webkit-text-security: disc;
    min-height: 80px;
  }

  .secret-value-input:focus {
    outline: none;
    border-color: var(--accent);
  }

  .secret-value-input:disabled {
    opacity: 0.5;
  }

  .bulk-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 4px;
  }

  .bulk-btn {
    font-size: 12px;
    padding: 4px 12px;
    border-radius: 4px;
    border: 1px solid;
    background: transparent;
    cursor: pointer;
  }

  .bulk-btn.delete {
    border-color: var(--danger, #ef4444);
    color: var(--danger, #ef4444);
  }

  .bulk-btn.delete:hover:not(:disabled) {
    background-color: var(--danger, #ef4444);
    color: white;
  }

  .bulk-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
</style>
