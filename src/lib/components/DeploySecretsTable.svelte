<script lang="ts">
  import { onMount } from 'svelte';
  import { allRepos } from '$lib/stores/repos';
  import { pat } from '$lib/stores/settings';
  import { tStore } from '$lib/i18n';
  import { addToast } from '$lib/stores/ui';
  import {
    listDeploySecrets, upsertDeploySecret,
    ensureDeploySecretsPopulated, recordDeploySecretEvent,
    listSecretBundles, getBundleDecrypted,
  } from '$lib/api/tauri-commands';
  import { mergeBundleValues } from '$lib/api/bundle-apply';
  import {
    listRepoSecrets, listEnvironmentSecrets, getEnvironmentPublicKey,
    createOrUpdateEnvironmentSecret, deleteEnvironmentSecret,
    splitRepoFullName, type RepoSecret, type EnvironmentSecret,
  } from '$lib/api/github';
  import { encryptSecret } from '$lib/api/secrets-crypto';
  import type { DeploySecret, DeploySecretRole, SecretBundle } from '$lib/types';

  interface Props {
    deployEnvId: number;
    envName: string;
    repoId: number;
    /** Notifies parent that workflow files are stale (role changes affect
     *  which YAML section the secret renders into). M9 review-fix. */
    onRoleChange?: () => void;
  }
  let { deployEnvId, envName, repoId, onRoleChange }: Props = $props();

  let dbSecrets = $state<DeploySecret[]>([]);
  let repoSecretsFromGitHub = $state<RepoSecret[]>([]);
  let envSecretsFromGitHub = $state<EnvironmentSecret[]>([]);
  let values = $state<Record<string, string>>({});
  let bundles = $state<SecretBundle[]>([]);
  let loading = $state(true);

  const repo = $derived($allRepos.find((r) => r.id === repoId) ?? null);
  /** True when PAT missing OR repo not linked to GitHub — disables GitHub-dependent flows. */
  const githubUnavailable = $derived(!$pat || !repo?.github_name);

  function ownerRepo(): { owner: string; repo: string } | null {
    if (!repo?.github_name) return null;
    const parts = splitRepoFullName(repo.github_name);
    return parts;
  }

  async function load() {
    loading = true;
    try {
      const or = ownerRepo();
      if (or && $pat) {
        // Step 1: list repo secrets from GitHub
        repoSecretsFromGitHub = await listRepoSecrets($pat, or.owner, or.repo);
        // Step 2: seed deploy_secrets (idempotent — first open populates, subsequent are no-op)
        await ensureDeploySecretsPopulated(deployEnvId, repoSecretsFromGitHub.map((s) => s.name));
        // Step 3: list env-scoped secrets
        try {
          envSecretsFromGitHub = await listEnvironmentSecrets($pat, or.owner, or.repo, envName);
        } catch {
          // Env might not exist yet — treat as empty
          envSecretsFromGitHub = [];
        }
      }
      // Step 4: list deploy_secrets from DB (after seed)
      dbSecrets = await listDeploySecrets(deployEnvId);
      // Step 5: load bundles for the apply control (best-effort)
      try { bundles = await listSecretBundles(); } catch { bundles = []; }
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      loading = false;
    }
  }

  async function toggleIncluded(s: DeploySecret) {
    const willBeIncluded = !s.included;
    const newOverride = s.override_enabled && willBeIncluded;
    await upsertDeploySecret(deployEnvId, s.secret_name, s.role ?? 'deploy', willBeIncluded, newOverride);
    // Optimistic update — no full re-render
    dbSecrets = dbSecrets.map(x =>
      x.secret_name === s.secret_name
        ? { ...x, included: willBeIncluded, override_enabled: newOverride }
        : x
    );
    if (s.included && s.override_enabled && $pat) {
      const or = ownerRepo();
      if (or) {
        try {
          await deleteEnvironmentSecret($pat, or.owner, or.repo, envName, s.secret_name);
          envSecretsFromGitHub = envSecretsFromGitHub.filter(e => e.name !== s.secret_name);
          // v0.20.0: record deploy secret event for timeline
          await recordDeploySecretEvent(deployEnvId, repoId, 'env_secret_delete', s.secret_name).catch((e) => console.warn(e));
        } catch {}
      }
    }
  }

  async function changeRole(s: DeploySecret, role: DeploySecretRole) {
    if (s.role === role) return;
    await upsertDeploySecret(deployEnvId, s.secret_name, role, s.included, s.override_enabled);
    // Optimistic update — no full re-render
    dbSecrets = dbSecrets.map(x =>
      x.secret_name === s.secret_name ? { ...x, role } : x
    );
    onRoleChange?.();
  }

  async function cycleRole(s: DeploySecret) {
    if (!s.included) return;
    const order: DeploySecretRole[] = ['build', 'deploy', 'runtime'];
    const cur = (s.role ?? 'deploy') as DeploySecretRole;
    const idx = order.indexOf(cur);
    const next = order[(idx + 1) % order.length];
    await changeRole(s, next);
  }

  async function toggleOverride(s: DeploySecret) {
    const next = !s.override_enabled;
    await upsertDeploySecret(deployEnvId, s.secret_name, s.role ?? 'deploy', s.included, next);
    // Optimistic update — no full re-render
    dbSecrets = dbSecrets.map(x =>
      x.secret_name === s.secret_name ? { ...x, override_enabled: next } : x
    );
    if (!next && $pat) {
      const or = ownerRepo();
      if (or) {
        try {
          await deleteEnvironmentSecret($pat, or.owner, or.repo, envName, s.secret_name);
          envSecretsFromGitHub = envSecretsFromGitHub.filter(e => e.name !== s.secret_name);
          // v0.20.0: record deploy secret event for timeline
          await recordDeploySecretEvent(deployEnvId, repoId, 'env_secret_delete', s.secret_name).catch((e) => console.warn(e));
        } catch {}
      }
    }
  }

  async function saveValue(s: DeploySecret) {
    const value = (values[s.secret_name] ?? '').trim();
    if (!value) return;
    if (!$pat) {
      addToast($tStore('deploy.githubRequired' as any) || 'GitHub token required', 'error');
      return;
    }
    const or = ownerRepo();
    if (!or) {
      addToast($tStore('deploy.githubRequired' as any) || 'GitHub repo required', 'error');
      return;
    }
    try {
      const { key, key_id } = await getEnvironmentPublicKey($pat, or.owner, or.repo, envName);
      const encrypted = await encryptSecret(key, value);
      await createOrUpdateEnvironmentSecret($pat, or.owner, or.repo, envName, s.secret_name, encrypted, key_id);
      // v0.20.0: record deploy secret event for timeline
      await recordDeploySecretEvent(deployEnvId, repoId, 'env_secret_set', s.secret_name).catch((e) => console.warn(e));
      addToast(($tStore('deploy.secretSaved' as any) || 'Secret "{0}" saved').replace('{0}', s.secret_name), 'success');
      values[s.secret_name] = '';
      // B-000009: optimistic update of envSecretsFromGitHub instead of `await load()`.
      // A full reload replaces dbSecrets/envSecretsFromGitHub arrays → keyed each
      // block re-renders → focus is lost from the next textarea the user tabs to.
      // We know the push succeeded; mark the env secret as just-updated locally.
      const nowIso = new Date().toISOString();
      const existing = envSecretsFromGitHub.find((e) => e.name === s.secret_name);
      if (existing) {
        envSecretsFromGitHub = envSecretsFromGitHub.map((e) =>
          e.name === s.secret_name ? { ...e, updated_at: nowIso } : e
        );
      } else {
        envSecretsFromGitHub = [
          ...envSecretsFromGitHub,
          { name: s.secret_name, created_at: nowIso, updated_at: nowIso },
        ];
      }
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  // Fill the per-secret value inputs from a bundle (bundle wins on name clash),
  // but only for declared rows — unmatched bundle names have no row/push path.
  // Only fills inputs; the user still reviews and pushes via the existing controls.
  async function applyBundle(bundleId: number) {
    const bundle = bundles.find((b) => b.id === bundleId);
    if (!bundle) return;
    const items = await getBundleDecrypted(bundleId);
    const known = new Set(dbSecrets.map((s) => s.secret_name));
    const filtered = items.filter((i) => known.has(i.secret_name));
    values = mergeBundleValues(values, filtered);
    addToast($tStore('bundles.appliedToast' as any).replace('{0}', bundle.name), 'success');
  }

  function repoSecretMeta(name: string): RepoSecret | null {
    return repoSecretsFromGitHub.find((r) => r.name === name) ?? null;
  }
  function envSecretMeta(name: string): EnvironmentSecret | null {
    return envSecretsFromGitHub.find((r) => r.name === name) ?? null;
  }

  onMount(load);
</script>

<div class="deploy-secrets-table">
  <div class="header-row">
    {#if !loading && bundles.length > 0 && dbSecrets.length > 0}
      <select
        class="bundle-apply-select"
        value=""
        onchange={(e) => {
          const t = e.target as HTMLSelectElement;
          const id = Number(t.value);
          t.value = '';
          if (id) applyBundle(id);
        }}
      >
        <option value="" disabled>{$tStore('bundles.applyPrompt' as any)}</option>
        {#each bundles as b (b.id)}
          <option value={b.id}>{b.name}</option>
        {/each}
      </select>
    {/if}
    <button class="ghost mini refresh-btn"
            onclick={load}
            disabled={loading}
            title={$tStore('secrets.refresh' as any) || 'Refresh from GitHub'}
            type="button">{loading ? '⟳' : '↻'}</button>
  </div>
  {#if loading}
    <p>{$tStore('common.loading' as any)}</p>
  {:else if githubUnavailable && dbSecrets.length === 0}
    <p class="empty-hint">{$tStore('deploy.githubTokenRequiredForSecrets' as any) || 'Set a GitHub token in Settings to load this deployment\'s secrets list.'}</p>
  {:else}
    {#each dbSecrets as s (s.secret_name)}
      <div class="secret-row" class:disabled={!s.included}>
        <span class="secret-name">{s.secret_name}</span>
        <button class="role-chip role-{s.role ?? 'deploy'}"
                onclick={() => cycleRole(s)}
                disabled={!s.included}
                title={$tStore('deploy.roleTooltip' as any) || 'build = baked into image at compile time / deploy = workflow context / runtime = docker run --env (click to change)'}>
          {s.role ?? 'deploy'}
        </button>
        <label class="override-toggle" class:muted={!s.included}>
          <input type="checkbox" checked={s.override_enabled} disabled={!s.included} onchange={() => toggleOverride(s)} />
          {$tStore('deploy.overrideCheckbox' as any) || 'Override'}
        </label>
        <textarea class="value-input"
                  rows="1"
                  spellcheck="false"
                  autocomplete="off"
                  bind:value={values[s.secret_name]}
                  disabled={!s.included || !s.override_enabled}
                  placeholder={!s.included
                    ? ($tStore('deploy.notIncluded' as any) || 'not included')
                    : !s.override_enabled
                      ? (repoSecretMeta(s.secret_name)
                          ? ($tStore('deploy.inheritedFromRepo' as any) || '(from repo, updated {0})').replace('{0}', repoSecretMeta(s.secret_name)!.updated_at.slice(0, 10))
                          : ($tStore('deploy.notSetInRepo' as any) || 'Not set in repo'))
                      : (envSecretMeta(s.secret_name)
                          ? ($tStore('deploy.overrideSavedHint' as any) || 'saved {0}').replace('{0}', envSecretMeta(s.secret_name)!.updated_at.slice(0, 10))
                          : ($tStore('deploy.enterOverrideValue' as any) || 'Enter value'))}
                  onblur={() => saveValue(s)}></textarea>
        <label class="include-toggle">
          <input type="checkbox" checked={s.included} onchange={() => toggleIncluded(s)} />
          {$tStore('deploy.includeCheckbox' as any) || 'Include'}
        </label>
      </div>
    {/each}

  {/if}
</div>

<style>
  .secret-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.4rem 0;
    border-bottom: 1px solid var(--border-light);
  }
  .secret-row.disabled .secret-name { opacity: 0.55; }
  .secret-row.disabled .role-chip { opacity: 0.4; pointer-events: none; }

  .secret-name {
    font-weight: 600;
    font-family: var(--font-mono, monospace);
    font-size: 0.95em;
    min-width: 9rem;
    flex-shrink: 0;
  }
  .role-chip {
    border: none;
    border-radius: 3px;
    padding: 0.1rem 0.45rem;
    font-size: 0.75em;
    font-weight: 600;
    text-transform: uppercase;
    cursor: pointer;
    color: white;
  }
  .role-chip.role-build { background: #6366f1; }      /* indigo */
  .role-chip.role-deploy { background: #14b8a6; }     /* teal */
  .role-chip.role-runtime { background: #f59e0b; }    /* amber */
  .role-chip:disabled { cursor: not-allowed; }
  .spacer { flex: 1; }
  .include-toggle, .override-toggle {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    font-size: 0.9em;
    cursor: pointer;
    user-select: none;
  }
  .override-toggle.muted {
    opacity: 0.5;
  }
  .value-input {
    flex: 1;
    padding: 0.35rem 0.5rem;
    box-sizing: border-box;
    font-family: var(--font-mono, monospace);
    font-size: 0.85em;
    resize: vertical;
    min-height: calc(0.85em * 1.5 + 0.7rem);
    white-space: pre;
    overflow: auto;
    /* Mask like password — textarea accepts multi-line paste, user sees dots.
       Mirrors SecretsPanel.svelte `.secret-value-input` pattern. */
    -webkit-text-security: disc;
  }
  .value-input:focus {
    min-height: 80px;
  }
  .value-input:disabled {
    opacity: 0.6;
    background: var(--hover-bg);
  }
  .empty-hint {
    color: var(--text-muted);
    padding: 1rem;
  }
  .header-row {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-bottom: 0.4rem;
  }
  .bundle-apply-select {
    font-size: 11px;
    padding: 2px 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    cursor: pointer;
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
</style>
