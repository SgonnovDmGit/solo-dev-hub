import { invoke } from '@tauri-apps/api/core';
import type { Project, Repository, FileBugNote, ReadBugsResult, BugView, MigrationReport, StatsSummary, SyncResult, RequirementInfo, RepoRename, TemplateFile, TemplateLanguage, RenderedFile, WriteResult, DashboardFilter, DashboardData, DeployEnvironment, DeploySecret, CreateDeployEnvironmentArgs, UpdateDeployEnvironmentArgs, DeploySecretRole, ActivityEvent, Task, SyncTasksReport, TimelineFilter, ProjectGraph, UntrackReport, GitignoredListing } from '$lib/types';

// ── Project commands ──────────────────────────────────────────────────────────

export async function createProject(name: string, description?: string, projectType: 'standard' | 'microservice' = 'standard'): Promise<Project> {
  return invoke<Project>('create_project', { name, description: description ?? null, projectType });
}

export async function listProjects(): Promise<Project[]> {
  return invoke<Project[]>('list_projects');
}

export async function updateProject(
  id: number,
  name: string,
  description?: string
): Promise<Project> {
  return invoke<Project>('update_project', { id, name, description: description ?? null });
}

export async function deleteProject(id: number): Promise<void> {
  return invoke<void>('delete_project', { id });
}

// ── Repository commands ───────────────────────────────────────────────────────

export async function createLocalRepository(
  localPath: string,
  displayName: string,
  projectId: number | null = null,
  role: string | null = null,
): Promise<Repository> {
  return invoke<Repository>('create_local_repository', { localPath, displayName, projectId, role });
}

export type UpsertRepoOutcome =
  | { kind: 'inserted'; repo: Repository }
  | { kind: 'merged'; repo: Repository; merged_with_local_id: number; local_path: string }
  | {
      kind: 'ambiguous';
      github_name: string;
      github_url: string | null;
      description: string | null;
      language: string | null;
      last_pushed_at: string | null;
      github_id: number | null;
      candidates: Repository[];
    };

export async function upsertRepository(
  githubName: string,
  githubUrl: string | null,
  description: string | null,
  language: string | null,
  lastPushedAt: string | null,
  githubId: number | null = null,
): Promise<UpsertRepoOutcome> {
  return invoke<UpsertRepoOutcome>('upsert_repository', {
    githubName,
    githubUrl,
    description,
    language,
    lastPushedAt,
    githubId,
  });
}

export async function resolveMergeWithLocal(
  localId: number,
  githubName: string,
  githubUrl: string | null,
  description: string | null,
  language: string | null,
  lastPushedAt: string | null,
  githubId: number | null = null,
): Promise<Repository> {
  return invoke<Repository>('resolve_merge_with_local', {
    localId,
    githubName,
    githubUrl,
    description,
    language,
    lastPushedAt,
    githubId,
  });
}

export async function forceInsertGithubRepo(
  githubName: string,
  githubUrl: string | null,
  description: string | null,
  language: string | null,
  lastPushedAt: string | null,
  githubId: number | null = null,
): Promise<Repository> {
  return invoke<Repository>('force_insert_github_repo', {
    githubName,
    githubUrl,
    description,
    language,
    lastPushedAt,
    githubId,
  });
}

export async function assignRepository(
  id: number,
  projectId?: number | null,
  role?: string | null
): Promise<Repository> {
  return invoke<Repository>('assign_repository', {
    id,
    projectId: projectId ?? null,
    role: role ?? null,
  });
}

export async function listReposByProject(projectId?: number | null): Promise<Repository[]> {
  return invoke<Repository[]>('list_repos_by_project', { projectId: projectId ?? null });
}

export async function listAllRepos(): Promise<Repository[]> {
  return invoke<Repository[]>('list_all_repos');
}

export async function getRepository(id: number): Promise<Repository> {
  return invoke<Repository>('get_repository', { id });
}

export async function getRepositoryByName(githubName: string): Promise<Repository> {
  return invoke<Repository>('get_repository_by_name', { githubName });
}

