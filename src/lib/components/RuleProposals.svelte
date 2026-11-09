<script lang="ts">
  // T-000241: the proposal channel of one rule — the owner's decision queue and
  // the contributor's sent/withdraw surface, per shared repo.
  import { tStore, tf } from '$lib/i18n';
  import type { RuleInfo, RuleProposalView, RuleAcceptance, RepoSharedState } from '$lib/types';
  import {
    listRuleProposals,
    listRuleAcceptances,
    decideRuleProposal,
    withdrawRuleProposal,
    resolveRuleAcceptanceConflict,
  } from '$lib/api/tauri-commands';
  import { addToast } from '$lib/stores/ui';
  import { sharedStates, githubLogin } from '$lib/stores/shared';
  import { isSharedRepo } from '$lib/utils/shared-mode';
  import { buildRuleDiff } from '$lib/utils/rule-diff';
  import { formatRuleDate } from '$lib/utils/rules';
  import RuleDiff from './RuleDiff.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';

  let {
    rule,
    /** Bumped by the parent after a sync — the data may have moved. */
    generation,
    onChanged,
  }: {
    rule: RuleInfo;
    generation: number;
    /** The rule text may have changed (an accepted proposal applied) — parent reloads. */
    onChanged: () => void;
  } = $props();

  interface RepoBlock {
    repo: RepoSharedState;
    proposals: RuleProposalView[];
    acceptances: RuleAcceptance[];
  }

  let blocks = $state<RepoBlock[]>([]);
  let loading = $state(false);
  let busy = $state(false);
  let openDiff = $state<string | null>(null);
  let confirmDecision = $state<{
    repoId: number;
    rule: string;
    login: string;
    rev: string;
    action: 'accept' | 'reject';
  } | null>(null);
  // The last generation already loaded — a parent re-bump re-reads.
  let loadedFor = $state(-1);

  const sharedRepos = $derived(
    [...$sharedStates.values()].filter((st) => isSharedRepo(st) && (st.shared_role === 'main' || st.shared_role === 'contributor'))
  );
  const login = $derived($githubLogin);

  async function load() {
    loading = true;
    try {
      const out = await Promise.all(
        sharedRepos.map(async (repo) => {
          const [proposals, acceptances] = await Promise.all([
            listRuleProposals(repo.repo_id).catch(() => [] as RuleProposalView[]),
            repo.shared_role === 'main'
              ? listRuleAcceptances(repo.repo_id).catch(() => [] as RuleAcceptance[])
              : Promise.resolve([] as RuleAcceptance[]),
          ]);
          return { repo, proposals: proposals.filter((p) => p.rule === rule.rule), acceptances };
        })
      );
      blocks = out.filter((b) => b.proposals.length > 0 || b.acceptances.length > 0);
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (generation !== loadedFor) {
      loadedFor = generation;
      void load();
    }
  });

  /** The diff runs against the install's own canon — the saved rule text. For a
   *  contributor that is the adopted owner's text, never the bundled canon. */
  function canonFor(): string {
    return rule.content;
  }

  function askDecision(repoId: number, p: RuleProposalView, action: 'accept' | 'reject') {
    if (!p.rev) return;
    confirmDecision = { repoId, rule: p.rule, login: p.login, rev: p.rev, action };
  }

  async function runDecision() {
    const d = confirmDecision;
    if (!d || busy) return;
    busy = true;
    confirmDecision = null;
    try {
      await decideRuleProposal(d.repoId, d.rule, d.login, d.rev, d.action);
      await load();
      onChanged();
    } catch (err) {
      addToast(String(err), 'error');
      await load();
    } finally {
      busy = false;
    }
  }

  async function withdraw(repoId: number, p: RuleProposalView) {
    if (busy) return;
    busy = true;
    try {
      await withdrawRuleProposal(repoId, p.file_name);
      await load();
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      busy = false;
    }
  }

  async function resolveConflict(repoId: number, a: RuleAcceptance, resolution: 'apply' | 'dismiss') {
    if (busy) return;
    busy = true;
    try {
      await resolveRuleAcceptanceConflict(repoId, a.rule, a.proposer_login, a.decided_rev, resolution);
      await load();
      onChanged();
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      busy = false;
    }
  }
</script>

