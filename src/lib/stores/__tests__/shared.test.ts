/**
 * T-000223 review: a refresh requested while another one is running must not
 * be answered with the (possibly stale) running one — exactly one more run
 * follows it.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { get } from 'svelte/store';

type Deferred<T> = { promise: Promise<T>; resolve: (v: T) => void };
function deferred<T>(): Deferred<T> {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

const listRepoSharedStates = vi.fn();
const getSetting = vi.fn();
vi.mock('$lib/api/tauri-commands', () => ({
  listRepoSharedStates: () => listRepoSharedStates(),
  getSetting: (key: string) => getSetting(key),
}));
const loadAllRepos = vi.fn(async () => undefined);
vi.mock('../repos', () => ({ loadAllRepos: () => loadAllRepos() }));
const loadProjects = vi.fn(async () => undefined);
vi.mock('../projects', () => ({ loadProjects: () => loadProjects() }));

function state(id: number) {
  return { repo_id: id, shared_mode: true, shared_role: 'main', shared_status: null, shared_status_detail: null };
}

describe('refreshShared', () => {
  beforeEach(() => {
    vi.resetModules();
    listRepoSharedStates.mockReset();
    getSetting.mockReset();
    getSetting.mockResolvedValue('Owner');
    loadAllRepos.mockClear();
    loadProjects.mockClear();
  });

  it('loads the states and the login', async () => {
    listRepoSharedStates.mockResolvedValue([state(1)]);
    const { refreshShared, sharedStates, githubLogin } = await import('../shared');
    await refreshShared();
    expect([...get(sharedStates).keys()]).toEqual([1]);
    expect(get(githubLogin)).toBe('Owner');
  });

  it('runs once more, not on the stale result, when called during a run', async () => {
    const first = deferred<unknown[]>();
    listRepoSharedStates.mockReturnValueOnce(first.promise).mockResolvedValue([state(2)]);
    const { refreshShared, sharedStates } = await import('../shared');

    const a = refreshShared();
    const b = refreshShared(); // arrives while the first is in flight
    const c = refreshShared(); // shares the single follow-up
    expect(listRepoSharedStates).toHaveBeenCalledTimes(1);

    first.resolve([state(1)]);
    await a;
    // the first result is stale for b and c: they wait for the second run
    await Promise.all([b, c]);
    expect(listRepoSharedStates).toHaveBeenCalledTimes(2);
    expect([...get(sharedStates).keys()]).toEqual([2]);
  });

  it('serves a later call with a new run once everything settled', async () => {
    listRepoSharedStates.mockResolvedValue([state(1)]);
    const { refreshShared } = await import('../shared');
    await refreshShared();
    await refreshShared();
    expect(listRepoSharedStates).toHaveBeenCalledTimes(2);
  });

  it('stays quiet when the backend fails and keeps what is shown', async () => {
    listRepoSharedStates.mockResolvedValueOnce([state(1)]).mockRejectedValueOnce(new Error('x'));
    const { refreshShared, sharedStates } = await import('../shared');
    await refreshShared();
    await refreshShared();
    expect([...get(sharedStates).keys()]).toEqual([1]);
  });

  // T-000239: RepoDetail reloads the structure and the join requests on every
  // refresh — it keys on the state OBJECT, because a sync can change the
  // requests/structure while role/status/mode stay exactly the same. The
  // backend hands out fresh objects each call (Tauri deserializes), so the
  // signal must survive identical field values.
  it('hands out a new state object even when every field is unchanged', async () => {
    listRepoSharedStates.mockImplementation(async () => [state(1)]);
    const { refreshShared, sharedStates } = await import('../shared');
    await refreshShared();
    const first = get(sharedStates).get(1);
    await refreshShared();
    const second = get(sharedStates).get(1);
    expect(second).toEqual(first);
    expect(second).not.toBe(first);
  });
});

describe('refreshAfterSync', () => {
  beforeEach(() => {
    vi.resetModules();
    listRepoSharedStates.mockReset().mockResolvedValue([]);
    getSetting.mockReset().mockResolvedValue(null);
    loadAllRepos.mockClear();
    loadProjects.mockClear();
  });

  it('reloads the shared states, the repos and the projects', async () => {
    const { refreshAfterSync } = await import('../shared');
    await refreshAfterSync();
    expect(listRepoSharedStates).toHaveBeenCalledTimes(1);
    expect(loadAllRepos).toHaveBeenCalledTimes(1);
    expect(loadProjects).toHaveBeenCalledTimes(1);
  });

  it('does not fail when a reload fails', async () => {
    loadAllRepos.mockRejectedValueOnce(new Error('boom'));
    const { refreshAfterSync } = await import('../shared');
    await expect(refreshAfterSync()).resolves.toBeUndefined();
  });
});
