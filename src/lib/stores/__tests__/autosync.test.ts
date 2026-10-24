/**
 * T-000207 follow-up: auto-sync used to discard the sync result, so a rule
 * uptake was invisible and a lost hand-back marker was cleared with nobody told.
 * It now reports those (rare) rule findings; other errors stay swallowed.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { writable } from 'svelte/store';

const syncProject = vi.fn();
const setSharedAdminVerified = vi.fn();
vi.mock('$lib/api/tauri-commands', () => ({
  syncProject: (id: number) => syncProject(id),
  probeSharedRepos: vi.fn(),
  setSharedAdminVerified: (...args: unknown[]) => setSharedAdminVerified(...args),
}));

const isRepoAdmin = vi.fn();
vi.mock('$lib/api/github', () => ({
  isRepoAdmin: (...args: unknown[]) => isRepoAdmin(...args),
}));

vi.mock('../settings', () => ({
  autoSyncEnabled: writable(true),
  autoSyncIntervalMin: writable(15),
  autoSyncLastAt: writable(null),
  saveAutoSyncLastAt: vi.fn(async () => undefined),
  pat: writable('token'),
}));

vi.mock('../projects', () => ({
  projects: writable([{ id: 7, auto_sync_enabled: true }]),
}));

const refreshAfterSync = vi.fn(async () => undefined);
vi.mock('../shared', () => ({
  refreshAfterSync: () => refreshAfterSync(),
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
    admin_checks: [],
    ...extra,
  };
}

describe('auto-sync rule findings', () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    syncProject.mockReset();
    addToast.mockReset();
    refreshAfterSync.mockClear();
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
    expect(addToast).toHaveBeenCalledWith('toast.rulesTaken: sdh-retro', 'info', { persist: true });
    expect(addToast).toHaveBeenCalledWith('toast.rulesLost: sdh-debugging (beta)', 'warning');
  });

  it('reloads repos, projects and shared states after the sync', async () => {
    syncProject.mockResolvedValue(result({}));
    await runOneTick();
    expect(refreshAfterSync).toHaveBeenCalledTimes(1);
  });

  it('stays quiet on ordinary errors and when no rule changed', async () => {
    syncProject.mockResolvedValue(result({ errors: [{ code: 'missing_counterpart', subject: 'P', target: 'server' }] }));
    await runOneTick();
    expect(syncProject).toHaveBeenCalledWith(7);
    expect(addToast).not.toHaveBeenCalled();
  });
});

// T-000227: the background timer goes through the same administrator
// verification as a manual sync.
describe('auto-sync administrator checks', () => {
  const check = { repo: 'Acme-Org/web', login: 'boss' };

  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    syncProject.mockReset();
    setSharedAdminVerified.mockReset();
    isRepoAdmin.mockReset();
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

  it('verifies the asked logins and syncs once more when one just became an admin', async () => {
    syncProject
      .mockResolvedValueOnce(result({ admin_checks: [check], copied: 1 }))
      .mockResolvedValueOnce(result({ admin_checks: [check], copied: 2 }));
    isRepoAdmin.mockResolvedValue(true);
    setSharedAdminVerified.mockResolvedValue(true);
    await runOneTick();
    expect(isRepoAdmin).toHaveBeenCalledWith('token', 'Acme-Org', 'web', 'boss');
    expect(setSharedAdminVerified).toHaveBeenCalledWith('Acme-Org/web', 'boss', true);
    // exactly one re-run, even though the second result asks again
    expect(syncProject).toHaveBeenCalledTimes(2);
  });

  it('does not sync again when nobody newly became an admin', async () => {
    syncProject.mockResolvedValue(result({ admin_checks: [check] }));
    isRepoAdmin.mockResolvedValue(true);
    setSharedAdminVerified.mockResolvedValue(false);
    await runOneTick();
    expect(setSharedAdminVerified).toHaveBeenCalledTimes(1);
    expect(syncProject).toHaveBeenCalledTimes(1);
  });

  it('writes nothing when the GitHub call fails', async () => {
    syncProject.mockResolvedValue(result({ admin_checks: [check] }));
    isRepoAdmin.mockRejectedValue(new Error('403'));
    await runOneTick();
    expect(setSharedAdminVerified).not.toHaveBeenCalled();
    expect(syncProject).toHaveBeenCalledTimes(1);
  });
});