// ── File-based Bug commands ──────────────────────────────────────────────────

export async function readBugsFromFile(filePath: string): Promise<ReadBugsResult> {
  return invoke<ReadBugsResult>('read_bugs_from_file', { filePath });
}

export async function writeBugsToFile(
  filePath: string, repoRoot: string, bugs: FileBugNote[]
): Promise<void> {
  return invoke<void>('write_bugs_to_file', { filePath, repoRoot, bugs });
}

export async function setRepoLocalPath(id: number, localPath: string | null): Promise<Repository> {
  return invoke<Repository>('set_repo_local_path', { id, localPath });
}

export async function updateRepoDescription(repoId: number, newDescription: string): Promise<Repository> {
  return invoke<Repository>('update_repo_description', { repoId, newDescription });
}

export async function deleteRepository(id: number, clearGitLocal: boolean, localPath: string | null): Promise<void> {
  return invoke<void>('delete_repository', { id, clearGitLocal, localPath });
}

// ── F-000041: untrack gitignored files ──────────────────────────────────────

export async function checkGitAvailableForRepo(repositoryId: number): Promise<boolean> {
  return invoke<boolean>('check_git_available_for_repo', { repositoryId });
}

export async function listGitignoredTracked(repositoryId: number): Promise<GitignoredListing> {
  return invoke<GitignoredListing>('list_gitignored_tracked', { repositoryId });
}

export async function untrackFiles(repositoryId: number, files: string[]): Promise<UntrackReport> {
  return invoke<UntrackReport>('untrack_files', { repositoryId, files });
}

export async function scanWorkspaceForRepos(workspaceRoot: string, githubNames: string[]): Promise<Record<string, string>> {
  return invoke<Record<string, string>>('scan_workspace_for_repos', { workspaceRoot, githubNames });
}

// ── Microservice connection commands (F-012) ────────────────────────────────

export async function connectMicroservice(projectId: number, microserviceProjectId: number): Promise<void> {
  return invoke<void>('connect_microservice', { projectId, microserviceProjectId });
}

export async function disconnectMicroservice(projectId: number, microserviceProjectId: number): Promise<void> {
  return invoke<void>('disconnect_microservice', { projectId, microserviceProjectId });
}

export async function listProjectMicroservices(projectId: number): Promise<number[]> {
  return invoke<number[]>('list_project_microservices', { projectId });
}

export async function listMicroserviceProjects(): Promise<Project[]> {
  return invoke<Project[]>('list_microservice_projects');
}

export async function listParentsOfMicroservice(msProjectId: number): Promise<Project[]> {
  return invoke<Project[]>('list_parents_of_microservice', { msProjectId });
}

export async function updateProjectType(id: number, newType: 'standard' | 'microservice'): Promise<Project> {
  return invoke<Project>('update_project_type', { id, newType });
}

export async function serverRepoOfMicroservice(msProjectId: number): Promise<Repository> {
  return invoke<Repository>('server_repo_of_microservice', { msProjectId });
}

// ── PAT / Keyring commands ────────────────────────────────────────────────────

export async function storePat(token: string): Promise<void> {
  return invoke<void>('store_pat', { token });
}

export async function getPat(): Promise<string | null> {
  return invoke<string | null>('get_pat');
}

export async function deletePat(): Promise<void> {
  return invoke<void>('delete_pat');
}

// ── Settings commands ─────────────────────────────────────────────────────────

export async function getSetting(key: string): Promise<string | null> {
  return invoke<string | null>('get_setting', { key });
}

export async function setSetting(key: string, value: string): Promise<void> {
  return invoke<void>('set_setting', { key, value });
}

// ── Bug commands (v0.16.0, SQLite SoT) ────────────────────────────────────────

export async function ensureBugsMigrated(repoId: number): Promise<MigrationReport> {
  return invoke<MigrationReport>('ensure_bugs_migrated', { repoId });
}

export async function reconcileBugsForRepo(repoId: number): Promise<void> {
  return invoke<void>('reconcile_bugs_for_repo', { repoId });
}

