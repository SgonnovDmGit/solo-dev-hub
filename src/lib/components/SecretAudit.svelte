<script lang="ts">
  import { onMount } from 'svelte';
  import { tStore, tf } from '$lib/i18n';
  import { pat } from '$lib/stores/settings';
  import { allRepos } from '$lib/stores/repos';
  import { addToast } from '$lib/stores/ui';
  import { listSecretPushEvents, listDeployEnvironments } from '$lib/api/tauri-commands';
  import { splitRepoFullName, listRepoSecrets, listEnvironmentSecrets } from '$lib/api/github';
  import type { SecretPushEvent } from '$lib/types';

  // The push journal (DB-only, no token needed). Loaded once in onMount.
  // Cap of 1000 is generous so the reconcile fold below sees the full history
  // for a repo, not just a recent page (reconcile relies on latest-wins across
  // all logged events for the selected repo).
  let events = $state<SecretPushEvent[]>([]);
  let loadingEvents = $state(true);

  // Repo chosen for reconcile.
  let selectedRepoId = $state<number | null>(null);
  let reconciling = $state(false);

  // Only GitHub-linked repos can be reconciled against live secrets.
  const reconcilableRepos = $derived($allRepos.filter((r) => r.github_name));
  const noToken = $derived(!$pat);

  // Three-bucket comparison of logged-expected vs live for one scope.
  interface Buckets {
    inBoth: string[];
    loggedMissing: string[];
    unlogged: string[];
  }
  interface ScopeResult {
    key: string;
    isEnv: boolean;
    envName: string | null;
    buckets: Buckets;
  }
  // Reconcile output: one entry for the repo scope + one per deploy env.
  let reconcileResult = $state<ScopeResult[] | null>(null);

  function fmtDateTime(ts: string): string {
    const d = new Date(ts);
    if (isNaN(d.getTime())) return ts;
    const pad = (n: number) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }

  // Latest-wins fold: given a set of events, keep the newest by ts per
  // secret_name, then return the names whose latest action === 'set' (i.e.
  // expected to be present in GitHub). A name whose latest action is 'delete'
  // is NOT expected present.
  function expectedPresent(evs: SecretPushEvent[]): Set<string> {
    const latest = new Map<string, SecretPushEvent>();
    for (const ev of evs) {
      const cur = latest.get(ev.secret_name);
      if (!cur || ev.ts > cur.ts) latest.set(ev.secret_name, ev);
    }
    const present = new Set<string>();
    for (const [name, ev] of latest) {
      if (ev.action === 'set') present.add(name);
    }
    return present;
  }

  // Compute the three buckets for one scope from the logged-expected set and
  // the live set of names.
  function buildBuckets(loggedExpected: Set<string>, live: Set<string>): Buckets {
    const inBoth: string[] = [];
    const loggedMissing: string[] = [];
    const unlogged: string[] = [];
    for (const name of loggedExpected) {
      if (live.has(name)) inBoth.push(name);
      else loggedMissing.push(name);
    }
    for (const name of live) {
      if (!loggedExpected.has(name)) unlogged.push(name);
    }
    inBoth.sort();
    loggedMissing.sort();
    unlogged.sort();
    return { inBoth, loggedMissing, unlogged };
  }

  async function runReconcile() {
    if (selectedRepoId === null || noToken) return;
    const repo = $allRepos.find((r) => r.id === selectedRepoId);
    if (!repo?.github_name || !$pat) return;
    const or = splitRepoFullName(repo.github_name);
    const token = $pat;
    const repoId = selectedRepoId;

    reconciling = true;
    try {
      const scopes: ScopeResult[] = [];

      // --- Repo scope ---
      const repoEvents = events.filter(
        (ev) => ev.source === 'repo' && ev.repository_id === repoId,
      );
      const liveRepo = new Set((await listRepoSecrets(token, or.owner, or.repo)).map((s) => s.name));
      scopes.push({
        key: 'repo',
        isEnv: false,
        envName: null,
        buckets: buildBuckets(expectedPresent(repoEvents), liveRepo),
      });

      // --- Env scopes ---
      const envs = await listDeployEnvironments(repoId);
      for (const env of envs) {
        const envEvents = events.filter(
          (ev) =>
            ev.source === 'env' &&
            ev.repository_id === repoId &&
            ev.deploy_env_id === env.id,
        );
        let liveEnv: Set<string>;
        try {
          liveEnv = new Set(
            (await listEnvironmentSecrets(token, or.owner, or.repo, env.name)).map((s) => s.name),
          );
        } catch {
          // Env may not exist on GitHub yet — treat live set as empty so
          // logged pushes surface as loggedMissing rather than aborting.
          liveEnv = new Set();
        }
        scopes.push({
          key: `env-${env.id}`,
          isEnv: true,
          envName: env.name,
          buckets: buildBuckets(expectedPresent(envEvents), liveEnv),
        });
      }

      reconcileResult = scopes;
    } catch (e) {
      reconcileResult = null;
      addToast(tf('secretAudit.reconcileError', String(e)), 'error');
    } finally {
      reconciling = false;
    }
  }

  function onRepoChange() {
    reconcileResult = null;
    if (selectedRepoId !== null) runReconcile();
  }

  onMount(async () => {
    try {
      events = await listSecretPushEvents(1000, 0);
    } catch (e) {
      addToast(String(e), 'error');
    } finally {
      loadingEvents = false;
    }
  });
