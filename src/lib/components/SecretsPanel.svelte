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
    getRepoPublicKey, createOrUpdateRepoSecret,
    splitRepoFullName,
  } from '$lib/api/github';
  import { encryptSecret } from '$lib/api/secrets-crypto';
  import { parseEnvText } from '$lib/api/secrets-parser';
  import { type SecretBundle } from '$lib/types';
  import SecretsList from './SecretsList.svelte';
  import { allRepos } from '$lib/stores/repos';
  import {
    registerRepoSecretInDeploys,
    recordSecretEvent, listSecretBundles, getBundleDecrypted,
  } from '$lib/api/tauri-commands';
  import { mergeBundleIntoEnvText } from '$lib/api/bundle-apply';

  interface Props {
    repoFullName?: string;
    /**
     * Show the header-toggle that collapses the body. Default `true` (legacy behaviour in
     * ProjectDetail where SecretsPanel sits alongside other collapsible sections).
     * Pass `false` when the panel is already the sole content of a dedicated tab
     * (e.g. RepoDetail's "Секреты" tab) — tab-inside-tab collapse is redundant (B-003).
     */
    collapsible?: boolean;
  }

  let {
    repoFullName,
    collapsible = true,
  }: Props = $props();

  // State
  let secretsText = $state('');
  let bundles = $state<SecretBundle[]>([]);
  let pushing = $state(false);
  let collapsed = $state(true);
  // Effective open-state for rendering: always open if panel isn't collapsible (dedicated tab).
  const bodyOpen = $derived(!collapsible || !collapsed);
  let parseErrors = $state<string[]>([]);

  // Child ref — parent triggers a refresh + verify after push via reload().
  let secretsListRef = $state<ReturnType<typeof SecretsList> | null>(null);

  // Load bundles once — repo-independent, reused across repo switches.
  $effect(() => {
    if (bodyOpen && $pat && bundles.length === 0) {
      listSecretBundles().then((b) => { bundles = b; }).catch(() => { bundles = []; });
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

  function handleTextInput() {
    if (secretsText.trim() === '') {
      parseErrors = [];
      return;
    }
    const result = parseEnvText(secretsText);
    parseErrors = result.errors;
  }

  // Merge a bundle's values into the bulk textarea (bundle wins on name clash).
  // Only fills the input — the user still reviews and pushes via the existing button.
  async function applyBundle(bundleId: number) {
    const bundle = bundles.find((b) => b.id === bundleId);
    if (!bundle) return;
    const items = await getBundleDecrypted(bundleId);
    secretsText = mergeBundleIntoEnvText(secretsText, items);
    handleTextInput();
    addToast($tStore('bundles.appliedToast' as any).replace('{0}', bundle.name), 'success');
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
      // Post-push verification: re-fetch the list (via the child, which also
      // resets its selection) and ensure every pushed name actually landed.
      // Catches silent "API returned 201 but secret not saved" cases (and gives
      // the user a real error instead of a misleading success).
      const fresh = await secretsListRef?.reload() ?? [];
      const freshNames = new Set(fresh.map((s) => s.name));
      const missing = result.secrets.filter((s) => !freshNames.has(s.name)).map((s) => s.name);
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

</script>

<div class="secrets-section" class:flat={!collapsible}>
  {#if collapsible}
    <button class="ghost secrets-toggle" onclick={() => (collapsed = !collapsed)} type="button">
      {collapsed ? '▶' : '▼'} {$tStore('secrets.title' as any)}
    </button>
  {/if}

  {#if bodyOpen}
    <div class="secrets-body">
      <!-- Existing secrets list -->
      <SecretsList {repoFullName} busy={pushing} bind:this={secretsListRef} />

      <!-- Input textarea for new secrets -->
      <div class="new-secrets">
        <div class="new-secrets-label-row">
          <div class="sub-label">{$tStore('secrets.addNew' as any)}</div>
          {#if bundles.length > 0}
            <select
              class="bundle-apply-select"
              value=""
              onchange={(e) => {
                const t = e.target as HTMLSelectElement;
                const id = Number(t.value);
                t.value = '';
                if (id) applyBundle(id);
              }}
              disabled={pushing}
            >
              <option value="" disabled>{$tStore('bundles.applyPrompt' as any)}</option>
              {#each bundles as b (b.id)}
                <option value={b.id}>{b.name}</option>
              {/each}
            </select>
          {/if}
        </div>
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
            onclick={handlePushRepo}
            disabled={pushing || parseErrors.length > 0 || secretsText.trim() === ''}
            type="button"
          >
            {#if pushing}
              {$tStore('secrets.pushing' as any)}
            {:else}
              {$tStore('secrets.push' as any)}
            {/if}
          </button>
        </div>
      </div>
    </div>
  {/if}
</div>

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

  /* T-000129 follow-up: keeping existing-secrets at natural height (flex-shrink:0)
     so .new-secrets gets the remaining flex space now lives inside SecretsList's
     own scoped stylesheet — Svelte scoped styles don't cross component boundaries. */

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

  .new-secrets {
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }

  .new-secrets-label-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 6px;
  }

  .new-secrets-label-row .sub-label {
    margin-bottom: 0;
  }

  .bundle-apply-select {
    margin-left: auto;
    font-size: 11px;
    padding: 2px 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 4px;
    color: var(--text);
    cursor: pointer;
  }

  .bundle-apply-select:disabled {
    opacity: 0.4;
    cursor: not-allowed;
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
</style>
