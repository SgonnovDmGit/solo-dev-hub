<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import cytoscape from 'cytoscape';
  import type { Core } from 'cytoscape';
  import { getProjectGraph } from '$lib/api/tauri-commands';
  import type { ProjectGraph, GraphNode } from '$lib/types';
  import { ROLE_ICONS } from '$lib/types';
  import { selectedRepoId, selectedProjectId, currentScreen } from '$lib/stores/ui';
  import { theme } from '$lib/stores/settings';
  import { tStore } from '$lib/i18n';

  interface Props { projectId: number; }
  let { projectId }: Props = $props();

  let containerEl: HTMLDivElement;
  let cy: Core | null = null;
  let loading = $state(true);
  let isEmpty = $state(false);
  let errorMsg = $state<string | null>(null);

  function composeLabel(n: GraphNode): string {
    const icon = n.role ? (ROLE_ICONS[n.role] ?? '') : '';
    return icon ? `${icon}  ${n.label}` : n.label;
  }

  function buildStyleSheet() {
    const cs = getComputedStyle(document.documentElement);
    const surface = cs.getPropertyValue('--surface').trim() || '#25282d';
    const text = cs.getPropertyValue('--text').trim() || '#e6e6e6';
    const textMuted = cs.getPropertyValue('--text-muted').trim() || '#94a3b8';
    const accentServer = cs.getPropertyValue('--accent').trim() || '#5b9dd9';
    const accentClient = '#4ade80';
    const accentMs = '#c084fc';
    const accentTool = textMuted;

    return [
      { selector: 'node', style: {
        'shape': 'round-rectangle',
        'background-color': surface,
        'label': 'data(label)',
        'color': text,
        'text-valign': 'center',
        'text-halign': 'center',
        'width': 'label',
        'height': 40,
        'padding': '12px',
        'font-size': 12,
        'border-width': 1.5,
      }},
      { selector: 'node[isCenter]', style: { 'border-width': 3, 'height': 50, 'font-weight': 600 }},
      { selector: 'node[role = "server"]',       style: { 'border-color': accentServer }},
      { selector: 'node[role = "client"]',       style: { 'border-color': accentClient }},
      { selector: 'node[role = "landing"]',      style: { 'border-color': accentClient }},
      { selector: 'node[role = "test_client"]',  style: { 'border-color': accentClient }},
      { selector: 'node[role = "admin_client"]', style: { 'border-color': accentClient }},
      { selector: 'node[role = "microservice"]', style: { 'border-color': accentMs }},
      { selector: 'node[role = "tool"]',         style: { 'border-color': accentTool }},
      { selector: 'edge', style: {
        'line-color': textMuted,
        'opacity': 0.7,
        'width': 1.5,
        'curve-style': 'straight',
      }},
      { selector: 'edge[kind = "cross_project_ms"]', style: { 'line-style': 'dashed' }},
    ];
  }

  async function buildGraph() {
    loading = true;
    errorMsg = null;
    try {
      const data: ProjectGraph = await getProjectGraph(projectId);
      if (!data.center) {
        isEmpty = true;
        loading = false;
        return;
      }
      isEmpty = false;
      const elements = [
        { data: { id: data.center.id, label: composeLabel(data.center), role: data.center.role, isCenter: true } },
        ...data.ring.map((n) => ({ data: { id: n.id, label: composeLabel(n), role: n.role, isCenter: false } })),
        ...data.edges.map((e) => ({ data: { source: e.source, target: e.target, kind: e.kind } })),
      ];
      cy?.destroy();
      cy = cytoscape({
        container: containerEl,
        elements,
        style: buildStyleSheet() as any,
        layout: {
          name: 'concentric',
          concentric: (n: any) => (n.data('isCenter') ? 1 : 0),
          levelWidth: () => 1,
          minNodeSpacing: 60,
        },
        wheelSensitivity: 0.2,
      });
      cy.on('tap', 'node', (evt) => {
        const id = String(evt.target.data('id'));
        if (id.startsWith('repo:')) {
          selectedRepoId.set(parseInt(id.slice(5), 10));
          currentScreen.set({ name: 'repo-detail' });
        } else if (id.startsWith('project:')) {
          selectedProjectId.set(parseInt(id.slice(8), 10));
          currentScreen.set({ name: 'project' });
        }
      });
    } catch (err) {
      errorMsg = String(err);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    buildGraph();
  });

  $effect(() => {
    void $theme;
    if (cy) cy.style(buildStyleSheet() as any).update();
  });

  $effect(() => {
    void projectId;
    if (cy) buildGraph();
  });

  onDestroy(() => {
    cy?.destroy();
    cy = null;
  });
</script>

<div class="graph-wrapper">
  <div class="graph-container" bind:this={containerEl}></div>
  {#if loading}
    <div class="overlay loading">…</div>
  {:else if isEmpty}
    <div class="overlay empty">{$tStore('project.graphEmpty' as any)}</div>
  {:else if errorMsg}
    <div class="overlay error">{errorMsg}</div>
  {/if}
</div>

<style>
  /* Explicit min-height — parent .project-detail uses block+overflow-y, not flex.
     Without this, cytoscape canvas gets 0x0 and renders nothing. */
  .graph-wrapper { position: relative; min-height: 600px; height: calc(100vh - 280px); display: flex; }
  .graph-container { flex: 1; background: var(--bg); border: 1px solid var(--border); border-radius: 6px; }
  .overlay {
    position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;
    color: var(--text-muted); font-size: 13px; pointer-events: none;
  }
  .overlay.error { color: var(--text); background: rgba(239, 68, 68, 0.1); }
</style>
