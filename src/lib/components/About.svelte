<script lang="ts">
  import { onMount } from 'svelte';
  import heroLogo from '$lib/assets/logo-large.png';
  import { tStore, tf, locale } from '$lib/i18n';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { getVersion } from '@tauri-apps/api/app';
  import { addToast } from '$lib/stores/ui';
  import {
    updaterStatus,
    lastCheckedAt,
    checkForUpdate,
    downloadAndInstall
  } from '$lib/stores/updater';

  let version = $state('');
  const githubUrl = 'https://github.com/SgonnovDmGit/github-repo-manager';
  const boostyUrl = 'https://boosty.to/sgonnovdm/donate';
  const tonAddress = 'UQA-0I3SN2vw8F2ZzEoOTXT36-ToF0mu4Yp4_6pVmsR_dI0S';
  let tonCopied = $state(false);

  onMount(async () => {
    try {
      version = await getVersion();
    } catch (err) {
      console.warn('getVersion failed', err);
    }
  });

  async function openGithub() {
    try { await openUrl(githubUrl); } catch (err) { addToast(String(err), 'error'); }
  }

  async function openBoosty() {
    try { await openUrl(boostyUrl); } catch (err) { addToast(String(err), 'error'); }
  }

  async function copyTon() {
    try {
      await navigator.clipboard.writeText(tonAddress);
      tonCopied = true;
      setTimeout(() => { tonCopied = false; }, 2000);
    } catch (err) {
      addToast(String(err), 'error');
    }
  }

  function formatLastChecked(ts: number | null): string {
    if (!ts) return $tStore('about.update.lastCheckedNever' as any);
    const date = new Date(ts);
    const loc = $locale === 'ru' ? 'ru-RU' : 'en-US';
    return date.toLocaleString(loc, {
      year: 'numeric', month: 'short', day: 'numeric',
      hour: '2-digit', minute: '2-digit'
    });
  }

  // 6 features for the "Что умеет" grid
  const features = [
    { icon: '🐛', key: 'about.features.bugs' },
    { icon: '⟳', key: 'about.features.sync' },
    { icon: '📦', key: 'about.features.deploy' },
    { icon: '📊', key: 'about.features.dashboard' },
    { icon: '🗺', key: 'about.features.graph' },
    { icon: '🔐', key: 'about.features.secrets' },
  ];
</script>

