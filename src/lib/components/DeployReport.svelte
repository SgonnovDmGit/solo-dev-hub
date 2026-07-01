<script lang="ts">
  import { onMount } from 'svelte';
  import { tStore } from '$lib/i18n';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { listDeployReport } from '$lib/api/tauri-commands';
  import { projects } from '$lib/stores/projects';
  import { selectedRepoId, deployDrillTarget, navigateTo, addToast } from '$lib/stores/ui';
  import type { DeployReportRow } from '$lib/types';

  let rows = $state<DeployReportRow[]>([]);
  let loading = $state(true);

  // Filters (ephemeral — no persisted store per spec decision #2).
  let projectFilter = $state<number | 'all'>('all');
  let envFilter = $state<string>('all');
  let search = $state('');

  onMount(async () => {
    try {
      rows = await listDeployReport();
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      loading = false;
    }
  });

  // Distinct env names present, for the env filter dropdown.
  const envOptions = $derived([...new Set(rows.map((r) => r.env_name))].sort());

  const filtered = $derived(
    rows.filter((r) => {
      if (projectFilter !== 'all' && r.project_id !== projectFilter) return false;
      if (envFilter !== 'all' && r.env_name !== envFilter) return false;
      const q = search.trim().toLowerCase();
      if (q) {
        const hay = `${r.repo_name} ${r.domain} ${r.deploy_branch} ${r.image_tag} ${r.env_name}`.toLowerCase();
        if (!hay.includes(q)) return false;
      }
      return true;
    }),
  );

  interface Group {
    projectId: number | null;
    projectName: string | null;
    rows: DeployReportRow[];
  }

  // Group filtered rows by project, preserving query order (already sorted:
  // named projects first by name, orphans last).
  const groups = $derived.by(() => {
    const map = new Map<number | null, Group>();
    const order: (number | null)[] = [];
    for (const r of filtered) {
      const key = r.project_id;
      if (!map.has(key)) {
        map.set(key, { projectId: key, projectName: r.project_name, rows: [] });
        order.push(key);
      }
      map.get(key)!.rows.push(r);
    }
    return order.map((k) => map.get(k)!);
  });

  const projectCount = $derived(new Set(filtered.map((r) => r.project_id)).size);

  function envClass(env: string): string {
    const e = env.toLowerCase();
    if (e.includes('prod')) return 'prod';
    if (e.includes('test')) return 'test';
    if (e.includes('stag') || e === 'stg') return 'stg';
    return 'cust';
  }

  // Absolute DD.MM.YYYY (per design — config-changed date, not deploy time).
  function fmtDate(ts: string): string {
    const p = ts.slice(0, 10).split('-');
    return p.length === 3 ? `${p[2]}.${p[1]}.${p[0]}` : ts;
  }

  function repoCountOf(g: Group): number {
    return new Set(g.rows.map((r) => r.repository_id)).size;
  }

  // Drill-down: select the repo + arm the one-shot signal, navigate to the
  // repo's Deploy tab (RepoDetail + DeployScreen consume deployDrillTarget).
  function drillTo(row: DeployReportRow) {
    selectedRepoId.set(row.repository_id);
    deployDrillTarget.set({ repoId: row.repository_id, deployEnvId: row.deploy_env_id });
    navigateTo('repo-detail');
  }

  // v1.6.0: the report shows only the database NAME (a DB-classified field
  // ending in _NAME, or DATABASE / PGDATABASE) — display-only, in the main row.
  // Host/user and SSH are still returned by the backend for future filters but
  // not shown here. Github-only names (no local value) are simply omitted.
  function dbNameOf(r: DeployReportRow): string {
    return r.db_fields
      .filter((f) => {
        const u = f.name.toUpperCase();
        return u.endsWith('_NAME') || u === 'DATABASE' || u === 'PGDATABASE';
      })
      .map((f) => f.value)
      .filter((v): v is string => !!v)
      .join(', ');
  }

  async function openDomain(e: MouseEvent, domain: string) {
    e.preventDefault(); // don't let the webview navigate to the href
    e.stopPropagation(); // don't also trigger the row drill
    if (!domain) return;
    const url = domain.startsWith('http') ? domain : `https://${domain}`;
    try {
      await openUrl(url);
    } catch (err) {
      addToast(String(err), 'error');
    }
  }