<div class="proposals">
  {#if loading && blocks.length === 0}
    <div class="empty">{$tStore('common.loading')}</div>
  {:else if blocks.length === 0}
    <div class="empty">{$tStore('rules.proposals.empty')}</div>
  {:else}
    {#each blocks as block (block.repo.repo_id)}
      {@const owner = block.repo.shared_role === 'main'}
      {#each block.proposals as p (p.file_name)}
        <div class="proposal" class:withdrawn={p.status === 'withdrawn'}>
          <div class="prow">
            <span class="who">{tf('rules.proposals.from', p.login, formatRuleDate(p.at))}</span>
            <span class="meta">{tf('rules.proposals.seq', p.seq)}</span>
            {#if p.status === 'withdrawn'}
              <span class="chip muted">{$tStore('rules.proposals.withdrawn')}</span>
            {:else if p.resolved_action === 'accept'}
              <span class="chip accepted">{tf('rules.proposals.acceptedBy', p.resolved_by ?? '')}</span>
            {:else if p.resolved_action === 'reject'}
              <span class="chip rejected">{tf('rules.proposals.rejectedBy', p.resolved_by ?? '')}</span>
            {:else if p.acceptance_status}
              <span class="chip queued">{$tStore('rules.proposals.queued')}</span>
            {:else}
              <span class="chip open">{$tStore('rules.proposals.open')}</span>
            {/if}
            <span class="flags">
              {#if p.on_origin}
                <span class="flag on">{$tStore('rules.proposals.onOrigin')}</span>
              {:else if p.on_dev}
                <span class="flag on">{$tStore('rules.proposals.sent')}</span>
              {:else}
                <span class="flag">{$tStore('rules.proposals.notSent')}</span>
              {/if}
            </span>
            <span class="grow"></span>
            <button type="button" class="ghost diff-btn" onclick={() => (openDiff = openDiff === p.file_name ? null : p.file_name)}>
              {openDiff === p.file_name ? $tStore('rules.proposals.hideDiff') : $tStore('rules.proposals.showDiff')}
            </button>
            {#if owner && p.status === 'open' && p.resolved_action === null && p.acceptance_status === null && p.rev && p.on_dev}
              <button type="button" class="primary act" disabled={busy} onclick={() => askDecision(block.repo.repo_id, p, 'accept')}>
                {$tStore('rules.proposals.accept')}
              </button>
              <button type="button" class="ghost act" disabled={busy} onclick={() => askDecision(block.repo.repo_id, p, 'reject')}>
                {$tStore('rules.proposals.reject')}
              </button>
            {/if}
            {#if !owner && p.status === 'open' && p.resolved_action === null && p.login === login}
              <button type="button" class="ghost act" disabled={busy} onclick={() => withdraw(block.repo.repo_id, p)}>
                {$tStore('rules.proposals.withdraw')}
              </button>
            {/if}
          </div>
          {#if openDiff === p.file_name}
            <RuleDiff diff={buildRuleDiff(canonFor(), p.text)} legend emptyText={$tStore('rules.diffEmpty')} />
          {/if}
        </div>
      {/each}
      {#each block.acceptances as a (a.rule + a.proposer_login + a.decided_rev)}
        {#if a.status === 'conflicted'}
          <div class="proposal conflicted">
            <div class="prow">
              <span class="who">{tf('rules.proposals.from', a.proposer_login, formatRuleDate(a.updated_at))}</span>
              <span class="chip conflict">{$tStore('rules.proposals.conflicted')}</span>
              <span class="grow"></span>
              <button type="button" class="primary act" disabled={busy} onclick={() => resolveConflict(block.repo.repo_id, a, 'apply')}>
                {$tStore('rules.proposals.conflictApply')}
              </button>
              <button type="button" class="ghost act" disabled={busy} onclick={() => resolveConflict(block.repo.repo_id, a, 'dismiss')}>
                {$tStore('rules.proposals.conflictDismiss')}
              </button>
            </div>
          </div>
        {:else if a.pub_state === 'blocked'}
          <div class="proposal">
            <div class="prow">
              <span class="who">{tf('rules.proposals.from', a.proposer_login, formatRuleDate(a.updated_at))}</span>
              <span class="chip conflict">{$tStore('rules.proposals.blocked')}</span>
            </div>
          </div>
        {:else if a.pub_state === 'pending' && a.status !== 'prepared'}
          <div class="proposal">
            <div class="prow">
              <span class="who">{tf('rules.proposals.from', a.proposer_login, formatRuleDate(a.updated_at))}</span>
              <span class="chip queued">{$tStore('rules.proposals.queued')}</span>
            </div>
          </div>
        {:else if a.pub_state === 'overtaken'}
          <div class="proposal">
            <div class="prow">
              <span class="who">{tf('rules.proposals.from', a.proposer_login, formatRuleDate(a.updated_at))}</span>
              <span class="chip muted">{$tStore('rules.proposals.overtaken')}</span>
            </div>
          </div>
        {/if}
      {/each}
    {/each}
  {/if}
</div>

{#if confirmDecision}
  <ConfirmDialog
    title={confirmDecision.action === 'accept' ? $tStore('rules.proposals.accept') : $tStore('rules.proposals.reject')}
    message={tf('rules.proposals.from', confirmDecision.login, confirmDecision.rule)}
    onConfirm={runDecision}
    onCancel={() => (confirmDecision = null)}
  />
{/if}

<style>
  .proposals { display: flex; flex-direction: column; gap: 8px; }
  .empty { font-size: 12px; color: var(--text-muted); padding: 12px 0; }
  .proposal { border: 1px solid var(--border); border-radius: var(--radius); background: var(--surface); padding: 8px 10px; display: flex; flex-direction: column; gap: 6px; }
  .proposal.withdrawn { opacity: 0.65; }
  .proposal.conflicted { border-color: var(--danger); }
  .prow { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .who { font-size: 12px; font-weight: 600; }
  .meta { font-size: 11px; color: var(--text-muted); }
  .chip { font-size: 9px; padding: 1px 6px; border-radius: 6px; white-space: nowrap; }
  .chip.open { background: rgba(234, 179, 8, 0.15); color: rgb(234, 179, 8); }
  .chip.accepted { background: rgba(74, 222, 128, 0.15); color: #4ade80; }
  .chip.rejected { background: rgba(239, 68, 68, 0.15); color: var(--danger); }
  .chip.queued { border: 1px solid var(--accent); color: var(--accent); padding: 0 6px; }
  .chip.conflict { background: rgba(239, 68, 68, 0.15); color: var(--danger); }
  .chip.muted { border: 1px solid var(--border); color: var(--text-muted); padding: 0 6px; }
  .flags { display: flex; gap: 6px; }
  .flag { font-size: 10px; color: var(--text-muted); }
  .flag.on { color: #4ade80; }
  .grow { flex: 1; }
  .diff-btn { font-size: 11px; padding: 2px 8px; }
  .act { font-size: 11px; padding: 3px 10px; }
</style>
