import { writable } from 'svelte/store';

export type ScreenName = 'repo-detail' | 'settings' | 'project' | 'dashboard' | 'sync' | 'templates' | 'app_defaults' | 'about' | 'timeline' | 'deploy_report' | 'secret_bundles' | 'global_claude_editor' | 'reports' | 'secret_audit';

export interface ScreenState {
  name: ScreenName;
  params?: Record<string, unknown>;
}

export type ToastType = 'success' | 'error' | 'info' | 'warning';

export interface Toast {
  id: number;
  message: string;
  type: ToastType;
}

let toastCounter = 0;

export const currentScreen = writable<ScreenState>({ name: 'dashboard' });
export const previousScreen = writable<ScreenState>({ name: 'dashboard' });
export const selectedRepoId = writable<number | null>(null);
export const selectedProjectId = writable<number | null>(null);

/** Navigate to screen, remembering previous for back navigation */
export function navigateTo(screen: ScreenName | ScreenState): void {
  const next: ScreenState = typeof screen === 'string' ? { name: screen } : screen;
  currentScreen.update(current => {
    previousScreen.set(current);
    return next;
  });
}

/** Go back to previous screen */
export function goBack(): void {
  previousScreen.update(prev => {
    currentScreen.set(prev);
    return { name: 'dashboard' }; // reset previous
  });
}
export const toasts = writable<Toast[]>([]);

export function addToast(message: string, type: ToastType = 'info'): void {
  const id = ++toastCounter;
  toasts.update((list) => [...list, { id, message, type }]);

  // B-000027: errors/warnings persist until the user dismisses them (they may be
  // long and need reading/copying). Transient success/info auto-dismiss after 5s.
  if (type !== 'error' && type !== 'warning') {
    setTimeout(() => {
      toasts.update((list) => list.filter((t) => t.id !== id));
    }, 5000);
  }
}

export function dismissToast(id: number): void {
  toasts.update((list) => list.filter((t) => t.id !== id));
}

/**
 * v0.22.0 (T-000056): transient store для pre-fill repo-filter в top-level Timeline
 * экран при навигации из RecentActivityFeed deep-link "Все события →".
 * Timeline.svelte читает значение в onMount, применяет к selectedRepos, очищает (`set(null)`).
 * Используется как "one-shot" сигнал — не наблюдаемый long-living state.
 */
export const timelineInitialRepoIds = writable<number[] | null>(null);

/**
 * v1.2.0 (deploy report): one-shot drill-down signal. Set by DeployReport on
 * row click; consumed by RepoDetail (→ activeTab='deploy') then DeployScreen
 * (→ open that env's detail in viewMode='detail'), then cleared. Same one-shot
 * pattern as `timelineInitialRepoIds` above.
 */
export const deployDrillTarget = writable<{ repoId: number; deployEnvId: number } | null>(null);
