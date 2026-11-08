<script lang="ts">
  // F-000044 / T-000223 (frames 1 and 3): the owner's "Общий режим" switch with
  // its enable dialog (preflight, public-repo warning, blocking .gitignore
  // lines) and the switch-off confirmation.
  import { onDestroy } from 'svelte';
  import { tStore } from '$lib/i18n';
  import { pat } from '$lib/stores/settings';
  import { githubLogin } from '$lib/stores/shared';
  import { verifyAdmin } from '$lib/sync-core';
  import { needsOwnAdminCheck } from '$lib/utils/shared-admin';
  import { addToast } from '$lib/stores/ui';
  import { sharedModePreflight, enableSharedMode, disableSharedMode, setRepoShareSpecs, getRepoSharedState } from '$lib/api/tauri-commands';
  import { isRepoPublic, splitRepoFullName } from '$lib/api/github';
  import type { IgnoreLine } from '$lib/types';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import SharedEnableDialog from './SharedEnableDialog.svelte';

  interface Props {
    repoId: number;
    repoName: string;
    githubName: string | null;
    /** The owner's branch: the repo's auto-commit branch. */
    devBranch: string;
    /** The owner's switch as stored. */
    checked: boolean;
    /** Preflight refused to enable: the texts to show under the chips (empty = clear). */
    onPreflightErrors: (messages: string[]) => void;
    /** Mode switched on or off: reload the state. */
    onChanged: () => void;
  }

  let { repoId, repoName, githubName, devBranch, checked, onPreflightErrors, onChanged }: Props = $props();

  // The card may be switched to another repo while a check is in flight: the
  // answer of the old one must not open a dialog or set errors on the new one.
  let alive = true;
  onDestroy(() => {
    alive = false;
  });

  let busy = $state(false);
  let showEnable = $state(false);
  let showDisable = $state(false);
  let blockingLines = $state<IgnoreLine[]>([]);
  let isPublic = $state<boolean | null>(null);

  // T-000243: the owner's "Делиться спеками и планами" checkbox. The flag is
  // read from the DB through `get_repo_shared_state` on purpose: the store
  // RepoDetail's switch reads (`list_repo_shared_states`) skips repos that
  // never enabled sharing, so for a preconfigured non-shared repo the store's
  // absence must never be guessed as "on". null = not loaded (or failed to):
  // the checkbox stays disabled, the value is never defaulted.
  let specs = $state<boolean | null>(null);
  let specsBusy = $state(false);

  async function loadSpecs() {
    try {
      const st = await getRepoSharedState(repoId);
      if (!alive) return;
      specs = st.share_specs;
    } catch {
      if (!alive) return;
      specs = null;
      addToast($tStore('shared.specs.loadFailed' as any), 'error');
    }
  }

  // On mount and on every flip of the shared-mode switch: enabling/disabling
  // refreshes the shared state around this chip, and the flag must not be
  // cached across that (mid-sync changes are re-read the same way).
  $effect(() => {
    void checked;
    void loadSpecs();
  });

  async function handleSpecsToggle() {
    if (specsBusy || specs === null) return;
    const next = !specs;
    specsBusy = true;
    try {
      await setRepoShareSpecs(repoId, next);
      // Only after the backend accepted it: a failed save must not look like
      // a successful switch-off.
      specs = next;
      onChanged();
    } catch (err) {
      addToast($tStore('shared.specs.saveFailed' as any).replace('{0}', String(err)), 'error');
    } finally {
      specsBusy = false;
    }
  }

  async function checkPublic(): Promise<boolean | null> {
    if (!$pat || !githubName || !githubName.includes('/')) return null;
    const { owner, repo } = splitRepoFullName(githubName);
    return isRepoPublic($pat, owner, repo);
  }

  async function handleToggle() {
    if (busy) return;
    if (checked) {
      showDisable = true;
      return;
    }
    busy = true;
    onPreflightErrors([]);
    try {
      // T-000227: the repo's owner may switch sharing on without asking GitHub;
      // anybody else (an organization's repo) must be one of its administrators.
      if (githubName && $githubLogin && needsOwnAdminCheck(githubName, $githubLogin)) {
        const admin = $pat ? (await verifyAdmin($pat, githubName, $githubLogin)).admin : false;
        if (!alive) return;
        if (!admin) {
          onPreflightErrors([$tStore('shared.preflight.notOwnerOrAdmin' as any)]);
          return;
        }
      }
      const pre = await sharedModePreflight(repoId);
      if (!alive) return;
      const problems: string[] = [];
      if (pre.missing_project) problems.push($tStore('shared.preflight.missingProject' as any));
      if (pre.missing_dev_branch) problems.push($tStore('shared.preflight.missingDevBranch' as any));
      if (problems.length > 0) {
        onPreflightErrors(problems);
        return;
      }
      blockingLines = pre.blocking_lines;
      isPublic = await checkPublic();
      if (!alive) return;
      showEnable = true;
    } catch (err) {
      if (alive) addToast($tStore('shared.preflight.failed' as any).replace('{0}', String(err)), 'error');
    } finally {
      busy = false;
    }
  }

  async function confirmEnable() {
    if (busy) return;
    busy = true;
    try {
      await enableSharedMode(repoId, blockingLines.length > 0);
      showEnable = false;
      addToast($tStore('shared.enable.done' as any), 'success');
      onChanged();
    } catch (err) {
      addToast($tStore('shared.enable.failed' as any).replace('{0}', String(err)), 'error');
    } finally {
      busy = false;
    }
  }

  async function confirmDisable() {
    showDisable = false;
    if (busy) return;
    busy = true;
    try {
      const res = await disableSharedMode(repoId);
      addToast($tStore('shared.disable.done' as any), 'success');
      if (res.note) addToast(res.note, 'warning');
      onChanged();
    } catch (err) {
      addToast($tStore('shared.disable.failed' as any).replace('{0}', String(err)), 'error');
    } finally {
      busy = false;
    }
  }
