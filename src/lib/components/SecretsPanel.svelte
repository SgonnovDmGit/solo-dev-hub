<script module lang="ts">
  // B-000025: per-repo bulk-paste drafts, shared across SecretsPanel instances
  // so the textarea content survives repo switches and is restored on return
  // (the user wants per-repo memory, not a wipe). Keyed by repoFullName;
  // module-level so it persists for the app session, across repo switches.
  const bulkDrafts: Record<string, string> = {};
</script>

<script lang="ts">
  import { pat } from '$lib/stores/settings';
  import { addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import {
    listRepoSecrets, getRepoPublicKey, createOrUpdateRepoSecret,
    deleteRepoSecret, splitRepoFullName, type RepoSecret,
  } from '$lib/api/github';
  import { encryptSecret } from '$lib/api/secrets-crypto';
  import { parseEnvText } from '$lib/api/secrets-parser';
  import { getDisplayName, type Repository } from '$lib/types';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import { allRepos } from '$lib/stores/repos';
  import {
    registerRepoSecretInDeploys, listDeployEnvironments, deleteDeploySecret,
    recordSecretEvent,
  } from '$lib/api/tauri-commands';

  interface Props {
    mode: 'repo' | 'project';
    repoFullName?: string;
    projectRepos?: Repository[];
    /**
     * Show the header-toggle that collapses the body. Default `true` (legacy behaviour in
     * ProjectDetail where SecretsPanel sits alongside other collapsible sections).
     * Pass `false` when the panel is already the sole content of a dedicated tab
     * (e.g. RepoDetail's "Секреты" tab) — tab-inside-tab collapse is redundant (B-003).
     */
    collapsible?: boolean;
  }

  let {
    mode,
    repoFullName,
    projectRepos = [],
    collapsible = true,
  }: Props = $props();

  // State
  let secretsText = $state('');
  let existingSecrets = $state<RepoSecret[]>([]);
  let loading = $state(false);
  let pushing = $state(false);
  let collapsed = $state(true);
  // Effective open-state for rendering: always open if panel isn't collapsible (dedicated tab).
  const bodyOpen = $derived(!collapsible || !collapsed);
  let parseErrors = $state<string[]>([]);

  // Existing secrets: checkboxes + inline new values
  let selectedSecrets = $state<Set<string>>(new Set());
  let secretValues = $state<Record<string, string>>({});
  let savingSecret = $state<string | null>(null);

  // B-000009: cache the repo's public key once and reuse for per-row autosaves.
  // Without caching, each onblur would re-fetch the key (1 extra API roundtrip).
  let repoPublicKey = $state<{ key: string; key_id: string } | null>(null);

  // Confirm dialogs
  let showDeleteConfirm = $state(false);

  // Project mode state
  let showPushConfirm = $state(false);
  let selectedRepoIds = $state<Set<number>>(new Set());
  let pushProgress = $state('');

  const hasSelectedSecrets = $derived(selectedSecrets.size > 0);

  // Load existing secrets when the body is open (repo mode only).
  // `bodyOpen` covers both cases: collapsible-expanded and non-collapsible (always open).
  $effect(() => {
    if (bodyOpen && mode === 'repo' && repoFullName && $pat) {
      loadSecrets();
    }
  });

  // B-000025: per-repo bulk-paste draft memory. On repo switch, stash the
  // outgoing repo's draft and restore the incoming one, so going back shows what
  // was typed before (and the post-push clear to '' persists). `lastDraftRepo`
  // is a plain non-reactive marker — this effect re-runs on every keystroke
  // (it reads secretsText) but those are cheap no-ops since current === last.
  let lastDraftRepo = '';
  $effect(() => {
    const current = repoFullName ?? '';
    if (current !== lastDraftRepo) {
      if (lastDraftRepo) bulkDrafts[lastDraftRepo] = secretsText;
      lastDraftRepo = current;
      secretsText = current ? (bulkDrafts[current] ?? '') : '';
    }
  });

  async function loadSecrets() {
    if (!repoFullName || !$pat) return;
    loading = true;
    try {
      const { owner, repo } = splitRepoFullName(repoFullName);
      existingSecrets = await listRepoSecrets($pat, owner, repo);
      selectedSecrets = new Set();
      secretValues = {};
      // Invalidate cached key on reload (repo could have changed).
      repoPublicKey = null;
    } catch (err: any) {
      if (err?.status === 403 || err?.status === 401) {
        addToast($tStore('secrets.permissionError' as any), 'error');
      }
      existingSecrets = [];
    } finally {
      loading = false;
    }
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
    pushing = true;
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
    pushing = false;
    if (failures === 0) {
      addToast($tStore('secrets.deletedCount' as any).replace('{0}', String(names.length)), 'success');
    } else {
      addToast($tStore('secrets.pushPartialFail' as any).replace('{0}', String(failures)).replace('{1}', String(names.length)), 'error');
    }
  }

  function handleTextInput() {
    if (secretsText.trim() === '') {
      parseErrors = [];
      return;
    }
    const result = parseEnvText(secretsText);
    parseErrors = result.errors;
  }

  async function pushSecretsToRepo(token: string, owner: string, repo: string, secrets: { name: string; value: string }[]): Promise<void> {
    const { key, key_id } = await getRepoPublicKey(token, owner, repo);
    for (const secret of secrets) {
      const encrypted = await encryptSecret(key, secret.value);
      await createOrUpdateRepoSecret(token, owner, repo, secret.name, encrypted, key_id);
    }
  }

  async function handlePushRepo() {
    if (!repoFullName || !$pat) return;
    const result = parseEnvText(secretsText);
    if (result.errors.length > 0) {
      parseErrors = result.errors;
      return;
    }
    if (result.secrets.length === 0) {
      addToast($tStore('secrets.emptyInput' as any), 'error');
      return;
    }

    pushing = true;
    try {
      const { owner, repo } = splitRepoFullName(repoFullName);
      await pushSecretsToRepo($pat, owner, repo, result.secrets);
      // v0.18.0: register each pushed secret in all deploys of this repo (idempotent INSERT OR IGNORE)
      const repoIdForSync = $allRepos.find(r => r.github_name === repoFullName)?.id;
      if (repoIdForSync !== undefined) {
        for (const s of result.secrets) {
          try { await registerRepoSecretInDeploys(repoIdForSync, s.name); } catch {}
          // v0.20.0: record secret event for timeline
          await recordSecretEvent(repoIdForSync, 'set', s.name).catch((e) => console.warn('record_secret_event failed:', e));
        }
      }
      // Post-push verification: re-fetch the list and ensure every pushed name
      // actually landed. Catches silent "API returned 201 but secret not saved"
      // cases (and gives the user a real error instead of a misleading success).
      const fresh = await listRepoSecrets($pat, owner, repo);
      const freshNames = new Set(fresh.map((s) => s.name));
      const missing = result.secrets.filter((s) => !freshNames.has(s.name)).map((s) => s.name);
      existingSecrets = fresh;
      selectedSecrets = new Set();
      secretValues = {};
      if (missing.length > 0) {
        addToast(
          $tStore('secrets.pushVerifyFailed' as any).replace('{0}', missing.join(', ')),
          'error',
        );
      } else {
        addToast($tStore('secrets.pushSuccess' as any).replace('{0}', repoFullName), 'success');
        secretsText = '';
        parseErrors = [];
      }
    } catch (err: any) {
      if (err?.status === 403 || err?.status === 401) {
        addToast($tStore('secrets.permissionError' as any), 'error');
      } else {
        addToast($tStore('secrets.pushFailed' as any).replace('{0}', String(err?.message ?? err)), 'error');
      }
    } finally {
      pushing = false;
    }
  }

  function handlePushProject() {
    const result = parseEnvText(secretsText);
    if (result.errors.length > 0) {
      parseErrors = result.errors;
      return;
    }
    if (result.secrets.length === 0) {
      addToast($tStore('secrets.emptyInput' as any), 'error');
      return;
    }
    selectedRepoIds = new Set(projectRepos.filter(r => r.github_name).map(r => r.id));
    showPushConfirm = true;
  }

  function toggleRepo(id: number) {
    const next = new Set(selectedRepoIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedRepoIds = next;
  }

  function selectAllRepos() {
    selectedRepoIds = new Set(projectRepos.filter(r => r.github_name).map(r => r.id));
  }

  function deselectAllRepos() {
    selectedRepoIds = new Set();
  }

  async function confirmProjectPush() {
    if (!$pat) return;
    if (selectedRepoIds.size === 0) {
      addToast($tStore('secrets.noReposSelected' as any), 'error');
      return;
    }

    const result = parseEnvText(secretsText);
    const selectedRepos = projectRepos.filter(r => selectedRepoIds.has(r.id) && r.github_name);
    showPushConfirm = false;
    pushing = true;

    let failures = 0;
    const total = selectedRepos.length;

    for (let i = 0; i < selectedRepos.length; i++) {
      const repo = selectedRepos[i];
      pushProgress = $tStore('secrets.progressPushing' as any)
        .replace('{0}', String(i + 1))
        .replace('{1}', String(total));
      try {
        const { owner, repo: repoName } = splitRepoFullName(repo.github_name ?? '');
        await pushSecretsToRepo($pat, owner, repoName, result.secrets);
        // v0.20.0: record secret event per secret for timeline
        for (const s of result.secrets) {
          await recordSecretEvent(repo.id, 'set', s.name).catch((e) => console.warn('record_secret_event failed:', e));
        }
      } catch {
        failures++;
      }
    }

    pushing = false;
    pushProgress = '';

    if (failures === 0) {
      addToast($tStore('secrets.pushSuccess' as any).replace('{0}', `${total} repos`), 'success');
      secretsText = '';
      parseErrors = [];
    } else {
      addToast(
        $tStore('secrets.pushPartialFail' as any).replace('{0}', String(failures)).replace('{1}', String(total)),
        'error'
      );
    }
  }
</script>

<div class="secrets-section" class:flat={!collapsible}>
  {#if collapsible}
    <button class="ghost secrets-toggle" onclick={() => (collapsed = !collapsed)} type="button">
      {collapsed ? '▶' : '▼'} {$tStore('secrets.title' as any)}
    </button>
  {/if}

  {#if bodyOpen}
    <div class="secrets-body">
      <!-- Existing secrets list (repo mode only) -->
      {#if mode === 'repo'}
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
                    disabled={pushing || savingSecret === secret.name}
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
                  disabled={pushing}
                  type="button"
                >
                  {$tStore('secrets.deleteSelected' as any)} ({selectedSecrets.size})
                </button>
              </div>
            {/if}
          {/if}
        </div>
      {/if}

      <!-- Input textarea for new secrets -->
      <div class="new-secrets">
        <div class="sub-label">{$tStore('secrets.addNew' as any)}</div>
        <textarea
          class="secrets-textarea"
          bind:value={secretsText}
          oninput={handleTextInput}
          placeholder={$tStore('secrets.textareaPlaceholder' as any)}
          rows="4"
          disabled={pushing}
        ></textarea>

        {#if parseErrors.length > 0}
          <div class="parse-errors">
            {#each parseErrors as error}
              <div class="parse-error">{error}</div>
            {/each}
          </div>
        {/if}

        <div class="push-row">
          <button
            class="push-btn"
            onclick={mode === 'repo' ? handlePushRepo : handlePushProject}
            disabled={pushing || parseErrors.length > 0 || secretsText.trim() === ''}
            type="button"
          >
            {#if pushing}
              {pushProgress || $tStore('secrets.pushing' as any)}
            {:else}
              {$tStore('secrets.push' as any)}
            {/if}
          </button>
        </div>
      </div>
    </div>
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

<!-- Project push confirmation -->
{#if showPushConfirm}
  <ConfirmDialog
    title={$tStore('secrets.confirmPushTitle' as any)}
    message={$tStore('secrets.confirmPushMessage' as any)}
    onConfirm={confirmProjectPush}
    onCancel={() => (showPushConfirm = false)}
  >
    <div class="repo-checkboxes">
      <div class="select-actions">
        <button class="ghost mini" onclick={selectAllRepos} type="button">{$tStore('secrets.selectAll' as any)}</button>
        <button class="ghost mini" onclick={deselectAllRepos} type="button">{$tStore('secrets.deselectAll' as any)}</button>
      </div>
      {#each projectRepos.filter(r => r.github_name) as repo (repo.id)}
        <label class="repo-checkbox">
          <input
            type="checkbox"
            checked={selectedRepoIds.has(repo.id)}
            onchange={() => toggleRepo(repo.id)}
          />
          {getDisplayName(repo)}
          <span class="repo-full-name">{repo.github_name}</span>
        </label>
      {/each}
    </div>
  </ConfirmDialog>
{/if}

<style>
  .secrets-section {
    border-top: 1px solid var(--border);
    padding: 6px 0;
  }

  /* flat: panel is the sole content of a dedicated tab — no border-top (tab-nav already draws a divider) and no vertical padding (the tab wrapper provides it). */
  /* T-000129: in flat mode the panel fills its tab — flex column so .new-secrets
     can grow to absorb the unused vertical real estate below the existing-secrets
     list. Without this cascade .secrets-textarea stays pinned at rows="4" (~70px)
     and leaves half the viewport blank on wide screens. */
  .secrets-section.flat {
    border-top: none;
    padding: 0;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .secrets-section.flat .secrets-body {
    padding-top: 0;
    flex: 1;
    min-height: 0;
  }

  /* T-000129 follow-up: keep existing-secrets at natural height so
     .new-secrets gets the remaining flex space. Without flex-shrink:0
     the existing-list gets compressed when many secrets are present. */
  .secrets-section.flat .existing-secrets {
    flex-shrink: 0;
  }

  .secrets-section.flat .new-secrets {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .secrets-section.flat .new-secrets .secrets-textarea {
    flex: 1;
    min-height: 70px;
  }

  .secrets-toggle {
    font-size: 11px;
    padding: 2px 4px;
    color: var(--text-muted);
  }

  .secrets-toggle:hover {
    color: var(--accent);
  }

  .secrets-body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 8px;
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

  .new-secrets {
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }

  .secrets-textarea {
    width: 100%;
    font-size: 12px;
    font-family: monospace;
    padding: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    resize: vertical;
    min-height: 70px;
  }

  .secrets-textarea:focus {
    outline: none;
    border-color: var(--accent);
  }

  .secrets-textarea:disabled {
    opacity: 0.5;
  }

  .parse-errors {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 4px;
  }

  .parse-error {
    font-size: 11px;
    color: var(--danger, #ef4444);
    font-family: monospace;
  }

  .push-row {
    display: flex;
    justify-content: flex-end;
    margin-top: 6px;
  }

  .push-btn {
    font-size: 12px;
    padding: 5px 14px;
    border-radius: 4px;
    border: 1px solid var(--accent);
    background: transparent;
    color: var(--accent);
    cursor: pointer;
  }

  .push-btn:hover:not(:disabled) {
    background-color: var(--accent);
    color: white;
  }

  .push-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .repo-checkboxes {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 250px;
    overflow-y: auto;
    margin: 8px 0;
  }

  .select-actions {
    display: flex;
    gap: 8px;
    margin-bottom: 4px;
  }

  .select-actions .mini {
    font-size: 11px;
    color: var(--accent);
    padding: 0;
  }

  .repo-checkbox {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    cursor: pointer;
    padding: 3px 0;
  }

  .repo-checkbox input[type="checkbox"] {
    cursor: pointer;
  }

  .repo-full-name {
    font-size: 11px;
    color: var(--text-muted);
    margin-left: auto;
    font-family: monospace;
  }
</style>
