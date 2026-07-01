// T-000136: background auto-sync timer. Fixed 60s check cadence; an actual
// sync run is gated on (master enabled) AND (configured interval elapsed since
// the last run, manual or auto) AND (no run currently in flight). Per-project
// opt-in via project.auto_sync_enabled. Errors are swallowed per-project so a
// failing repo doesn't spam toasts every tick.
import { get } from 'svelte/store';
import { autoSyncEnabled, autoSyncIntervalMin, autoSyncLastAt, saveAutoSyncLastAt } from './settings';
import { projects } from './projects';
import { syncProject } from '$lib/api/tauri-commands';

const CHECK_INTERVAL_MS = 60_000;
let timer: ReturnType<typeof setInterval> | null = null;
let running = false;
let lastRunMs = 0;

export function startAutoSyncTimer(): void {
  if (timer) return;
  // Seed from the persisted last-run timestamp so a fresh launch doesn't
  // immediately re-sync if the last auto-sync was recent.
  const persisted = get(autoSyncLastAt);
  if (persisted) {
    const t = Date.parse(persisted);
    if (Number.isFinite(t)) lastRunMs = t;
  }
  timer = setInterval(() => { void tick(); }, CHECK_INTERVAL_MS);
}

async function tick(): Promise<void> {
  if (running) return;
  if (!get(autoSyncEnabled)) return;
  const intervalMs = get(autoSyncIntervalMin) * 60_000;
  const now = Date.now();
  if (now - lastRunMs < intervalMs) return;
  const enabled = get(projects).filter((p) => p.auto_sync_enabled);
  if (enabled.length === 0) return;
  running = true;
  try {
    for (const p of enabled) {
      try {
        await syncProject(p.id);
      } catch {
        // swallow — don't spam toasts every tick on a persistently failing repo
      }
    }
    lastRunMs = now;
    await saveAutoSyncLastAt(new Date().toISOString());
  } finally {
    running = false;
  }
}

// Called when a MANUAL sync happens, to reset the elapsed clock (debounce): the
// next auto-run waits a full interval after a manual sync.
export function noteManualSync(): void {
  lastRunMs = Date.now();
}
