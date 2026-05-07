/**
 * Regression tests for VB-002 and VB-005
 *
 * VB-002: After project deletion, repos should appear in Unassigned (loadAllRepos called)
 * VB-005: Only one server allowed per project
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { get } from 'svelte/store';

// --- Mocks ---

const mockRepos: any[] = [];
let mockProjects: any[] = [];

vi.mock('$lib/api/tauri-commands', () => ({
  listAllRepos: vi.fn(async () => mockRepos),
  listProjects: vi.fn(async () => mockProjects),
  deleteProject: vi.fn(async () => undefined),
  createProject: vi.fn(async (name: string, description?: string) => ({
    id: Date.now(),
    name,
    description: description ?? null,
    created_at: new Date().toISOString(),
  })),
  updateProject: vi.fn(),
  upsertRepository: vi.fn(),
  assignRepository: vi.fn(async (id: number, projectId: number | null, role: string | null) => {
    const repo = mockRepos.find((r) => r.id === id);
    if (!repo) throw new Error('repo not found');
    return { ...repo, project_id: projectId, role };
  }),
}));

vi.mock('../ui', () => ({
  addToast: vi.fn(),
}));

vi.mock('$lib/i18n', () => ({
  t: (key: string) => key,
  tf: (key: string, ...args: any[]) => `${key}: ${args.join(', ')}`,
}));

import { allRepos, loadAllRepos, assignRepo } from '../repos';
import { projects, removeProject } from '../projects';

// Helper to reset store state between tests
function resetStores() {
  allRepos.set([]);
  projects.set([]);
  mockRepos.length = 0;
  mockProjects.length = 0;
}

// -------------------------------------------------------------------
// VB-002: removeProject must call loadAllRepos so unassigned repos reappear
// -------------------------------------------------------------------
describe('VB-002: removeProject reloads repos', () => {
  beforeEach(() => {
    resetStores();
    vi.clearAllMocks();
  });

  it('after removeProject, allRepos is refreshed from DB', async () => {
    // Set up: project 1 with one repo currently assigned
    mockProjects.push({ id: 1, name: 'MyProject', description: null, created_at: '2026-01-01' });
    projects.set([...mockProjects]);

    // Initially the repo appears assigned in the store
    allRepos.set([
      { id: 10, project_id: 1, github_name: 'org/repo', role: null, description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null, deploy_target: null },
    ]);

    // After deletion DB returns the repo with project_id = null (ON DELETE SET NULL)
    mockRepos.push(
      { id: 10, project_id: null, github_name: 'org/repo', role: null, description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null }
    );

    await removeProject(1);

    // allRepos should now reflect DB state (project_id = null)
    const repos = get(allRepos);
    expect(repos).toHaveLength(1);
    expect(repos[0].project_id).toBeNull();
  });

  it('after removeProject, the project is removed from projects store', async () => {
    mockProjects.push({ id: 2, name: 'ToDelete', description: null, created_at: '2026-01-01' });
    projects.set([...mockProjects]);

    await removeProject(2);

    expect(get(projects).find((p) => p.id === 2)).toBeUndefined();
  });
});

// -------------------------------------------------------------------
// VB-005: assignRepo must reject a second server in the same project
// -------------------------------------------------------------------
describe('VB-005: one server per project', () => {
  beforeEach(() => {
    resetStores();
    vi.clearAllMocks();
  });

  it('allows assigning server when project has no server yet', async () => {
    mockRepos.push(
      { id: 1, project_id: 10, github_name: 'org/client', role: 'client', description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null, deploy_target: null },
      { id: 2, project_id: 10, github_name: 'org/server', role: null, description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null }
    );
    allRepos.set([...mockRepos]);

    const result = await assignRepo(2, 10, 'server');
    expect(result).not.toBeNull();
    expect(result?.role).toBe('server');
  });

  it('rejects assigning server when project already has one', async () => {
    mockRepos.push(
      { id: 1, project_id: 10, github_name: 'org/server1', role: 'server', description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null, deploy_target: null },
      { id: 2, project_id: 10, github_name: 'org/server2', role: null, description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null }
    );
    allRepos.set([...mockRepos]);

    const result = await assignRepo(2, 10, 'server');
    expect(result).toBeNull();
  });

  it('allows re-assigning server role to the same repo (update)', async () => {
    mockRepos.push(
      { id: 1, project_id: 10, github_name: 'org/server', role: 'server', description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null }
    );
    allRepos.set([...mockRepos]);

    // Re-assigning server to itself (e.g., after editing description) should succeed
    const result = await assignRepo(1, 10, 'server');
    expect(result).not.toBeNull();
  });

  it('allows server in a different project even if another project has a server', async () => {
    mockRepos.push(
      { id: 1, project_id: 10, github_name: 'org/s1', role: 'server', description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null, deploy_target: null },
      { id: 2, project_id: 20, github_name: 'org/s2', role: null, description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null }
    );
    allRepos.set([...mockRepos]);

    // repo 2 is in project 20, not project 10 — should be allowed
    const result = await assignRepo(2, 20, 'server');
    expect(result).not.toBeNull();
  });

  it('allows server when projectId is null (unassigned)', async () => {
    mockRepos.push(
      { id: 1, project_id: 10, github_name: 'org/server', role: 'server', description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null, deploy_target: null },
      { id: 2, project_id: null, github_name: 'org/other', role: null, description: null,
        github_url: null, language: null, last_pushed_at: null, local_path: null,
        added_at: '2026-01-01', updated_at: '2026-01-01', github_id: null }
    );
    allRepos.set([...mockRepos]);

    // Assigning role=server with projectId=null (unassigned) — no project constraint
    const result = await assignRepo(2, null, 'server');
    expect(result).not.toBeNull();
  });
});
