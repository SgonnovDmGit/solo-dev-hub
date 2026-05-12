import { writable, get } from 'svelte/store';
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export type UpdaterErrorCategory = 'network' | 'notFound' | 'signature' | 'unknown';

export type UpdaterStatus =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'upToDate' }
  | { kind: 'available'; update: Update; version: string; notes: string }
  | { kind: 'downloading'; percent: number; downloaded: number; total: number | null }
  | { kind: 'installing' }
  | { kind: 'error'; category: UpdaterErrorCategory; message: string };

function categorizeError(err: unknown): UpdaterErrorCategory {
  const s = String(err).toLowerCase();
  if (
    s.includes('could not fetch') ||
    s.includes('not found') ||
    s.includes('404') ||
    s.includes('no release')
  ) {
    return 'notFound';
  }
  if (
    s.includes('network') ||
    s.includes('timeout') ||
    s.includes('timed out') ||
    s.includes('connection') ||
    s.includes('connect error') ||
    s.includes('dns') ||
    s.includes('unreachable') ||
    s.includes('offline')
  ) {
    return 'network';
  }
  if (
    s.includes('signature') ||
    s.includes('verif') ||
    s.includes('pubkey') ||
    s.includes('minisign')
  ) {
    return 'signature';
  }
  return 'unknown';
}

export const updaterStatus = writable<UpdaterStatus>({ kind: 'idle' });
export const lastCheckedAt = writable<number | null>(null);

const LAST_CHECK_KEY = 'updater.lastCheckAt';

function loadLastCheckedAt(): number {
  const raw = Number(localStorage.getItem(LAST_CHECK_KEY) ?? 0);
  lastCheckedAt.set(raw > 0 ? raw : null);
  return raw;
}

function saveLastCheckedAt(ts: number): void {
  localStorage.setItem(LAST_CHECK_KEY, String(ts));
  lastCheckedAt.set(ts);
}

export async function checkForUpdate(silent = false): Promise<void> {
  updaterStatus.set({ kind: 'checking' });
  try {
    const update = await check();
    saveLastCheckedAt(Date.now());
    if (update) {
      updaterStatus.set({
        kind: 'available',
        update,
        version: update.version,
        notes: update.body ?? ''
      });
    } else {
      updaterStatus.set({ kind: 'upToDate' });
    }
  } catch (err) {
    const category = categorizeError(err);
    if (silent) {
      // M11 review-fix: silent-mode previously reset everything to `idle`
      // and swallowed all error categories — fine for `notFound` (expected
      // on private repo pre-public-flip), but `network` / `signature` /
      // `unknown` are real problems that the About card should be able to
      // surface when the user opens it. Keep `notFound` quiet, escalate
      // the others into the normal error state.
      if (category === 'notFound') {
        updaterStatus.set({ kind: 'idle' });
        console.warn('Silent update check: not found (expected on private repo)', err);
        return;
      }
      console.warn('Silent update check surfaced unexpected error:', err);
    } else {
      console.warn('Update check failed:', err);
    }
    updaterStatus.set({ kind: 'error', category, message: String(err) });
  }
}

export async function downloadAndInstall(): Promise<void> {
  const status = get(updaterStatus);
  if (status.kind !== 'available') return;
  const update = status.update;
  let total: number | null = null;
  let downloaded = 0;
  try {
    updaterStatus.set({ kind: 'downloading', percent: 0, downloaded: 0, total: null });
    await update.downloadAndInstall((event) => {
      if (event.event === 'Started') {
        total = event.data.contentLength ?? null;
        updaterStatus.set({ kind: 'downloading', percent: 0, downloaded: 0, total });
      } else if (event.event === 'Progress') {
        downloaded += event.data.chunkLength;
        const percent = total ? Math.min(100, Math.round((downloaded / total) * 100)) : 0;
        updaterStatus.set({ kind: 'downloading', percent, downloaded, total });
      } else if (event.event === 'Finished') {
        updaterStatus.set({ kind: 'installing' });
      }
    });
    updaterStatus.set({ kind: 'installing' });
    await relaunch();
  } catch (err) {
    updaterStatus.set({ kind: 'error', category: categorizeError(err), message: String(err) });
  }
}

export function dismissUpdateStatus(): void {
  const status = get(updaterStatus);
  if (status.kind === 'upToDate' || status.kind === 'error') {
    updaterStatus.set({ kind: 'idle' });
  }
}

// Инициализация: подгрузить timestamp из localStorage при импорте
if (typeof window !== 'undefined') {
  loadLastCheckedAt();
}