</script>

<div class="chip" class:on={checked} title={$tStore('shared.chipHint' as any)}>
  <span class="chip-label">{$tStore('shared.chipLabel' as any)}:</span>
  <button
    type="button"
    class="switch"
    class:on={checked}
    role="switch"
    aria-checked={checked}
    aria-label={$tStore('shared.chipLabel' as any)}
    disabled={busy}
    onclick={handleToggle}
  >
    <span class="knob"></span>
  </button>
</div>

<!-- T-000243: the owner's specs switch, same chip language. Rendered inside
     this component (under RepoDetail's showSharedToggle) so it is owner/main
     only — also before shared mode is enabled, for preconfiguration. -->
<label class="chip" class:on={specs === true} title={$tStore('shared.specs.hint' as any)}>
  <input
    class="specs-check"
    type="checkbox"
    checked={specs === true}
    disabled={specs === null || specsBusy}
    onchange={handleSpecsToggle}
  />
  <span class="chip-label">{$tStore('shared.specs.label' as any)}</span>
</label>

{#if showEnable}
  <SharedEnableDialog
    {repoName}
    {devBranch}
    {isPublic}
    {blockingLines}
    {busy}
    onConfirm={confirmEnable}
    onCancel={() => (showEnable = false)}
  />
{/if}

{#if showDisable}
  <ConfirmDialog
    title={$tStore('shared.disable.title' as any)}
    message={$tStore('shared.disable.message' as any)}
    onConfirm={confirmDisable}
    onCancel={() => (showDisable = false)}
  />
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 10px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    transition: border-color 150ms, background 150ms;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .chip:hover {
    border-color: var(--accent);
    background: var(--surface-hover);
  }
  /* Frame 1: on -> green border. */
  .chip.on,
  .chip.on:hover {
    border-color: var(--toast-success);
  }
  .chip-label {
    color: var(--text-muted);
    font-size: 11px;
    white-space: nowrap;
  }

  .switch {
    position: relative;
    width: 28px;
    height: 16px;
    padding: 0;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--border);
    cursor: pointer;
    transition: background 150ms, border-color 150ms;
    flex-shrink: 0;
  }
  .switch.on {
    background: var(--toast-success);
    border-color: var(--toast-success);
  }
  .switch:disabled {
    opacity: 0.5;
    cursor: wait;
  }
  .knob {
    position: absolute;
    top: 1px;
    left: 1px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text);
    transition: transform 150ms;
  }
  .switch.on .knob {
    transform: translateX(12px);
    background: white;
  }

  /* T-000243: the specs checkbox — the same chip language, a native checkbox
     instead of the switch. The label wraps the input, so clicking the text
     toggles it too (and does nothing while it is disabled). */
  .specs-check {
    margin: 0;
    width: 13px;
    height: 13px;
    accent-color: var(--toast-success);
    cursor: pointer;
    flex-shrink: 0;
  }
  .specs-check:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