export interface ReconcileAllReport {
  repos_scanned: number;
  errors: string[];
}

/** Portfolio-wide MD→DB reconcile for bugs + tasks (no cross-repo file copies). */
export async function reconcileAllProjects(): Promise<ReconcileAllReport> {
  return invoke<ReconcileAllReport>('reconcile_all_projects');
}

export async function readBugsFromDb(repoId: number, includeConfirmed: boolean): Promise<BugView[]> {
  return invoke<BugView[]>('read_bugs_from_db', { repoId, includeConfirmed });
}

export async function countConfirmedBugs(repoId: number): Promise<number> {
  return invoke<number>('count_confirmed_bugs', { repoId });
}

export async function createBug(repoId: number, description: string, severity: string, category: string): Promise<BugView> {
  return invoke<BugView>('create_bug', { repoId, description, severity, category });
}

/// Update user-owned fields. Omit a field to leave it unchanged.
/// For `comment`, pass empty string `""` to clear (DB NULL); pass text to set.
export async function updateBugFields(
  repoId: number,
  displayId: string,
  fields: {
    description?: string;
    severity?: string;
    category?: string;
    comment?: string;
  },
): Promise<BugView> {
  return invoke<BugView>('update_bug_fields', { repoId, displayId, ...fields });
}

export async function deleteBug(repoId: number, displayId: string): Promise<void> {
  return invoke<void>('delete_bug', { repoId, displayId });
}

export async function resolveBug(repoId: number, displayId: string): Promise<BugView> {
  return invoke<BugView>('resolve_bug', { repoId, displayId });
}

export async function rejectBug(repoId: number, displayId: string): Promise<BugView> {
  return invoke<BugView>('reject_bug', { repoId, displayId });
}

// T-000130: reopen a confirmed-or-rejected bug back to testing (undo verdict).
export async function reopenBug(repoId: number, displayId: string): Promise<BugView> {
  return invoke<BugView>('reopen_bug', { repoId, displayId });
}

// ── Stats / Graph summaries (v0.22.0 lifetime-only API; live-computed) ───────

export async function getRepoStatsSummary(repositoryId: number): Promise<StatsSummary> {
  return invoke<StatsSummary>('get_repo_stats_summary', { repositoryId });
}

export async function getProjectStatsSummary(projectId: number): Promise<StatsSummary> {
  return invoke<StatsSummary>('get_project_stats_summary', { projectId });
}

export async function getProjectGraph(projectId: number): Promise<ProjectGraph> {
  return invoke<ProjectGraph>('get_project_graph', { projectId });
}

// ── Requirements sync commands ──────────────────────────────────────────────

export interface SyncGlobalClaudeResult {
  path: string;
  synced_at: string;
}

export async function syncGlobalClaudeMd(): Promise<SyncGlobalClaudeResult> {
  return await invoke<SyncGlobalClaudeResult>('sync_global_claude_md');
}

export async function initDocsForRepo(repoId: number): Promise<string[]> {
  return invoke<string[]>('init_docs_for_repo', { repoId });
}

export async function syncProject(projectId: number): Promise<SyncResult> {
  return invoke<SyncResult>('sync_project', { projectId });
}

export async function listProjectRequirements(projectId: number): Promise<RequirementInfo[]> {
  return invoke<RequirementInfo[]>('list_project_requirements', { projectId });
}

export async function confirmRequirement(
  projectId: number,
  filename: string,
  sourceRepoId: number,
  targetRepoId: number,
): Promise<void> {
  return invoke<void>('confirm_requirement', { projectId, filename, sourceRepoId, targetRepoId });
}

// ── Rename log (F-033) ────────────────────────────────────────────────────────

export async function listRepoRenames(): Promise<RepoRename[]> {
  return invoke<RepoRename[]>('list_rename_history');
}

export async function listRenamesForRepo(repoId: number): Promise<RepoRename[]> {
  return invoke<RepoRename[]>('list_renames_for_repo', { repoId });
}

