<script lang="ts">
  import { onMount } from 'svelte';
  import '../app.css';
  import { currentScreen, navigateTo, goBack } from '$lib/stores/ui';
  import { loadSettings } from '$lib/stores/settings';
  import { initUiScale } from '$lib/ui-scale';
  import { loadProjects } from '$lib/stores/projects';
  import { loadAllRepos, pendingMergeCases, type AmbiguousMergeCase } from '$lib/stores/repos';
  import { addToast } from '$lib/stores/ui';
  import { resolveMergeWithLocal, forceInsertGithubRepo } from '$lib/api/tauri-commands';
  import { tStore, tf, initLocale } from '$lib/i18n';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { checkForUpdate, updaterStatus } from '$lib/stores/updater';
  import Toast from '$lib/components/Toast.svelte';
  import MergeChoiceDialog from '$lib/components/MergeChoiceDialog.svelte';
  import Sidebar from '$lib/components/Sidebar.svelte';
  import RepoDetail from '$lib/components/RepoDetail.svelte';
  import Settings from '$lib/components/Settings.svelte';
  import ProjectDetail from '$lib/components/ProjectDetail.svelte';
  import Dashboard from '$lib/components/Dashboard.svelte';
  import SyncScreen from '$lib/components/SyncScreen.svelte';
  import TemplatesScreen from '$lib/components/TemplatesScreen.svelte';
  import AppDefaultsScreen from '$lib/components/AppDefaultsScreen.svelte';
  import DeployScreen from '$lib/components/DeployScreen.svelte';
  import About from '$lib/components/About.svelte';
  import Timeline from '$lib/components/Timeline.svelte';
  import GlobalClaudeEditor from '$lib/components/GlobalClaudeEditor.svelte';
  import InputContextMenu from '$lib/components/InputContextMenu.svelte';
  import logo from '$lib/assets/logo.png';

  const appWindow = getCurrentWindow();

  // B-000006: track maximize state so titlebar can swap maximize→restore icon
  // (and tooltip) instead of static "□" glyph that didn't reflect window state.
  let isMaximized = $state(false);

  // B-000007: custom context menu state for input/textarea fields. We suppress
  // the WebView2 native menu in PROD (which leaked browser-flavoured items
  // like "More tools" / "Writing direction") and render a clean 4-item
  // replacement instead. Use `$state.raw` because the value contains a DOM
  // element (`target`) and Svelte 5's default deep proxy on $state misbehaves
  // when wrapping non-plain objects like HTMLElement — that was likely why
  // round 2 silently rendered nothing in PROD: state assignment effectively
  // no-op'd through the proxy.
  let ctxMenu = $state.raw<{ x: number; y: number; target: HTMLInputElement | HTMLTextAreaElement } | null>(null);

  async function refreshMaximized() {
    try { isMaximized = await appWindow.isMaximized(); } catch {}
  }
  async function handleToggleMaximize() {
    await appWindow.toggleMaximize();
    await refreshMaximized();
  }

  onMount(async () => {
    await initLocale();
    await loadSettings();
    await initUiScale();
    await Promise.all([loadProjects(), loadAllRepos()]);
    checkForUpdate(true);

    // B-000007: suppress WebView2's default right-click menu in release builds
    // (Inspect / Reload / etc — distracting and unprofessional in a desktop
    // app). Native cut/copy/paste menu inside input/textarea/contenteditable
    // is preserved so users can still right-click in form fields. In dev
    // builds the menu stays available for debugging.
    if (import.meta.env.PROD) {
      document.addEventListener('contextmenu', (e: MouseEvent) => {
        e.preventDefault();
        const target = e.target as HTMLElement | null;
        const field = target?.closest('input, textarea') as
          | HTMLInputElement
          | HTMLTextAreaElement
          | null;
        // Right-click in an input → show custom menu; outside → clear (so
        // a previously-open menu from another field doesn't linger).
        ctxMenu = field ? { x: e.clientX, y: e.clientY, target: field } : null;
      });
    }

    // B-000006: subscribe to resize events so the maximize/restore icon stays
    // in sync with snap-resize, double-click on titlebar, OS shortcuts etc —
    // not just our explicit toggleMaximize click.
    await refreshMaximized();
    appWindow.onResized(() => { void refreshMaximized(); });
  });

  let currentMergeCase = $derived<AmbiguousMergeCase | null>($pendingMergeCases[0] ?? null);

  function popMergeCase() {
    pendingMergeCases.update((q) => q.slice(1));
  }

  async function onMergeResolve(localId: number) {
    const c = currentMergeCase;
    if (!c) return;
    try {
      const updated = await resolveMergeWithLocal(
        localId,
        c.github_name,
        c.github_url,
        c.description,
        c.language,
        c.last_pushed_at,
        c.github_id,
      );
      addToast(tf('toast.repoMerged', updated.github_name ?? '', updated.local_path ?? ''), 'success');
      await loadAllRepos();
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      popMergeCase();
    }
  }

  async function onMergeCreateNew() {
    const c = currentMergeCase;
    if (!c) return;
    try {
      await forceInsertGithubRepo(
        c.github_name,
        c.github_url,
        c.description,
        c.language,
        c.last_pushed_at,
        c.github_id,
      );
      await loadAllRepos();
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      popMergeCase();
    }
  }

  function onMergeSkip() {
    const c = currentMergeCase;
    if (c) addToast(tf('toast.mergeSkipped', c.github_name), 'warning');
    popMergeCase();
  }