</script>

<div class="wrap">
  <div class="head">
    <h1>{$tStore('secretAudit.title' as any)}</h1>
    <span class="sub">{$tStore('secretAudit.subtitle' as any)}</span>
  </div>

  <!-- Journal (always visible, DB-only) -->
  <section class="panel">
    <div class="panel-head">
      <div class="dot"></div>
      <h2>{$tStore('secretAudit.journalTitle' as any)}</h2>
    </div>
    {#if loadingEvents}
      <p class="state">{$tStore('common.loading' as any)}</p>
    {:else if events.length === 0}
      <p class="state">{$tStore('secretAudit.emptyJournal' as any)}</p>
    {:else}
      <table>
        <thead>
          <tr>
            <th>{$tStore('secretAudit.colTime' as any)}</th>
            <th>{$tStore('secretAudit.colTarget' as any)}</th>
            <th>{$tStore('secretAudit.colSecret' as any)}</th>
            <th>{$tStore('secretAudit.colAction' as any)}</th>
          </tr>
        </thead>
        <tbody>
          {#each events as ev, i (ev.source + '-' + ev.repository_id + '-' + (ev.deploy_env_id ?? '') + '-' + ev.secret_name + '-' + ev.ts + '-' + i)}
            <tr>
              <td class="mono muted">{fmtDateTime(ev.ts)}</td>
              <td class="target">
                {ev.repo_name}{#if ev.source === 'env' && ev.env_name}<span class="arrow"> → {ev.env_name}</span>{/if}
              </td>
              <td class="mono">{ev.secret_name}</td>
              <td>
                <span class="badge {ev.action}">
                  {ev.action === 'set'
                    ? $tStore('secretAudit.actionSet' as any)
                    : $tStore('secretAudit.actionDelete' as any)}
                </span>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <!-- Reconcile -->
  <section class="panel">
    <div class="panel-head">
      <div class="dot"></div>
      <h2>{$tStore('secretAudit.reconcileTitle' as any)}</h2>
    </div>

    {#if noToken}
      <div class="banner">{$tStore('secretAudit.noToken' as any)}</div>
    {/if}

    <div class="controls">
      <select
        class="control"
        bind:value={selectedRepoId}
        onchange={onRepoChange}
        disabled={noToken || reconciling}
      >
        <option value={null} disabled>{$tStore('secretAudit.selectRepo' as any)}</option>
        {#each reconcilableRepos as r (r.id)}
          <option value={r.id}>{r.github_name}</option>
        {/each}
      </select>
      <button
        class="refresh-btn"
        type="button"
        onclick={runReconcile}
        disabled={noToken || reconciling || selectedRepoId === null}
        title={$tStore('secretAudit.refresh' as any)}
      >
        {reconciling ? '⟳' : '↻'} {$tStore('secretAudit.refresh' as any)}
      </button>
    </div>

    <p class="lag-note">{$tStore('secretAudit.lagNote' as any)}</p>

    {#if reconciling}
      <p class="state">{$tStore('secretAudit.reconciling' as any)}</p>
    {:else if reconcileResult}
      {#each reconcileResult as scope (scope.key)}
        <div class="scope">
          <h3 class="scope-title">
            {scope.isEnv
              ? tf('secretAudit.envScope', scope.envName ?? '')
              : $tStore('secretAudit.repoScope' as any)}
          </h3>
          <div class="buckets">
            <div class="bucket both">
              <div class="bucket-head">
                {$tStore('secretAudit.bucketInBoth' as any)}
                <span class="count">{scope.buckets.inBoth.length}</span>
              </div>
              <div class="chips">
                {#each scope.buckets.inBoth as name (name)}
                  <span class="chip">{name}</span>
                {:else}
                  <span class="none">—</span>
                {/each}
              </div>
            </div>
            <div class="bucket missing">
              <div class="bucket-head">
                {$tStore('secretAudit.bucketLoggedMissing' as any)}
                <span class="count">{scope.buckets.loggedMissing.length}</span>
              </div>
              <div class="chips">
                {#each scope.buckets.loggedMissing as name (name)}
                  <span class="chip warn">{name}</span>
                {:else}
                  <span class="none">—</span>
                {/each}
              </div>
            </div>
            <div class="bucket unlogged">
              <div class="bucket-head">
                {$tStore('secretAudit.bucketUnlogged' as any)}
                <span class="count">{scope.buckets.unlogged.length}</span>
              </div>
              <div class="chips">
                {#each scope.buckets.unlogged as name (name)}
                  <span class="chip">{name}</span>
                {:else}
                  <span class="none">—</span>
                {/each}
              </div>
            </div>
          </div>
        </div>
      {/each}
    {/if}
  </section>
</div>

<style>
  .wrap {
    height: 100%;
    overflow-y: auto;
    box-sizing: border-box;
    padding: 18px 24px 60px;
  }

  .head { display: flex; align-items: baseline; gap: 12px; margin-bottom: 14px; }
  .head h1 { font-size: 19px; margin: 0; font-weight: 650; }
  .head .sub { color: var(--text-muted); font-size: 12px; }

  .panel {
    margin-bottom: 22px;
    border: 1px solid var(--border);
    border-radius: 9px;
    overflow: hidden;
    background: var(--surface);
  }
  .panel-head {
    display: flex; align-items: center; gap: 10px; padding: 10px 14px;
    background: rgba(0, 0, 0, 0.12); border-bottom: 1px solid var(--border);
  }
  .panel-head .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--accent, #7c3aed); }
  .panel-head h2 { font-size: 14px; margin: 0; font-weight: 650; }

  .state { color: var(--text-muted); font-size: 13px; padding: 18px 14px; margin: 0; }

  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  thead th {
    text-align: left; font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-muted); font-weight: 600; padding: 7px 12px; border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  thead th:first-child, tbody td:first-child { padding-left: 14px; }
  tbody td {
    padding: 8px 12px; border-bottom: 1px solid var(--border); vertical-align: middle;
    font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
  }
  tbody tr:last-child td { border-bottom: none; }
  .mono { font-family: "SF Mono", "Cascadia Code", Consolas, monospace; font-size: 11.5px; }
  .muted { color: var(--text-muted); }
  .target { font-weight: 600; color: var(--text); }
  .arrow { font-weight: 500; color: var(--text-muted); }

  .badge {
    display: inline-block; padding: 2px 9px; border-radius: 11px;
    font-size: 11px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.03em;
  }
  .badge.set { color: #16a34a; background: rgba(22, 163, 74, 0.15); }
  .badge.delete { color: #dc2626; background: rgba(220, 38, 38, 0.15); }

  .banner {
    margin: 12px 14px 0;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent, #7c3aed);
    border-radius: 5px;
    background: rgba(124, 58, 237, 0.08);
    color: var(--text);
    font-size: 12px;
  }

  .controls { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; padding: 14px; }
  .control {
    background: var(--bg); border: 1px solid var(--border); color: var(--text);
    border-radius: 5px; padding: 6px 10px; font-size: 12px; min-width: 220px;
  }
  .control:disabled { opacity: 0.5; cursor: not-allowed; }
  .refresh-btn {
    background: var(--surface); border: 1px solid var(--border); color: var(--text);
    border-radius: 5px; padding: 6px 12px; font-size: 12px; cursor: pointer; white-space: nowrap;
  }
  .refresh-btn:hover:not(:disabled) { border-color: var(--accent, #7c3aed); background: rgba(124, 58, 237, 0.12); }
  .refresh-btn:disabled { opacity: 0.4; cursor: not-allowed; }

  .lag-note { color: var(--text-muted); font-size: 11px; padding: 0 14px; margin: 0 0 6px; }

  .scope { padding: 6px 14px 14px; border-top: 1px solid var(--border); }
  .scope-title { font-size: 13px; font-weight: 650; margin: 12px 0 10px; }

  .buckets { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; }
  .bucket { border: 1px solid var(--border); border-radius: 7px; padding: 10px; background: var(--bg); }
  .bucket-head {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.04em;
    color: var(--text-muted); font-weight: 600; margin-bottom: 8px;
  }
  .bucket.missing .bucket-head { color: #dc2626; }
  .bucket-head .count {
    display: inline-block; min-width: 20px; text-align: center;
    background: rgba(0, 0, 0, 0.15); border: 1px solid var(--border); border-radius: 10px;
    padding: 0 6px; font-size: 11px; font-variant-numeric: tabular-nums;
  }

  .chips { display: flex; flex-wrap: wrap; gap: 5px; }
  .chip {
    display: inline-block; font-family: "SF Mono", "Cascadia Code", Consolas, monospace;
    font-size: 11px; padding: 2px 8px; border-radius: 10px;
    background: var(--surface-hover); border: 1px solid var(--border); color: var(--text);
  }
  .chip.warn { border-color: #dc2626; color: #dc2626; background: rgba(220, 38, 38, 0.08); }
  .none { color: var(--text-muted); font-size: 12px; }
</style>