// ── Templates (0.6.0) ─────────────────────────────────────────────────────────

export async function listTemplateLanguages(): Promise<TemplateLanguage[]> {
  return invoke<TemplateLanguage[]>('list_template_languages');
}

export async function listTemplateFiles(languageKey: string): Promise<TemplateFile[]> {
  return invoke<TemplateFile[]>('list_template_files', { languageKey });
}

export async function getTemplateFile(languageKey: string, fileName: string): Promise<TemplateFile | null> {
  return invoke<TemplateFile | null>('get_template_file', { languageKey, fileName });
}

export async function saveTemplateFile(languageKey: string, fileName: string, content: string): Promise<void> {
  return invoke<void>('save_template_file', { languageKey, fileName, content });
}

export async function resetTemplateFile(languageKey: string, fileName: string): Promise<void> {
  return invoke<void>('reset_template_file', { languageKey, fileName });
}

// ── Deploy (0.7.0) ────────────────────────────────────────────────────────────

export async function setDeployTarget(id: number, target: string | null): Promise<Repository> {
  return invoke<Repository>('set_deploy_target', { id, target });
}

// T-000103 Task 1: repo-wide deploy config (placeholder values shared across
// envs — e.g. GO_VERSION baked into the single Dockerfile).
export async function getRepoDeployConfig(repoId: number): Promise<Record<string, string>> {
  return invoke<Record<string, string>>('get_repo_deploy_config', { repoId });
}

export async function setRepoDeployConfig(
  repoId: number,
  config: Record<string, string>,
): Promise<void> {
  return invoke<void>('set_repo_deploy_config', { repoId, config });
}


export async function readRepoFiles(repoId: number, relPaths: string[]): Promise<(string | null)[]> {
  return invoke<(string | null)[]>('read_repo_files', { repoId, relPaths });
}

export async function readRepoFile(repoId: number, relPath: string): Promise<string | null> {
  return invoke<string | null>('read_repo_file', { repoId, relPath });
}

// ── F-025 Manual ordering ─────────────────────────────────────────────────────

export async function reorderProject(id: number, direction: 'up' | 'down'): Promise<void> {
  return invoke<void>('reorder_project', { id, direction });
}

export async function reorderRepo(repoId: number, direction: 'up' | 'down'): Promise<void> {
  return invoke<void>('reorder_repo', { repoId, direction });
}

export async function rebalanceRepoGroup(orderedIds: number[]): Promise<void> {
  return invoke<void>('rebalance_repo_group', { orderedIds });
}

export async function rebalanceProjects(orderedIds: number[]): Promise<void> {
  return invoke<void>('rebalance_projects', { orderedIds });
}

export async function autoSortAll(): Promise<void> {
  return invoke<void>('auto_sort_all');
}

// ── F-021 Docs viewer ─────────────────────────────────────────────────────────

export interface TodoTask {
  id: string;
  description: string;
  effort: string;
  priority: string;
  status: string;
  created_at: string;  // YYYY-MM-DD; "" if 5-field legacy
}

export interface DoneTask {
  id: string;
  description: string;
  date: string;
  version: string;
}

export interface ReadTodoResult {
  tasks: TodoTask[];
  warnings: string[];
}

export interface ReadDoneResult {
  tasks: DoneTask[];
  warnings: string[];
}

export async function readRepoTodo(repoId: number): Promise<ReadTodoResult> {
  return invoke<ReadTodoResult>('read_repo_todo', { repoId });
}

export async function readRepoDone(repoId: number): Promise<ReadDoneResult> {
  return invoke<ReadDoneResult>('read_repo_done', { repoId });
}

export async function writeDeployFiles(deployEnvId: number, repoId: number, localPath: string, files: RenderedFile[]): Promise<WriteResult> {
  return invoke<WriteResult>('write_deploy_files', { deployEnvId, repoId, localPath, files });
}

// ── v0.17.0 Dashboard ──────────────────────────────────────────────────────