<div class="about">
  <!-- Hero: 2-col logo + info -->
  <div class="hero">
    <img src={heroLogo} alt="Solo Dev Hub" class="hero-logo" />
    <div class="hero-info">
      <h2 class="app-name">{$tStore('app.title')}</h2>
      <div class="version">{$tStore('about.version' as any)} {version}</div>
      <p class="tagline">{$tStore('about.tagline' as any)}</p>
      <button type="button" class="hero-link" onclick={openGithub} title={githubUrl}>
        🌐 {githubUrl}
      </button>
    </div>
  </div>

  <!-- Donate (featured, prominent) -->
  <div class="donate-card featured">
    <div class="donate-title">💖 {$tStore('about.support.title' as any)}</div>
    <div class="donate-row">
      <div class="donate-label-row">
        <span class="donate-label">{$tStore('about.support.boostyLabel' as any)}</span>
        <span class="donate-hint">{$tStore('about.support.boostyHint' as any)}</span>
      </div>
      <button type="button" class="link-btn" onclick={openBoosty} title={boostyUrl}>
        {boostyUrl}
      </button>
    </div>
    <div class="donate-row">
      <div class="donate-label-row">
        <span class="donate-label">{$tStore('about.support.tonLabel' as any)}</span>
        <span class="donate-hint">{$tStore('about.support.tonHint' as any)}</span>
      </div>
      <div class="ton-row">
        <code class="ton-address">{tonAddress}</code>
        <button type="button" class="copy-btn" onclick={copyTon}>
          {tonCopied
            ? $tStore('about.support.copied' as any)
            : $tStore('about.support.copy' as any)}
        </button>
      </div>
    </div>
  </div>

  <!-- Что умеет (features grid 2-col) -->
  <div class="features-card">
    <div class="section-title">{$tStore('about.features.title' as any)}</div>
    <div class="features-grid">
      {#each features as f}
        <div class="feature-row">
          <span class="feature-icon">{f.icon}</span>
          <span class="feature-text">{$tStore(f.key as any)}</span>
        </div>
      {/each}
    </div>
  </div>

  <!-- Update card (внизу, alt-канал в titlebar) -->
  <div class="update-card">
    <div class="update-header">
      <span class="update-title">{$tStore('about.update.title' as any)}</span>
      {#if $updaterStatus.kind === 'idle' || $updaterStatus.kind === 'upToDate' || $updaterStatus.kind === 'error'}
        <button type="button" class="link-btn" onclick={() => checkForUpdate(false)}>
          ↻ {$tStore('about.update.checkButton' as any)}
        </button>
      {/if}
    </div>

    {#if $updaterStatus.kind === 'idle'}
      <div class="update-body muted">
        {tf('about.update.lastCheckedAt' as any, formatLastChecked($lastCheckedAt))}
      </div>
    {:else if $updaterStatus.kind === 'checking'}
      <div class="update-body">
        <span class="spinner" aria-hidden="true"></span>
        {$tStore('about.update.checking' as any)}
      </div>
    {:else if $updaterStatus.kind === 'upToDate'}
      <div class="update-body success">
        ✓ {$tStore('about.update.upToDate' as any)}
        <div class="muted small">
          {tf('about.update.lastCheckedAt' as any, formatLastChecked($lastCheckedAt))}
        </div>
      </div>
    {:else if $updaterStatus.kind === 'available'}
      <div class="update-body">
        <div class="available-line">
          <strong>{tf('about.update.available' as any, $updaterStatus.version)}</strong>
        </div>
        <button type="button" class="install-btn" onclick={downloadAndInstall}>
          {$tStore('about.update.installButton' as any)}
        </button>
        {#if $updaterStatus.notes}
          <details class="notes">
            <summary>{$tStore('about.update.releaseNotes' as any)}</summary>
            <pre class="notes-body">{$updaterStatus.notes}</pre>
          </details>
        {/if}
      </div>
    {:else if $updaterStatus.kind === 'downloading'}
      <div class="update-body">
        {#if $updaterStatus.total}
          <div>{tf('about.update.downloading' as any, $updaterStatus.percent)}</div>
          <div class="progress">
            <div class="progress-bar" style="width: {$updaterStatus.percent}%"></div>
          </div>
        {:else}
          <div>{$tStore('about.update.downloadingUnknown' as any)}</div>
        {/if}
      </div>
    {:else if $updaterStatus.kind === 'installing'}
      <div class="update-body">
        <span class="spinner" aria-hidden="true"></span>
        {$tStore('about.update.installing' as any)}
      </div>
    {:else if $updaterStatus.kind === 'error'}
      <div class="update-body error-block">
        <div class="error-line">
          <span class="error-icon" aria-hidden="true">⚠</span>
          <span>{$tStore(`about.update.error.${$updaterStatus.category}` as any)}</span>
        </div>
      </div>
    {/if}
  </div>

  <!-- Devs one-liner. B-000008: removed standalone "ИИ-ассистент: Claude
       (Anthropic)" item; AI tooling is mentioned inline via "с ИИ-помощниками"
       so the line stays informative without sounding like product placement. -->
  <div class="devs-row">
    <span class="dev-label">{$tStore('about.devs.author' as any)}:</span>
    <span class="dev-value">{$tStore('about.devs.authorValue' as any)}, {$tStore('about.devs.aiHint' as any)}</span>
    <span class="dev-sep">·</span>
    <span class="dev-label">{$tStore('about.devs.license' as any)}:</span>
    <span class="dev-value">{$tStore('about.devs.licenseValue' as any)}</span>
  </div>
</div>

<style>
  .about {
    /* Adaptive horizontal padding: 32px на узком, до 80px на широком окне */
    padding: 28px clamp(32px, 4%, 80px);
    min-height: 100%;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  /* Vertical centering: auto-margin trick — содержимое центрируется когда
     умещается, и нормально скроллится когда не умещается (без clipping). */
  .about > :first-child { margin-top: auto; }
  .about > :last-child { margin-bottom: auto; }

  /* Hero — logo и название scale с шириной окна */
  .hero {
    display: grid;
    grid-template-columns: clamp(180px, 12vw, 280px) 1fr;
    gap: clamp(22px, 2vw, 40px);
    align-items: center;
  }
  .hero-logo {
    width: clamp(180px, 12vw, 280px);
    height: clamp(180px, 12vw, 280px);
    display: block;
    border-radius: 22px;
    flex-shrink: 0;
  }
  .hero-info { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .app-name { font-size: clamp(26px, 2vw, 36px); font-weight: 700; margin: 0; line-height: 1.1; }
  .version { font-size: 12px; color: var(--text-muted); font-family: monospace; }
  .tagline {
    font-size: 13.5px;
    color: var(--text);
    line-height: 1.5;
    margin: 6px 0;
  }
  .hero-link {
    background: none;
    border: none;
    color: var(--accent);
    padding: 0;
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    margin-top: 4px;
    text-decoration: none;
  }
  .hero-link:hover { text-decoration: underline; }

  /* Donate card */
  .donate-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .donate-card.featured {
    border-color: rgba(236, 72, 153, 0.4);
    background: linear-gradient(135deg, rgba(236, 72, 153, 0.04), rgba(244, 63, 94, 0.04));
  }
  :global([data-theme="dark"]) .donate-card.featured {
    background: linear-gradient(135deg, rgba(236, 72, 153, 0.10), rgba(244, 63, 94, 0.06));
  }
  .donate-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
  }
  .donate-row {
    /* Grid с явной колонкой 280px → линки/адреса в обоих строках начинаются
       на одном x-уровне. Не зависит от ширины текста label'а или font-render. */
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: 12px;
    align-items: baseline;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  .donate-row:first-of-type { border-top: none; padding-top: 0; }
  /* На узком окне (≤600px container) grid сворачивается в 1-кол */
  @media (max-width: 600px) {
    .donate-row { grid-template-columns: 1fr; }
  }
  /* Visual alignment: TON address имеет padding 8px (фон-бокс), Boosty link
     это plain text без бокса. Чтобы текст линка визуально начинался на том
     же x что текст внутри ton-address-бокса — линку даём такой же left-padding. */
  .donate-row > .link-btn { padding-left: 8px; }
  .donate-label-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
  }
  /* Фиксированная ширина label'а — "Boosty" короче чем "TON-кошелёк",
     min-width выравнивает hint'ы ("подписка / разовая" vs "прямой перевод") по x. */
  .donate-label { font-size: 13px; font-weight: 500; color: var(--text); min-width: 110px; }
  .donate-hint { font-size: 11px; color: var(--text-muted); }
  .ton-row { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .ton-address {
    font-family: monospace;
    font-size: 11px;
    background-color: rgba(0, 0, 0, 0.18);
    padding: 4px 8px;
    border-radius: 4px;
    color: var(--text);
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    word-break: break-all;
  }
  :global([data-theme="light"]) .ton-address { background-color: rgba(0, 0, 0, 0.04); }
  .copy-btn {
    background: none;
    border: 1px solid var(--border);
    color: var(--text);
    padding: 4px 12px;
    font-size: 12px;
    border-radius: 4px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .copy-btn:hover { background-color: var(--surface-hover, var(--border)); }
  .link-btn {
    background: none;
    border: none;
    color: var(--accent);
    padding: 0;
    font-size: 12px;
    cursor: pointer;
    text-align: left;
    word-break: break-all;
  }
  .link-btn:hover { text-decoration: underline; }

  /* Features card */
  .features-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 18px 20px;
  }
  .section-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    margin: 0 0 12px;
  }
  .features-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 24px;
  }
  .feature-row {
    display: grid;
    grid-template-columns: 22px 1fr;
    gap: 10px;
    font-size: 13px;
    line-height: 1.5;
    align-items: baseline;
  }
  .feature-icon { color: var(--accent); font-weight: 600; text-align: center; }
  .feature-text { color: var(--text); }

  /* Update card */
  .update-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 14px 20px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .update-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .update-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
  }
  .update-body {
    font-size: 13px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .muted { color: var(--text-muted); }
  .small { font-size: 12px; }
  .success { color: var(--success, #22c55e); }

  .error-block { color: var(--text); display: flex; flex-direction: column; gap: 10px; }
  .error-line {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    color: var(--danger, #ef4444);
    font-weight: 500;
  }
  .error-icon { flex-shrink: 0; font-size: 14px; line-height: 1.4; }

  .install-btn {
    align-self: flex-start;
    background: var(--accent);
    color: white;
    border: none;
    padding: 6px 14px;
    font-size: 13px;
    border-radius: 4px;
    cursor: pointer;
  }
  .install-btn:hover { opacity: 0.9; }
  .available-line { font-size: 14px; }
  .notes summary { cursor: pointer; font-size: 12px; color: var(--text-muted); }
  .notes-body {
    margin: 8px 0 0;
    padding: 10px;
    background-color: var(--surface-alt, rgba(0, 0, 0, 0.04));
    border-radius: 4px;
    font-size: 12px;
    font-family: monospace;
    white-space: pre-wrap;
    max-height: 240px;
    overflow: auto;
  }
  .progress {
    width: 100%;
    height: 6px;
    background-color: var(--border);
    border-radius: 3px;
    overflow: hidden;
  }
  .progress-bar {
    height: 100%;
    background-color: var(--accent);
    transition: width 0.2s ease;
  }
  .spinner {
    display: inline-block;
    width: 12px;
    height: 12px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    vertical-align: middle;
    margin-right: 6px;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* Devs one-liner */
  .devs-row {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 18px;
    display: flex;
    align-items: center;
    gap: 6px 12px;
    flex-wrap: wrap;
    font-size: 12px;
  }
  .dev-label { color: var(--text-muted); }
  .dev-value { color: var(--text); font-weight: 500; }
  .dev-sep { color: var(--border); }

  /* Narrow window — collapse to single column */
  @media (max-width: 720px) {
    .hero { grid-template-columns: 1fr; text-align: center; }
    .hero-logo { margin: 0 auto; }
    .features-grid { grid-template-columns: 1fr; }
  }

  /* Wide window — features expand to 3 columns + donate/update side-by-side */
  @media (min-width: 1100px) {
    .features-grid { grid-template-columns: 1fr 1fr 1fr; }
  }
</style>