</script>

<div class="wrap">
  <div class="head">
    <h1>{$tStore('deploy.report.title' as any)}</h1>
    <span class="sub">{$tStore('deploy.report.subtitle' as any)}</span>
  </div>

  {#if !loading && rows.length > 0}
    <div class="filters">
      <div class="fblock">
        <span class="flabel">{$tStore('deploy.report.filterProject' as any)}</span>
        <select class="control" bind:value={projectFilter}>
          <option value="all">{$tStore('deploy.report.allProjects' as any)}</option>
          {#each $projects as p (p.id)}
            <option value={p.id}>{p.name}</option>
          {/each}
        </select>
      </div>
      <div class="fblock">
        <span class="flabel">{$tStore('deploy.report.filterEnv' as any)}</span>
        <select class="control" bind:value={envFilter}>
          <option value="all">{$tStore('deploy.report.allEnvs' as any)}</option>
          {#each envOptions as e (e)}
            <option value={e}>{e}</option>
          {/each}
        </select>
      </div>
      <div class="fblock">
        <span class="flabel">{$tStore('deploy.report.search' as any)}</span>
        <input
          class="control search"
          bind:value={search}
          placeholder={$tStore('deploy.report.searchPlaceholder' as any)}
        />
      </div>
      <div class="spacer"></div>
      <div class="count">
        {$tStore('deploy.report.summaryFmt' as any)
          .replace('{0}', String(filtered.length))
          .replace('{1}', String(projectCount))}
      </div>
    </div>
  {/if}

  {#if loading}
    <p class="state">{$tStore('deploy.report.loading' as any)}</p>
  {:else if rows.length === 0}
    <p class="state">{$tStore('deploy.report.empty' as any)}</p>
  {:else if filtered.length === 0}
    <p class="state">{$tStore('deploy.report.noMatch' as any)}</p>
  {:else}
    {#each groups as g (g.projectId ?? -1)}
      <div class="section">
        <div class="sec-head">
          <span class="dot"></span>
          <h2>{g.projectName ?? $tStore('deploy.report.noProject' as any)}</h2>
          <span class="meta">
            {$tStore('deploy.report.sectionMetaFmt' as any)
              .replace('{0}', String(g.rows.length))
              .replace('{1}', String(repoCountOf(g)))}
          </span>
        </div>
        <table>
          <!-- Shared fixed widths so columns line up across every project
               section (each section is its own table — without this they
               auto-size per-section and drift). -->
          <colgroup>
            <col style="width: 20%" />
            <col style="width: 8%" />
            <col style="width: 18%" />
            <col style="width: 8%" />
            <col style="width: 11%" />
            <col style="width: 12%" />
            <col style="width: 7%" />
            <col style="width: 12%" />
            <col style="width: 4%" />
          </colgroup>
          <thead>
            <tr>
              <th>{$tStore('deploy.report.colRepo' as any)}</th>
              <th>{$tStore('deploy.report.colEnv' as any)}</th>
              <th>{$tStore('deploy.report.colDomain' as any)}</th>
              <th>{$tStore('deploy.report.colBranch' as any)}</th>
              <th>{$tStore('deploy.report.colImageTag' as any)}</th>
              <th>{$tStore('deploy.report.dbColumn' as any)}</th>
              <th class="num">{$tStore('deploy.report.colSecrets' as any)}</th>
              <th>{$tStore('deploy.report.colUpdated' as any)}</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each g.rows as r (r.deploy_env_id)}
              <tr class="main" onclick={() => drillTo(r)}>
                <td class="repo">{r.repo_name}</td>
                <td><span class="env {envClass(r.env_name)}">{r.env_name}</span></td>
                <td>
                  {#if r.domain}
                    <a class="domain" href={'https://' + r.domain} onclick={(e) => openDomain(e, r.domain)}>{r.domain}</a>
                  {:else}
                    <span class="domain none">{$tStore('deploy.report.internalDomain' as any)}</span>
                  {/if}
                </td>
                <td class="mono">{r.deploy_branch}</td>
                <td class="mono">{r.image_tag}</td>
                <td class="mono db-name">{#if dbNameOf(r)}{dbNameOf(r)}{:else}<span class="muted">—</span>{/if}</td>
                <td class="num"><span class="sbadge">{r.secrets_count}</span></td>
                <td class="muted">{fmtDate(r.updated_at)}</td>
                <td class="num"><span class="drill">→</span></td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/each}
  {/if}
</div>

<style>
  .wrap {
    height: 100%;
    overflow-y: auto;
    box-sizing: border-box;
    padding: 18px 24px 60px;
  }

  .head { display: flex; align-items: baseline; gap: 12px; margin-bottom: 4px; }
  .head h1 { font-size: 19px; margin: 0; font-weight: 650; }
  .head .sub { color: var(--text-muted); font-size: 12px; }

  .filters { display: flex; gap: 10px; align-items: flex-end; flex-wrap: wrap; margin: 14px 0 18px; }
  .fblock { display: flex; flex-direction: column; gap: 3px; }
  .flabel { font-size: 9.5px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--text-muted); }
  .control {
    background: var(--surface); border: 1px solid var(--border); color: var(--text);
    border-radius: 5px; padding: 6px 10px; font-size: 12px; min-width: 150px;
  }
  .control.search { min-width: 220px; }
  .spacer { flex: 1; }
  .count { align-self: center; color: var(--text-muted); font-size: 12px; }

  .state { color: var(--text-muted); font-size: 13px; padding: 24px 4px; }

  .section { margin-bottom: 22px; border: 1px solid var(--border); border-radius: 9px; overflow: hidden; background: var(--surface); }
  .sec-head {
    display: flex; align-items: center; gap: 10px; padding: 10px 14px;
    background: rgba(0, 0, 0, 0.12); border-bottom: 1px solid var(--border);
  }
  .sec-head .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--accent, #7c3aed); }
  .sec-head h2 { font-size: 14px; margin: 0; font-weight: 650; }
  .sec-head .meta { color: var(--text-muted); font-size: 11.5px; }

  table { width: 100%; border-collapse: collapse; table-layout: fixed; }
  thead th {
    text-align: left; font-size: 10px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-muted); font-weight: 600; padding: 7px 12px; border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  thead th:first-child, tbody td:first-child { padding-left: 14px; }
  tbody td { padding: 9px 12px; border-bottom: 1px solid var(--border); vertical-align: middle; font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  tbody tr:last-child td { border-bottom: none; }
  tbody tr { cursor: pointer; transition: background 0.08s; }
  tbody tr:hover { background: rgba(124, 58, 237, 0.08); }
  tbody tr:hover .drill { opacity: 1; }
  .num { text-align: right; font-variant-numeric: tabular-nums; }
  .repo { font-weight: 600; color: var(--text); }
  .mono { font-family: "SF Mono", "Cascadia Code", Consolas, monospace; font-size: 11.5px; }
  .muted { color: var(--text-muted); }
  .domain { color: var(--accent, #7c3aed); text-decoration: none; }
  .domain:hover { text-decoration: underline; }
  .domain.none { color: var(--text-muted); cursor: default; }
  .drill { color: var(--text-muted); opacity: 0; transition: opacity 0.08s; font-size: 14px; }
  .sbadge {
    display: inline-block; min-width: 22px; text-align: center;
    background: rgba(0, 0, 0, 0.15); border: 1px solid var(--border); border-radius: 10px;
    padding: 1px 7px; font-size: 11px; color: var(--text-muted); font-variant-numeric: tabular-nums;
  }

  .env { display: inline-block; padding: 2px 9px; border-radius: 11px; font-size: 11px; font-weight: 600; }
  .env.prod { color: #16a34a; background: rgba(22, 163, 74, 0.15); }
  .env.test { color: #2563eb; background: rgba(37, 99, 235, 0.15); }
  .env.stg  { color: #d97706; background: rgba(217, 119, 6, 0.15); }
  .env.cust { color: #9aa0aa; background: rgba(124, 130, 138, 0.18); }

  .db-name { color: var(--text-muted); }
</style>
