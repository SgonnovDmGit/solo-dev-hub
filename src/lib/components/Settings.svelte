<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    hasPat,
    workspaceRoot,
    theme,
    savePat,
    removePat,
    saveWorkspaceRoot,
    saveTheme,
    saveUiScaleMode,
    saveUiScaleManual,
    aiRulesLastSyncAt,
    autoSyncEnabled,
    autoSyncIntervalMin,
    autoSyncLastAt,
    saveAutoSyncEnabled,
    saveAutoSyncInterval,
  } from "$lib/stores/settings";
  import { uiScaleMode, uiScaleManual, uiScaleAutoComputed, SCALE_PRESETS } from "$lib/ui-scale";
  import { allRepos, loadAllRepos } from "$lib/stores/repos";
  import {
    scanWorkspaceForRepos,
    setRepoLocalPath,
    syncGlobalClaudeMd,
  } from "$lib/api/tauri-commands";
  import { currentScreen } from "$lib/stores/ui";
  import { addToast } from "$lib/stores/ui";
  import { tStore, locale, setLocale } from "$lib/i18n";
  import { tf } from "$lib/i18n";
  import { formatRelativeTime, nowTick } from "$lib/utils/time-format";
  import type { Locale } from "$lib/i18n/translations";

  let showPat = $state(false);
  let patInput = $state("");
  let isSavingPat = $state(false);
  let isRemovingPat = $state(false);
  let workspaceRootInput = $state($workspaceRoot ?? "");
  let isScanning = $state(false);

  $effect(() => {
    workspaceRootInput = $workspaceRoot ?? "";
  });

  async function handleSavePat() {
    if (!patInput.trim()) return;
    isSavingPat = true;
    const ok = await savePat(patInput.trim());
    if (ok) patInput = "";
    isSavingPat = false;
  }

  async function handleRemovePat() {
    isRemovingPat = true;
    await removePat();
    patInput = "";
    isRemovingPat = false;
  }

  async function browseWorkspaceFolder() {
    const selected = await open({ directory: true, title: $tStore("settings.workspaceCard" as any) });
    if (selected) workspaceRootInput = selected as string;
  }

  async function handleSaveWorkspaceRoot() {
    if (!workspaceRootInput.trim()) return;
    await saveWorkspaceRoot(workspaceRootInput.trim());
  }

  async function handleScanRepos() {
    if (!$workspaceRoot) return;
    isScanning = true;
    try {
      const githubNames = $allRepos.map((r) => r.github_name).filter((n): n is string => n !== null);
      const found = await scanWorkspaceForRepos($workspaceRoot, githubNames);
      let count = 0;
      for (const [name, path] of Object.entries(found)) {
        const repo = $allRepos.find((r) => r.github_name === name);
        if (repo) { await setRepoLocalPath(repo.id, path); count++; }
      }
      await loadAllRepos();
      addToast(tf("toast.scanComplete", count), "success");
    } catch (err) {
      addToast(tf("toast.syncFailed", String(err)), "error");
    } finally {
      isScanning = false;
    }
  }

  function openTemplates() { currentScreen.set({ name: "templates" }); }
  function openAppDefaults() { currentScreen.set({ name: "app_defaults" }); }
  function openGlobalClaudeEditor() { currentScreen.set({ name: "global_claude_editor" }); }

  let syncing = $state(false);

  async function handleSyncFromCard() {
    if (syncing) return;
    syncing = true;
    try {
      const result = await syncGlobalClaudeMd();
      aiRulesLastSyncAt.set(result.synced_at);
      addToast($tStore('appDefaults.syncGlobalDone' as any).replace('{0}', result.path), 'success');
    } catch (err) {
      addToast(String(err), 'error');
    } finally {
      syncing = false;
    }
  }

  let scaleSelect = $derived<string>($uiScaleMode === "auto" ? "auto" : String($uiScaleManual));

  async function onScaleChange(e: Event) {
    const value = (e.currentTarget as HTMLSelectElement).value;
    if (value === "auto") {
      await saveUiScaleMode("auto");
    } else {
      const n = parseFloat(value);
      if (!Number.isFinite(n)) return;
      await saveUiScaleManual(n);
      await saveUiScaleMode("manual");
    }
  }
</script>

