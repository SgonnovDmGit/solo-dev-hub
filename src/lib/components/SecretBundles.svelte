<script lang="ts">
  import { onMount } from 'svelte';
  import { addToast } from '$lib/stores/ui';
  import { tStore } from '$lib/i18n';
  import type { SecretBundle, SecretBundleItemValue } from '$lib/types';
  import {
    listSecretBundles, createSecretBundle, renameSecretBundle,
    deleteSecretBundle, upsertBundleItem, getBundleDecrypted, deleteBundleItem,
  } from '$lib/api/tauri-commands';
  import { parseEnvText } from '$lib/api/secrets-parser';
  import ConfirmDialog from './ConfirmDialog.svelte';

  // ── Master state ────────────────────────────────────────────────────────────
  let bundles = $state<SecretBundle[]>([]);
  let selectedId = $state<number | null>(null);
  let items = $state<SecretBundleItemValue[]>([]); // decrypted items of open bundle
  let revealed = $state<Record<string, boolean>>({}); // per-secret-name reveal toggle

  const selectedBundle = $derived(
    selectedId == null ? null : (bundles.find((b) => b.id === selectedId) ?? null),
  );

  // ── New-bundle form (collapsed disclosure — button until opened) ─────────────
  let showNewForm = $state(false);
  let newName = $state('');
  let newDescription = $state('');
  let creating = $state(false);

  // ── Inline name/description edit (mirror of selected bundle) ─────────────────
  let editName = $state('');
  let editDescription = $state('');

  // ── Add-secret form (bulk KEY=VALUE) ────────────────────────────────────────
  let bulkText = $state('');

  // ── Delete-bundle confirm ───────────────────────────────────────────────────
  let bundleToDelete = $state<SecretBundle | null>(null);

  onMount(async () => {
    await reloadBundles();
  });

  async function reloadBundles() {
    try {
      bundles = await listSecretBundles();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function selectBundle(id: number) {
    selectedId = id;
    revealed = {};
    bulkText = '';
    const b = bundles.find((x) => x.id === id) ?? null;
    editName = b?.name ?? '';
    editDescription = b?.description ?? '';
    try {
      items = await getBundleDecrypted(id);
    } catch (err) {
      items = [];
      addToast(String(err), 'error');
    }
  }

  async function handleCreate() {
    const name = newName.trim();
    if (!name) {
      addToast($tStore('bundles.nameRequired' as any), 'error');
      return;
    }
    creating = true;
    try {
      const id = await createSecretBundle(name, newDescription.trim());
      newName = '';
      newDescription = '';
      showNewForm = false;
      await reloadBundles();
      await selectBundle(id);
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      creating = false;
    }
  }

  function cancelNewForm() {
    newName = '';
    newDescription = '';
    showNewForm = false;
  }

  async function saveBundleMeta() {
    if (selectedId == null) return;
    const name = editName.trim();
    if (!name) {
      addToast($tStore('bundles.nameRequired' as any), 'error');
      // revert visible field to the last persisted name
      editName = selectedBundle?.name ?? '';
      return;
    }
    try {
      await renameSecretBundle(selectedId, name, editDescription.trim());
      await reloadBundles();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function confirmDeleteBundle() {
    if (selectedBundle) bundleToDelete = selectedBundle;
  }

  async function handleDeleteBundle() {
    const b = bundleToDelete;
    bundleToDelete = null;
    if (!b) return;
    try {
      await deleteSecretBundle(b.id);
      if (selectedId === b.id) {
        selectedId = null;
        items = [];
        revealed = {};
      }
      await reloadBundles();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function toggleReveal(name: string) {
    revealed = { ...revealed, [name]: !revealed[name] };
  }

  async function saveItemValue(item: SecretBundleItemValue, value: string) {
    if (selectedId == null) return;
    if (value === item.value) return; // no-op on unchanged blur
    try {
      await upsertBundleItem(selectedId, item.secret_name, value);
      item.value = value;
      await reloadBundles();
      // non-noisy: no per-blur toast
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function handleDeleteSecret(item: SecretBundleItemValue) {
    if (selectedId == null) return;
    try {
      await deleteBundleItem(item.id);
      items = await getBundleDecrypted(selectedId);
      await reloadBundles();
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  async function handleAddSecrets() {
    if (selectedId == null) return;
    const text = bulkText.trim();
    if (!text) return;
    const { secrets, errors } = parseEnvText(text);
    if (errors.length > 0) {
      addToast(errors.join('\n'), 'error');
      return;
    }
    try {
      for (const s of secrets) {
        await upsertBundleItem(selectedId, s.name, s.value);
      }
      bulkText = '';
      items = await getBundleDecrypted(selectedId);
      await reloadBundles();
      addToast($tStore('bundles.savedToast' as any), 'success');
    } catch (err) {
      addToast(String(err), 'error');
    }
  }
</script>

<div class="bundles-screen">
  <header class="screen-head">
    <h2>{$tStore('bundles.title' as any)}</h2>
    <p class="subtitle">{$tStore('bundles.screenSubtitle' as any)}</p>
  </header>

  <div class="layout">
    <!-- ── Master: bundle list ────────────────────────────────────────────── -->
    <aside class="master">
      {#if bundles.length === 0}
        <p class="empty-note">{$tStore('bundles.empty' as any)}</p>
      {:else}
        <ul class="bundle-list">
          {#each bundles as b (b.id)}
            <li>
              <button
                class="bundle-row"
                class:active={selectedId === b.id}
                onclick={() => selectBundle(b.id)}
              >
                <span class="bundle-name">{b.name}</span>
                <span class="bundle-count">
                  {$tStore('bundles.secretsCount' as any).replace('{0}', String(b.secret_names.length))}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}

      <div class="new-bundle-wrap">
        {#if !showNewForm}
          <button class="new-bundle-btn" onclick={() => (showNewForm = true)}>
            {$tStore('bundles.newBundle' as any)}
          </button>
        {:else}
          <div class="new-bundle-form">
            <!-- svelte-ignore a11y_autofocus -->
            <input
              type="text"
              bind:value={newName}
              placeholder={$tStore('bundles.namePlaceholder' as any)}
              autofocus
            />
            <input
              type="text"
              bind:value={newDescription}
              placeholder={$tStore('bundles.descriptionPlaceholder' as any)}
            />
            <div class="new-bundle-actions">
              <button class="primary" onclick={handleCreate} disabled={creating || !newName.trim()}>
                {$tStore('bundles.create' as any)}
              </button>
              <button class="ghost" onclick={cancelNewForm} disabled={creating}>
                {$tStore('bundles.cancel' as any)}
              </button>
            </div>
          </div>
        {/if}
      </div>
    </aside>

    <!-- ── Detail: selected bundle editor ─────────────────────────────────── -->
    <section class="detail">
      {#if selectedBundle == null}
        <p class="empty-note">{$tStore('bundles.empty' as any)}</p>
      {:else}
        <div class="detail-head">
          <div class="meta-edit">
            <input
              class="name-edit"
              type="text"
              bind:value={editName}
              placeholder={$tStore('bundles.namePlaceholder' as any)}
              onblur={saveBundleMeta}
            />
            <input
              class="desc-edit"
              type="text"
              bind:value={editDescription}
              placeholder={$tStore('bundles.descriptionPlaceholder' as any)}
              onblur={saveBundleMeta}
            />
          </div>
          <button class="danger-btn" onclick={confirmDeleteBundle}>
            {$tStore('bundles.deleteBundle' as any)}
          </button>
        </div>

        {#if items.length === 0}
          <p class="empty-note">{$tStore('bundles.noSecrets' as any)}</p>
        {:else}
          <div class="items">
            {#each items as item (item.secret_name)}
              <div class="secret-row">
                <span class="item-name">{item.secret_name}</span>
                <textarea
                  class="item-value"
                  class:revealed={revealed[item.secret_name]}
                  rows="1"
                  spellcheck="false"
                  autocomplete="off"
                  value={item.value}
                  onblur={(e) => saveItemValue(item, (e.currentTarget as HTMLTextAreaElement).value)}
                ></textarea>
                <div class="item-actions">
                  <button
                    class="icon"
                    onclick={() => toggleReveal(item.secret_name)}
                    title={revealed[item.secret_name] ? $tStore('bundles.hide' as any) : $tStore('bundles.reveal' as any)}
                    aria-label={revealed[item.secret_name] ? $tStore('bundles.hide' as any) : $tStore('bundles.reveal' as any)}
                  >👁</button>
                  <button
                    class="icon"
                    onclick={() => handleDeleteSecret(item)}
                    title={$tStore('bundles.deleteSecret' as any)}
                    aria-label={$tStore('bundles.deleteSecret' as any)}
                  >🗑</button>
                </div>
              </div>
            {/each}
          </div>
        {/if}

        <div class="add-secret">
          <textarea
            class="bulk-input"
            bind:value={bulkText}
            rows="4"
            placeholder={$tStore('bundles.bulkPlaceholder' as any)}
          ></textarea>
          <button class="primary add-secret-btn" onclick={handleAddSecrets} disabled={!bulkText.trim()}>
            {$tStore('bundles.addSecretsBulk' as any)}
          </button>
        </div>
      {/if}
    </section>
  </div>
</div>

{#if bundleToDelete}
  <ConfirmDialog
    title={$tStore('bundles.deleteBundle' as any)}
    message={$tStore('bundles.deleteBundleConfirm' as any).replace('{0}', bundleToDelete.name)}
    onConfirm={handleDeleteBundle}
    onCancel={() => bundleToDelete = null}
  />
{/if}

<style>
  .bundles-screen { padding: 1rem; }
  .screen-head { margin-bottom: 1rem; }
  .screen-head h2 { margin: 0 0 0.25rem 0; font-size: 1.2rem; }
  .subtitle { color: var(--text-muted); font-size: 0.9em; margin: 0; }

  .layout {
    display: grid;
    grid-template-columns: 19rem 1fr;
    gap: 1rem;
    align-items: start;
  }

  /* ── Master ──────────────────────────────────────────────────────────────── */
  .master {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    padding: 0.5rem;
  }
  .bundle-list { list-style: none; margin: 0; padding: 0; }
  .bundle-list li + li .bundle-row { border-top: 1px solid var(--border); }
  .bundle-row {
    width: 100%;
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.6rem;
    text-align: left;
    padding: 0.55rem 0.6rem;
    background: transparent;
    border: 0;
    cursor: pointer;
    color: var(--text);
    font-family: inherit;
    font-size: 0.95rem;
  }
  .bundle-row:hover { background: var(--surface-hover); }
  .bundle-row.active { background: var(--surface-hover); }
  .bundle-row.active .bundle-name { color: var(--accent); }
  .bundle-name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bundle-count { color: var(--text-muted); font-size: 0.8em; white-space: nowrap; flex-shrink: 0; }

  .new-bundle-wrap {
    margin-top: 0.5rem;
    padding-top: 0.5rem;
    border-top: 1px solid var(--border);
  }
  .new-bundle-btn {
    width: 100%;
    padding: 0.5rem;
    background: transparent;
    color: var(--accent);
    border: 1px dashed var(--border);
    border-radius: 5px;
    cursor: pointer;
    font-weight: 500;
    font-family: inherit;
  }
  .new-bundle-btn:hover { background: var(--surface-hover); border-color: var(--accent); }
  .new-bundle-form { display: flex; flex-direction: column; gap: 0.45rem; }
  .new-bundle-form input { padding: 0.4rem 0.55rem; box-sizing: border-box; }
  .new-bundle-actions { display: flex; gap: 0.4rem; }
  .new-bundle-actions .primary { flex: 1; }

  /* ── Detail ──────────────────────────────────────────────────────────────── */
  .detail {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.9rem;
    min-height: 14rem;
    background: var(--surface);
  }
  .detail-head {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
    margin-bottom: 0.9rem;
  }
  .meta-edit {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex: 1;
  }
  .name-edit { padding: 0.45rem 0.6rem; font-weight: 600; box-sizing: border-box; }
  .desc-edit { padding: 0.45rem 0.6rem; color: var(--text-muted); box-sizing: border-box; }

  .items { display: flex; flex-direction: column; gap: 0.3rem; margin-bottom: 0.9rem; }
  .secret-row {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    padding: 0.3rem 0.5rem;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
  }
  .secret-row:hover { background: var(--surface); }
  .item-name {
    font-weight: 600;
    font-family: monospace;
    font-size: 0.8rem;
    min-width: 8.5rem;
    flex-shrink: 0;
    padding-top: 0.4rem;
    word-break: break-word;
  }
  .item-value {
    flex: 1;
    min-width: 0;
    font-family: monospace;
    font-size: 0.8rem;
    line-height: 1.5;
    padding: 0.25rem 0.45rem;
    box-sizing: border-box;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 3px;
    color: var(--text);
    resize: vertical;
    white-space: pre;
    overflow: auto;
    min-height: 1.6rem;
    max-height: 12rem;
    /* Mask like a password — textarea still accepts multi-line keys (SSH_KEY). */
    -webkit-text-security: disc;
  }
  .item-value:focus { outline: none; border-color: var(--accent); min-height: 5rem; }
  .item-value.revealed { -webkit-text-security: none; }
  .item-actions { display: flex; gap: 0.1rem; flex-shrink: 0; padding-top: 0.15rem; }
  .item-actions .icon {
    background: transparent;
    border: 0;
    cursor: pointer;
    font-size: 1rem;
    padding: 0.25rem 0.4rem;
    color: var(--text-muted);
    border-radius: 3px;
  }
  .item-actions .icon:hover { color: var(--text); background: var(--surface-hover); }

  .add-secret {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0.6rem;
    padding: 0.6rem;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
  }
  .bulk-input {
    width: 100%;
    box-sizing: border-box;
    min-height: 5rem;
    font-family: monospace;
    font-size: 0.8rem;
    padding: 0.5rem;
    resize: vertical;
  }
  .add-secret-btn { align-self: flex-end; }

  .empty-note { color: var(--text-muted); padding: 0.5rem; }

  .primary {
    padding: 0.45rem 1rem;
    border: 1px solid var(--accent);
    background: transparent;
    color: var(--accent);
    border-radius: 5px;
    cursor: pointer;
    font-weight: 500;
  }
  .primary:hover:not(:disabled) { background: var(--accent); color: #fff; }
  .primary:disabled { opacity: 0.4; cursor: not-allowed; }

  .ghost {
    padding: 0.45rem 0.9rem;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    border-radius: 5px;
    cursor: pointer;
    font-weight: 500;
  }
  .ghost:hover:not(:disabled) { background: var(--surface-hover); }
  .ghost:disabled { opacity: 0.4; cursor: not-allowed; }

  .danger-btn {
    padding: 0.45rem 0.9rem;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    border-radius: 5px;
    font-weight: 500;
    flex-shrink: 0;
  }
  .danger-btn:hover { background: var(--surface-hover); border-color: var(--danger); color: var(--danger); }
</style>