export async function readDashboard(filter: DashboardFilter): Promise<DashboardData> {
  return invoke<DashboardData>('read_dashboard', { filter });
}

export async function parseDoneEntriesInPeriod(
  repoId: number,
  start: string,
  end: string,
): Promise<Array<[string, number]>> {
  return invoke<Array<[string, number]>>('parse_done_entries_in_period_cmd', {
    repoId, start, end,
  });
}

// ── v0.19.0: Activity feed ────────────────────────────────────────────────────

export async function readRecentActivity(limit: number): Promise<ActivityEvent[]> {
  return invoke<ActivityEvent[]>('read_recent_activity', { limit });
}

// ── v0.20.0: Tasks + Timeline ─────────────────────────────────────────────────

export async function syncTasksForRepo(repoId: number): Promise<SyncTasksReport> {
  return invoke<SyncTasksReport>('sync_tasks_for_repo_cmd', { repoId });
}

export async function readTasksFromDb(repoId: number): Promise<Task[]> {
  return invoke<Task[]>('read_tasks_from_db', { repoId });
}

export async function readDoneFromDb(repoId: number): Promise<Task[]> {
  return invoke<Task[]>('read_done_from_db', { repoId });
}

export async function readTimeline(filter: TimelineFilter, offset: number, limit: number): Promise<ActivityEvent[]> {
  return invoke<ActivityEvent[]>('read_timeline', { filter, offset, limit });
}

export async function recordSecretEvent(repoId: number, action: 'set' | 'delete', secretName: string): Promise<void> {
  return invoke('record_secret_event', { repoId, action, secretName });
}

export async function recordDeploySecretEvent(deployEnvId: number, repoId: number, action: 'env_secret_set' | 'env_secret_delete', secretName: string): Promise<void> {
  return invoke('record_deploy_secret_event', { deployEnvId, repoId, action, secretName });
}

// ── v0.18.0: Multi-environment deploy ─────────────────────────────────────────

export const listDeployEnvironments = (repoId: number) =>
  invoke<DeployEnvironment[]>('list_deploy_environments', { repoId });

export const getDeployEnvironment = (id: number) =>
  invoke<DeployEnvironment | null>('get_deploy_environment', { id });

export const createDeployEnvironment = (args: CreateDeployEnvironmentArgs) =>
  invoke<DeployEnvironment>('create_deploy_environment', { args });

export const cloneDeployEnvironment = (sourceId: number, newName: string) =>
  invoke<DeployEnvironment>('clone_deploy_environment', { sourceId, newName });

export const updateDeployEnvironment = (args: UpdateDeployEnvironmentArgs) =>
  invoke<DeployEnvironment>('update_deploy_environment', { args });

export const deleteDeployEnvironment = (id: number) =>
  invoke<void>('delete_deploy_environment', { id });

export const reorderDeployEnvironments = (repoId: number, orderedIds: number[]) =>
  invoke<void>('reorder_deploy_environments', { repoId, orderedIds });

export const listDeploySecrets = (deployEnvId: number) =>
  invoke<DeploySecret[]>('list_deploy_secrets', { deployEnvId });

export const upsertDeploySecret = (
  deployEnvId: number, secretName: string,
  role: DeploySecretRole | null, included: boolean, overrideEnabled: boolean,
) => invoke<void>('upsert_deploy_secret', {
  deployEnvId, secretName, role, included, overrideEnabled,
});

export const deleteDeploySecret = (deployEnvId: number, secretName: string) =>
  invoke<void>('delete_deploy_secret', { deployEnvId, secretName });

export const ensureDeploySecretsPopulated = (deployEnvId: number, repoSecretNames: string[]) =>
  invoke<void>('ensure_deploy_secrets_populated', { deployEnvId, repoSecretNames });

export const registerRepoSecretInDeploys = (repoId: number, secretName: string) =>
  invoke<void>('register_repo_secret_in_deploys', { repoId, secretName });

export const renderDeployFilesForEnv = (deployEnvId: number) =>
  invoke<RenderedFile[]>('render_deploy_files_for_env', { deployEnvId });