<div class="settings">
  <h2 class="section-title">{$tStore("settings.title")}</h2>

  <!-- 1. GitHub Personal Access Token -->
  <section class="card-new">
    <div class="card-title-tab">GitHub Personal Access Token</div>
    <div class="pat-row-1" title={$tStore("settings.patTooltip" as any)}>
      <input
        type={showPat ? "text" : "password"}
        bind:value={patInput}
        placeholder={$hasPat ? $tStore("settings.tokenPlaceholderReplace") : $tStore("settings.tokenPlaceholderNew")}
        autocomplete="off"
        spellcheck="false"
      />
      <button class="icon-only" onclick={() => (showPat = !showPat)} title={showPat ? $tStore("settings.hideToken") : $tStore("settings.showToken")} type="button">
        {showPat ? "🙈" : "👁"}
      </button>
      <button class="primary" onclick={handleSavePat} disabled={isSavingPat || !patInput.trim()}>
        {isSavingPat ? $tStore("settings.validating") : $tStore("settings.saveToken")}
      </button>
    </div>
    <div class="pat-row-2">
      <div class="pat-row-2-left">
        <a class="help-link-inline" href="https://github.com/settings/tokens" target="_blank">{$tStore("settings.howToGetToken")} →</a>
        {#if $hasPat}<span class="status-saved">{$tStore("settings.patStatusSaved" as any)}</span>{/if}
      </div>
      {#if $hasPat}
        <button class="remove-btn" onclick={handleRemovePat} disabled={isRemovingPat}>
          {isRemovingPat ? $tStore("settings.removing") : $tStore("settings.removeToken")}
        </button>
      {/if}
    </div>
  </section>

  <!-- 2. Внешний вид -->
  <section class="card-new">
    <div class="card-title-tab">{$tStore("settings.appearance" as any)}</div>
    <div class="appearance-row">
      <div class="pref-pair">
        <span class="row-label">{$tStore("settings.langLabel" as any)}</span>
        <select bind:value={$locale} onchange={(e) => setLocale((e.currentTarget as HTMLSelectElement).value as Locale)}>
          <option value="ru">Русский</option>
          <option value="en">English</option>
        </select>
      </div>
      <div class="pref-pair">
        <span class="row-label">{$tStore("settings.themeLabel" as any)}</span>
        <select bind:value={$theme} onchange={(e) => saveTheme((e.currentTarget as HTMLSelectElement).value)}>
          <option value="dark">{$tStore("settings.themeDark" as any)}</option>
          <option value="light">{$tStore("settings.themeLight" as any)}</option>
        </select>
      </div>
      <div class="pref-pair">
        <span class="row-label">{$tStore("settings.uiScaleLabel" as any)}</span>
        <select value={scaleSelect} onchange={onScaleChange}>
          <option value="auto">{$tStore("settings.uiScaleAuto" as any)} ({Math.round($uiScaleAutoComputed * 100)}%)</option>
          {#each SCALE_PRESETS as preset}
            <option value={String(preset)}>{Math.round(preset * 100)}%</option>
          {/each}
        </select>
      </div>
    </div>
  </section>

  <!-- 3. Рабочее пространство -->
  <section class="card-new">
    <div class="card-title-tab">{$tStore("settings.workspaceCard" as any)}</div>
    <div class="row">
      <div class="row-label">{$tStore("settings.workspaceFolderLabel" as any)}</div>
      <div class="row-control">
        <input type="text" bind:value={workspaceRootInput} placeholder="C:\\Projects" autocomplete="off" />
        <button class="icon-only" onclick={browseWorkspaceFolder} title={$tStore("settings.browseFolderTooltip")} type="button">📁</button>
        <button class="primary" onclick={handleSaveWorkspaceRoot} disabled={!workspaceRootInput.trim()}>{$tStore("settings.save")}</button>
      </div>
    </div>
    <div class="row">
      <div class="row-label">{$tStore("settings.workspaceScanLabel" as any)}</div>
      <div class="row-control" style="justify-content:flex-start;">
        <button onclick={handleScanRepos} disabled={!$workspaceRoot || isScanning} type="button">
          {isScanning ? "..." : `↺ ${$tStore("settings.scanRepos")}`}
        </button>
      </div>
    </div>
  </section>

  <!-- 4. Шаблоны репозиториев -->
  <section class="card-new">
    <div class="card-title-tab">{$tStore("settings.templatesRepoCard" as any)}</div>
    <div class="row">
      <div class="row-control" style="grid-column: 1 / -1; justify-content:flex-start; gap: 8px;">
        <button onclick={openAppDefaults} type="button">{$tStore("settings.bucketRepoInit" as any)}</button>
        <button onclick={openTemplates} type="button">{$tStore("settings.bucketDeploy" as any)}</button>
      </div>
    </div>
  </section>

  <!-- 5. Глобальные правила AI -->
  <section class="card-new">
    <div class="card-title-tab">{$tStore("settings.aiRulesCard" as any)}</div>
    <div class="row">
      <div class="row-control" style="grid-column: 1 / -1; justify-content:flex-start; gap: 8px;">
        <button onclick={openGlobalClaudeEditor} type="button">{$tStore("settings.aiRulesOpenTemplate" as any)}</button>
        <button onclick={handleSyncFromCard} disabled={syncing} type="button">{syncing ? '...' : $tStore("settings.aiRulesSync" as any)}</button>
        <span class="sync-status" style="margin-left: auto;">
          {$aiRulesLastSyncAt
            ? $tStore('settings.aiRulesLastSync' as any).replace('{time}', formatRelativeTime($aiRulesLastSyncAt, $nowTick))
            : $tStore('settings.aiRulesNeverSynced' as any)}
        </span>
      </div>
    </div>
    <div class="ai-rules-hint">{$tStore("settings.aiRulesSkillsHint" as any)}</div>
  </section>

  <!-- 6. Авто-синхронизация -->
  <section class="card-new">
    <div class="card-title-tab">{$tStore("settings.autoSyncCard" as any)}</div>
    <div class="row">
      <div class="row-label">{$tStore("settings.autoSyncEnabled" as any)}</div>
      <div class="row-control" style="grid-column: 2 / -1; justify-content:flex-start;">
        <input
          type="checkbox"
          class="autosync-check"
          checked={$autoSyncEnabled}
          onchange={(e) => saveAutoSyncEnabled(e.currentTarget.checked)}
        />
      </div>
    </div>
    <div class="row">
      <div class="row-label">{$tStore("settings.autoSyncInterval" as any)}</div>
      <div class="row-control" style="grid-column: 2 / -1; justify-content:flex-start;">
        <input
          type="number"
          class="autosync-interval"
          min="5"
          max="120"
          step="1"
          value={$autoSyncIntervalMin}
          onchange={(e) => saveAutoSyncInterval(parseInt(e.currentTarget.value, 10))}
        />
      </div>
    </div>
    <div class="row">
      <div class="row-label">{$tStore("settings.autoSyncLast" as any)}</div>
      <div class="row-control" style="grid-column: 2 / -1; justify-content:flex-start;">
        <span class="sync-status">
          {$autoSyncLastAt
            ? formatRelativeTime($autoSyncLastAt, $nowTick)
            : $tStore("settings.autoSyncNever" as any)}
        </span>
      </div>
    </div>
    <div class="row">
      <div class="row-control" style="grid-column: 1 / -1; justify-content:flex-start;">
        <span class="autosync-hint">{$tStore("settings.autoSyncHint" as any)}</span>
      </div>
    </div>
  </section>
</div>

<style>
  .settings { padding: 24px; overflow-y: auto; height: 100%; }
  .section-title { font-size: 18px; font-weight: 700; margin: 0 0 20px 0; color: var(--text); }

  .card-new {
    background-color: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 12px 16px;
    margin-bottom: 12px;
  }
  .card-title-tab {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    margin-bottom: 10px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
  }
  .row {
    display: grid;
    grid-template-columns: 130px 1fr auto;
    gap: 12px;
    align-items: center;
    padding: 6px 0;
  }
  .row-label { font-size: 13px; color: var(--text); white-space: nowrap; }
  .row-control { display: flex; gap: 6px; align-items: center; min-width: 0; }
  .row-control input {
    flex: 1; min-width: 0; padding: 5px 8px; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: 4px; font-size: 12px;
  }
  .row-control button {
    padding: 5px 10px; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: 4px; cursor: pointer; font-size: 12px;
    white-space: nowrap;
  }
  .row-control button.primary { background: var(--accent); border-color: var(--accent); color: white; }
  .row-control button.icon-only { padding: 5px 7px; }
  .row-control button[disabled] { opacity: 0.5; cursor: not-allowed; }

  .pat-row-1 { display: flex; gap: 6px; align-items: center; }
  .pat-row-1 input { flex: 1; min-width: 0; padding: 5px 8px; background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: 4px; font-size: 12px; }
  .pat-row-1 button { padding: 5px 10px; background: var(--bg); color: var(--text); border: 1px solid var(--border); border-radius: 4px; cursor: pointer; font-size: 12px; white-space: nowrap; }
  .pat-row-1 button.primary { background: var(--accent); border-color: var(--accent); color: white; }
  .pat-row-1 button.icon-only { padding: 5px 7px; }
  .pat-row-1 button[disabled] { opacity: 0.5; cursor: not-allowed; }

  .pat-row-2 { margin-top: 6px; display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .pat-row-2-left { display: flex; align-items: center; gap: 12px; flex: 1; min-width: 0; }
  .help-link-inline { font-size: 12px; color: var(--accent); text-decoration: none; white-space: nowrap; }
  .help-link-inline:hover { text-decoration: underline; }
  .status-saved { font-size: 12px; color: #4ade80; white-space: nowrap; }
  .remove-btn {
    font-size: 11px; padding: 3px 8px; background: transparent; color: #f87171;
    border: 1px solid #f87171; border-radius: 4px; cursor: pointer; opacity: 0.7;
    white-space: nowrap;
  }
  .remove-btn:hover { opacity: 1; background: rgba(248,113,113,0.1); }
  .remove-btn[disabled] { opacity: 0.3; cursor: not-allowed; }

  .appearance-row { display: flex; gap: 32px; align-items: center; padding: 6px 0; }
  .pref-pair { display: flex; align-items: center; gap: 10px; }
  .pref-pair select {
    padding: 5px 8px; background: var(--bg); color: var(--text);
    border: 1px solid var(--border); border-radius: 4px; font-size: 12px;
  }

  .row-control button[type="button"]:not(.primary):not(.icon-only):hover { background: var(--surface); }

  .sync-status { font-size: 11px; color: var(--text-muted); font-style: italic; }

  .autosync-check { width: 16px; height: 16px; cursor: pointer; flex: none; }
  .autosync-interval { flex: none !important; width: 80px; }
  .autosync-hint { font-size: 11px; color: var(--text-muted); }
  .ai-rules-hint { font-size: 11px; color: var(--text-muted); margin-top: 6px; line-height: 1.4; }
</style>