</script>

<div class="app" class:maximized={isMaximized}>
  <header class="titlebar" data-tauri-drag-region>
    <div class="titlebar-left" data-tauri-drag-region>
      <img src={logo} alt="Solo Dev Hub" class="app-logo" data-tauri-drag-region />
      <span class="titlebar-title" data-tauri-drag-region>{$tStore('app.title')}</span>
    </div>

    <div class="titlebar-right">
      {#if $updaterStatus.kind === 'available'}
        <button
          class="update-cta titlebar-btn"
          onclick={() => navigateTo('about')}
          title={$tStore('about.update.badge' as any)}
        >
          ⬇ {tf('about.update.headerCta' as any, $updaterStatus.version)}
        </button>
      {/if}
      <button
        class="ghost titlebar-btn"
        class:active={$currentScreen.name === 'dashboard'}
        onclick={() => $currentScreen.name === 'dashboard' ? goBack() : navigateTo('dashboard')}
        title={$tStore('dashboard.tooltip')}
      >
        📊 {$tStore('dashboard.title')}
      </button>
      <button
        class="ghost titlebar-btn"
        class:active={$currentScreen.name === 'timeline'}
        onclick={() => $currentScreen.name === 'timeline' ? goBack() : navigateTo('timeline')}
        title={$tStore('timeline.title' as any)}
      >
        📅 {$tStore('timeline.title' as any)}
      </button>
      <button
        class="ghost titlebar-btn"
        class:active={$currentScreen.name === 'settings'}
        onclick={() => $currentScreen.name === 'settings' ? goBack() : navigateTo('settings')}
        title={$tStore('app.openSettings')}
      >
        ⚙ {$tStore('app.settings')}
      </button>
      <button
        class="ghost titlebar-btn"
        class:active={$currentScreen.name === 'about'}
        onclick={() => $currentScreen.name === 'about' ? goBack() : navigateTo('about')}
        title={$tStore('about.tooltip' as any)}
      >
        ℹ {$tStore('about.title' as any)}
      </button>

      <div class="window-controls">
        <!-- B-000006: SVG glyphs replace Unicode "─ □ ✕" — sharper, properly
             scaled, and the maximize button now swaps to a restore-down icon
             when window is maximized (mirrors native Windows titlebar). -->
        <button
          class="win-btn"
          onclick={() => appWindow.minimize()}
          title={$tStore('app.minimize')}
          aria-label={$tStore('app.minimize')}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="square">
            <path d="M2 6 H10" />
          </svg>
        </button>
        <button
          class="win-btn"
          onclick={handleToggleMaximize}
          title={isMaximized ? $tStore('app.restore') : $tStore('app.maximize')}
          aria-label={isMaximized ? $tStore('app.restore') : $tStore('app.maximize')}
        >
          {#if isMaximized}
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="square">
              <rect x="3" y="1.5" width="7.5" height="7.5" />
              <rect x="1.5" y="3" width="7.5" height="7.5" />
            </svg>
          {:else}
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="square">
              <rect x="1.5" y="1.5" width="9" height="9" />
            </svg>
          {/if}
        </button>
        <button
          class="win-btn win-close"
          onclick={() => appWindow.close()}
          title={$tStore('app.close')}
          aria-label={$tStore('app.close')}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="square">
            <path d="M2 2 L10 10 M10 2 L2 10" />
          </svg>
        </button>
      </div>
    </div>
  </header>

  <div class="body">
    <Sidebar />

    <main class="content">
      {#if $currentScreen.name === 'repo-detail'}
        <RepoDetail />
      {:else if $currentScreen.name === 'settings'}
        <Settings />
      {:else if $currentScreen.name === 'project'}
        <ProjectDetail />
      {:else if $currentScreen.name === 'dashboard'}
        <Dashboard />
      {:else if $currentScreen.name === 'sync'}
        <SyncScreen />
      {:else if $currentScreen.name === 'templates'}
        <TemplatesScreen />
      {:else if $currentScreen.name === 'app_defaults'}
        <AppDefaultsScreen />
      {:else if $currentScreen.name === 'deploy'}
        <DeployScreen />
      {:else if $currentScreen.name === 'about'}
        <About />
      {:else if $currentScreen.name === 'timeline'}
        <Timeline />
      {:else if $currentScreen.name === 'global_claude_editor'}
        <GlobalClaudeEditor />
      {/if}
    </main>
  </div>
</div>

<Toast />

{#if currentMergeCase}
  <MergeChoiceDialog
    githubName={currentMergeCase.github_name}
    candidates={currentMergeCase.candidates}
    onResolve={onMergeResolve}
    onCreateNew={onMergeCreateNew}
    onSkip={onMergeSkip}
  />
{/if}

{#if ctxMenu}
  <InputContextMenu
    x={ctxMenu.x}
    y={ctxMenu.y}
    target={ctxMenu.target}
    onClose={() => (ctxMenu = null)}
  />
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
    background-color: var(--bg);
    color: var(--text);
  }

  /* B-000014: Win11 + custom-titlebar (decorations: false) maximize bug.
     When maximized, Win extends the window ~8px past each screen edge to
     hide the invisible resize border. With our 100vh content, the bottom
     8px (and right 8px, etc) bleed offscreen — visible as a scrollbar /
     border that "goes below screen", especially with vertical scroll.
     Compensate by adding equivalent padding inside .app when maximized;
     `box-sizing: border-box` keeps total at 100vh. */
  .app.maximized {
    box-sizing: border-box;
    padding: 0 8px 8px 8px;
  }

  .titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 0 0 12px;
    height: 32px;
    background-color: var(--sidebar-bg);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    user-select: none;
  }

  .titlebar-left {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
  }

  .app-logo {
    width: 20px;
    height: 20px;
    display: block;
    flex-shrink: 0;
    pointer-events: none;
  }

  .titlebar-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
    opacity: 0.8;
  }

  .titlebar-right {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 100%;
  }

  .titlebar-btn {
    font-size: 12px;
    padding: 2px 8px;
    height: 100%;
    border-radius: 0;
  }

  .titlebar-btn.active {
    background-color: var(--border);
    color: var(--accent);
  }

  .update-cta {
    font-size: 12px;
    padding: 2px 10px;
    height: 100%;
    border-radius: 0;
    border: none;
    background-color: #22c55e;
    color: white;
    cursor: pointer;
    font-weight: 500;
    white-space: nowrap;
  }

  .update-cta:hover {
    background-color: #16a34a;
  }

  .window-controls {
    display: flex;
    height: 100%;
  }

  .win-btn {
    width: 46px;
    height: 100%;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .win-btn:hover {
    background-color: var(--border);
  }

  .win-close:hover {
    background-color: #e81123;
    color: white;
  }

  .body {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  .content {
    flex: 1;
    overflow: auto;
    background-color: var(--bg);
  }
</style>
