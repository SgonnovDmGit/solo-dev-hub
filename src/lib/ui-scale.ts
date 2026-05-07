import { writable, get } from 'svelte/store';
import { getCurrentWindow, currentMonitor } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';

export type UiScaleMode = 'auto' | 'manual';

export const SCALE_PRESETS: readonly number[] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5] as const;

export const uiScaleMode = writable<UiScaleMode>('auto');
export const uiScaleManual = writable<number>(1.0);
export const uiScaleApplied = writable<number>(1.0);
// What the auto-heuristic would yield for the current monitor, regardless of
// the active mode. Used by the "Авто (NN%)" dropdown label so the displayed
// auto-value stays meaningful even while user is in manual mode.
export const uiScaleAutoComputed = writable<number>(1.0);

export function computeAutoScale(effectiveWidth: number): number {
  if (effectiveWidth >= 3500) return 1.5;
  if (effectiveWidth >= 2500) return 1.25;
  if (effectiveWidth >= 1900) return 1.1;
  return 1.0;
}

async function getEffectiveWidth(): Promise<number> {
  try {
    const monitor = await currentMonitor();
    if (!monitor) return 1920;
    const scaleFactor = monitor.scaleFactor || 1;
    return monitor.size.width / scaleFactor;
  } catch {
    return 1920;
  }
}

async function applyZoom(scale: number): Promise<void> {
  try {
    await getCurrentWebview().setZoom(scale);
    uiScaleApplied.set(scale);
  } catch (err) {
    console.warn('webview.setZoom failed', err);
  }
}

export async function recomputeAndApply(): Promise<void> {
  // Always compute the auto value so the dropdown label "Авто (NN%)" reflects
  // what auto would yield for the current monitor, even if user is in manual
  // mode. Apply, however, depends on mode.
  const w = await getEffectiveWidth();
  const autoValue = computeAutoScale(w);
  uiScaleAutoComputed.set(autoValue);

  const mode = get(uiScaleMode);
  const target = mode === 'manual' ? get(uiScaleManual) : autoValue;
  await applyZoom(target);
}

let onMovedUnlisten: (() => void) | null = null;
let debounceTimer: ReturnType<typeof setTimeout> | null = null;

export async function initUiScale(): Promise<void> {
  // Defensive cleanup: an earlier dev iteration of B-000017 set CSS `zoom` on
  // documentElement, which HMR doesn't clear. Strip any leftover inline zoom
  // before applying our WebView-level zoom — otherwise the two would stack.
  try {
    (document.documentElement.style as { zoom?: string }).zoom = '';
  } catch {
    // ignore
  }
  await recomputeAndApply();
  if (onMovedUnlisten) return;
  try {
    onMovedUnlisten = await getCurrentWindow().onMoved(() => {
      // Recompute even in manual mode — the dropdown's "Авто (NN%)" label
      // should reflect the new monitor's auto-suggestion. Applying in manual
      // mode is a no-op (same value re-set).
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(() => {
        void recomputeAndApply();
      }, 300);
    });
  } catch (err) {
    console.warn('onMoved subscription failed', err);
  }
}
