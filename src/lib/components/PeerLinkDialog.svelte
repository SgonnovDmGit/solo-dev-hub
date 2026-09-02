<script lang="ts">
  // T-000193: modal for declaring a peer link. Every peer pair is explicit —
  // nothing is inferred from a shared role — so this dialog is the only way a
  // channel between equals comes into existence.
  import { tStore } from '$lib/i18n';
  import { getDisplayName, type Project, type Repository } from '$lib/types';

  interface Props {
    /** Repos of the current project — the left-hand side of the pair. */
    myRepos: Repository[];
    /** Every repo in the portfolio — the right-hand side. */
    allRepos: Repository[];
    /** Every project, so a candidate's project type can be compared. */
    projects: Project[];
    /** Already-linked pairs as "a::b" keys, so they can be marked. */
    linkedKeys: Set<string>;
    onLink: (selfId: number, peerId: number) => void;
    onCancel: () => void;
  }

  let { myRepos, allRepos, projects, linkedKeys, onLink, onCancel }: Props = $props();

  let selfId = $state<number | null>(null);
  let peerId = $state<number | null>(null);
  let filter = $state('');

  const projectTypeById = $derived(new Map(projects.map((p) => [p.id, p.project_type])));
  const selfRepo = $derived(myRepos.find((r) => r.id === selfId));

  // T-000201: why these two are not equals, or null when they are. Mirrors
  // `AppDb::check_peers_are_equal` in src-tauri/src/db/peers.rs — that one
  // enforces, this one explains. Change both together.
  function inequalityReason(a: Repository | undefined, b: Repository): string | null {
    if (!a) return null;
    if (!a.role || !b.role) return $tStore('project.peerNeedsRole' as any);
    if (a.role === 'other' || b.role === 'other') return $tStore('project.peerRoleOther' as any);
    if (a.role !== b.role) return $tStore('project.peerRoleMismatch' as any);
    const ta = a.project_id === null ? undefined : projectTypeById.get(a.project_id);
    const tb = b.project_id === null ? undefined : projectTypeById.get(b.project_id);
    if (ta === undefined || tb === undefined) return $tStore('project.peerNeedsProject' as any);
    if (ta !== tb) return $tStore('project.peerTypeMismatch' as any);
    return null;
  }

  const selectedReason = $derived.by(() => {
    const b = peerId === null ? undefined : allRepos.find((r) => r.id === peerId);
    return b ? inequalityReason(selfRepo, b) : null;
  });

  // Preselect this project's first repo so the dialog is usable without a first
  // click, without capturing the prop's initial value at construction time.
  $effect(() => {
    if (selfId === null && myRepos.length > 0) selfId = myRepos[0].id;
  });

  const candidates = $derived(
    allRepos
      .filter((r) => r.id !== selfId)
      .filter((r) => {
        const q = filter.trim().toLowerCase();
        if (!q) return true;
        return (
          getDisplayName(r).toLowerCase().includes(q) ||
          (r.role ?? '').toLowerCase().includes(q)
        );
      })
  );

  function pairKey(a: number, b: number): string {
    return a < b ? `${a}::${b}` : `${b}::${a}`;
  }

  const alreadyLinked = $derived(
    selfId !== null && peerId !== null && linkedKeys.has(pairKey(selfId, peerId))
  );

  function submit() {
    if (selfId === null || peerId === null || alreadyLinked || selectedReason !== null) return;
    onLink(selfId, peerId);
  }
</script>

<div
  class="overlay"
  role="presentation"
  onclick={onCancel}
  onkeydown={(e) => e.key === 'Escape' && onCancel()}
>
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="peer-link-title"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === 'Escape' && onCancel()}
  >
    <h3 id="peer-link-title" class="title">{$tStore('project.peerLinkTitle' as any)}</h3>
    <p class="hint">{$tStore('project.peerLinkHint' as any)}</p>
    <p class="hint">{$tStore('project.peerLinkEqualityHint' as any)}</p>

    <label class="field">
      <span class="label">{$tStore('project.peerLinkSelf' as any)}</span>
      <select bind:value={selfId}>
        {#each myRepos as r (r.id)}
          <option value={r.id}>{getDisplayName(r)}{r.role ? ` · ${r.role}` : ''}</option>
        {/each}
      </select>
    </label>

    <div class="field">
      <span class="label">{$tStore('project.peerLinkPeer' as any)}</span>
      <input
        type="text"
        bind:value={filter}
        placeholder={$tStore('project.peerLinkSearch' as any)}
      />
      <div class="candidates">
        {#each candidates as r (r.id)}
          {@const linked = selfId !== null && linkedKeys.has(pairKey(selfId, r.id))}
          {@const reason = inequalityReason(selfRepo, r)}
          <button
            type="button"
            class="candidate"
            class:selected={peerId === r.id}
            class:linked
            disabled={linked || reason !== null}
            onclick={() => (peerId = r.id)}
          >
            <span class="cand-name">{getDisplayName(r)}</span>
            <span class="cand-meta">{r.role ?? '—'}</span>
            {#if linked}
              <span class="cand-flag">{$tStore('project.peerAlreadyLinked' as any)}</span>
            {:else if reason}
              <span class="cand-flag">{reason}</span>
            {/if}
          </button>
        {:else}
          <div class="no-candidates">{$tStore('project.peerNoCandidates' as any)}</div>
        {/each}
      </div>
    </div>

    <div class="actions">
      <button onclick={onCancel}>{$tStore('dialog.cancel')}</button>
      <button class="primary" disabled={selfId === null || peerId === null || alreadyLinked || selectedReason !== null} onclick={submit}>
        {$tStore('project.peerLinkAdd' as any)}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background-color: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10000;
  }

  .dialog {
    background-color: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 24px;
    width: 560px;
    max-width: calc(100vw - 48px);
    max-height: calc(100vh - 96px);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .title {
    margin: 0;
    font-size: 16px;
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
  }

  .label {
    font-size: 12px;
    color: var(--text-muted);
  }

  .candidates {
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow-y: auto;
    max-height: 260px;
    display: flex;
    flex-direction: column;
  }

  .candidate {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    background: none;
    border: none;
    border-bottom: 1px solid var(--border);
    text-align: left;
    cursor: pointer;
    color: inherit;
    font-size: 13px;
  }

  .candidate:last-child {
    border-bottom: none;
  }

  .candidate:hover:not(:disabled) {
    background-color: var(--surface-hover);
  }

  /* The selected row needs to read as chosen before the button is pressed:
     an accent bar plus accent text, not just a tinted background. Both this
     rule and the hover above previously named `--hover` / `--accent-soft`,
     which app.css does not define — an undefined CSS variable silently
     resolves to nothing, so neither state showed at all. */
  .candidate.selected {
    background-color: var(--surface-hover);
    box-shadow: inset 3px 0 0 var(--accent);
    color: var(--accent);
    font-weight: 500;
  }

  .candidate:disabled {
    cursor: default;
    opacity: 0.55;
  }

  .cand-name {
    flex: 1;
  }

  .cand-meta,
  .cand-flag {
    color: var(--text-muted);
    font-size: 12px;
  }

  .candidate.selected .cand-meta {
    color: var(--accent);
  }

  .no-candidates {
    padding: 14px;
    color: var(--text-muted);
    font-size: 12px;
    text-align: center;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
