/**
 * T-000207 follow-up: auto-sync used to discard the sync result, so a rule
 * uptake was invisible and a lost hand-back marker was cleared with nobody told.
 * It now reports those (rare) rule findings; other errors stay swallowed.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { writable } from 'svelte/store';

const syncProject = vi.fn();
vi.mock('$lib/api/tauri-commands', () => ({
  syncProject: (id: number) => syncProject(id),
}));

vi.mock('../settings', () => ({
  autoSyncEnabled: writable(true),
  autoSyncIntervalMin: writable(15),
  autoSyncLastAt: writable(null),
  saveAutoSyncLastAt: vi.fn(async () => undefined),
}));

vi.mock('../projects', () => ({
  projects: writable([{ id: 7, auto_sync_enabled: true }]),
}));

const addToast = vi.fn();
vi.mock('$lib/stores/ui', () => ({
  addToast: (...args: unknown[]) => addToast(...args),
}));

vi.mock('$lib/i18n', () => ({
  tf: (key: string, ...args: unknown[]) => `${key}: ${args.join(', ')}`,
}));

function result(extra: Record<string, unknown>) {
  return {
    copied: 0,
    responses: 0,
    migrated: 0,
    errors: [],
    auto_closed: [],
    committed: [],
    rules_taken: [],
    rules_lost: [],
    ...extra,
  };
}

describe('auto-sync rule findings', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    syncProject.mockReset();
    addToast.mockReset();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  async function runOneTick() {
    const { startAutoSyncTimer } = await import('../autosync');
    startAutoSyncTimer();
    await vi.advanceTimersByTimeAsync(60_000);
  }

  it('reports taken and lost rules from a background sync', async () => {
    syncProject.mockResolvedValue(
      result({ rules_taken: ['sdh-retro'], rules_lost: ['sdh-debugging (beta)'] }),
    );
    await runOneTick();
    expect(syncProject).toHaveBeenCalledWith(7);
    expect(addToast).toHaveBeenCalledWith('toast.rulesTaken: sdh-retro', 'info');
    expect(addToast).toHaveBeenCalledWith('toast.rulesLost: sdh-debugging (beta)', 'warning');
  });

  it('stays quiet on ordinary errors and when no rule changed', async () => {
    syncProject.mockResolvedValue(result({ errors: [{ code: 'missing_counterpart', subject: 'P', target: 'server' }] }));
    await runOneTick();
    expect(syncProject).toHaveBeenCalledWith(7);
    expect(addToast).not.toHaveBeenCalled();
  });
});
