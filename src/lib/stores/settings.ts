import { writable, derived } from 'svelte/store';
import {
  getPat as tauriGetPat,
  storePat as tauriStorePat,
  deletePat as tauriDeletePat,
  getSetting,
  setSetting,
} from '$lib/api/tauri-commands';
import { validateToken } from '$lib/api/github';
import { addToast } from './ui';
import { t, tf, initLocale } from '$lib/i18n';
import {
  uiScaleMode,
  uiScaleManual,
  recomputeAndApply,
  type UiScaleMode,
} from '$lib/ui-scale';

export const pat = writable<string | null>(null);
export const workspaceRoot = writable<string | null>(null);
export const theme = writable<string>('dark');
export const aiRulesLastSyncAt = writable<string | null>(null);
// T-000136: background auto-sync master switch + interval (min) + last-run stamp.
export const autoSyncEnabled = writable<boolean>(false);
export const autoSyncIntervalMin = writable<number>(15);
export const autoSyncLastAt = writable<string | null>(null);
export const hasPat = derived(pat, ($pat) => $pat !== null && $pat.length > 0);

export async function loadSettings(): Promise<void> {
  try {
    await initLocale();
    const [storedPat, storedWorkspaceRoot] = await Promise.all([
      tauriGetPat(),
      getSetting('workspace_root'),
    ]);
    pat.set(storedPat);
    workspaceRoot.set(storedWorkspaceRoot);
    const storedTheme = await getSetting('theme');
    if (storedTheme) {
      theme.set(storedTheme);
      document.documentElement.dataset.theme = storedTheme;
    }
    const storedScaleMode = await getSetting('ui_scale_mode');
    if (storedScaleMode === 'manual') {
      uiScaleMode.set('manual');
    } else {
      uiScaleMode.set('auto');
    }
    const storedScaleManual = await getSetting('ui_scale_manual');
    if (storedScaleManual) {
      const n = parseFloat(storedScaleManual);
      if (Number.isFinite(n) && n > 0) uiScaleManual.set(n);
    }
    const storedAiRulesLastSyncAt = await getSetting('ai_rules_last_sync_at');
    aiRulesLastSyncAt.set(storedAiRulesLastSyncAt || null);
    const storedAutoSyncEnabled = await getSetting('auto_sync_enabled');
    autoSyncEnabled.set(storedAutoSyncEnabled === 'true');
    const storedAutoSyncInterval = await getSetting('auto_sync_interval_min');
    if (storedAutoSyncInterval) {
      const n = parseInt(storedAutoSyncInterval, 10);
      if (Number.isFinite(n) && n >= 5 && n <= 120) autoSyncIntervalMin.set(n);
    }
    autoSyncLastAt.set((await getSetting('auto_sync_last_at')) || null);
  } catch (err) {
    addToast(tf('toast.failedToLoadSettings', String(err)), 'error');
  }
}

export async function saveUiScaleMode(mode: UiScaleMode): Promise<void> {
  try {
    await setSetting('ui_scale_mode', mode);
    uiScaleMode.set(mode);
    await recomputeAndApply();
  } catch (err) {
    addToast(tf('toast.failedToSaveSetting', String(err)), 'error');
  }
}

export async function saveUiScaleManual(scale: number): Promise<void> {
  try {
    await setSetting('ui_scale_manual', String(scale));
    uiScaleManual.set(scale);
    await recomputeAndApply();
  } catch (err) {
    addToast(tf('toast.failedToSaveSetting', String(err)), 'error');
  }
}

export async function savePat(token: string): Promise<boolean> {
  try {
    const valid = await validateToken(token);
    if (!valid) {
      addToast(t('toast.invalidToken'), 'error');
      return false;
    }
    await tauriStorePat(token);
    pat.set(token);
    addToast(t('toast.tokenSaved'), 'success');
    return true;
  } catch (err) {
    addToast(tf('toast.failedToSaveToken', String(err)), 'error');
    return false;
  }
}

export async function removePat(): Promise<void> {
  try {
    await tauriDeletePat();
    pat.set(null);
    addToast(t('toast.tokenRemoved'), 'info');
  } catch (err) {
    addToast(tf('toast.failedToRemoveToken', String(err)), 'error');
  }
}

export async function saveWorkspaceRoot(path: string): Promise<void> {
  try {
    await setSetting('workspace_root', path);
    workspaceRoot.set(path);
    addToast(t('toast.workspaceRootSaved'), 'success');
  } catch (err) {
    addToast(tf('toast.failedToSaveSetting', String(err)), 'error');
  }
}

export async function saveTheme(newTheme: string): Promise<void> {
  try {
    await setSetting('theme', newTheme);
    theme.set(newTheme);
    document.documentElement.dataset.theme = newTheme;
  } catch (err) {
    addToast(tf('toast.failedToSaveSetting', String(err)), 'error');
  }
}

// T-000136: persist the auto-sync master switch.
export async function saveAutoSyncEnabled(enabled: boolean): Promise<void> {
  try {
    await setSetting('auto_sync_enabled', enabled ? 'true' : 'false');
    autoSyncEnabled.set(enabled);
  } catch (err) {
    addToast(tf('toast.failedToSaveSetting', String(err)), 'error');
  }
}

// T-000136: persist the auto-sync interval, clamped to 5..120 minutes.
export async function saveAutoSyncInterval(min: number): Promise<void> {
  const clamped = Math.min(120, Math.max(5, Math.round(min)));
  try {
    await setSetting('auto_sync_interval_min', String(clamped));
    autoSyncIntervalMin.set(clamped);
  } catch (err) {
    addToast(tf('toast.failedToSaveSetting', String(err)), 'error');
  }
}

// T-000136: persist the last auto-sync run timestamp (display-only; non-fatal).
export async function saveAutoSyncLastAt(iso: string): Promise<void> {
  try {
    await setSetting('auto_sync_last_at', iso);
    autoSyncLastAt.set(iso);
  } catch (err) {
    // non-fatal: timestamp display only
  }
}
