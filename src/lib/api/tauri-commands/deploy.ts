import { invoke } from '@tauri-apps/api/core';
import type { Repository, RenderedFile, WriteResult, DeployEnvironment, DeployReportRow, DeploySecret, DeploySecretValue, CreateDeployEnvironmentArgs, UpdateDeployEnvironmentArgs, DeploySecretRole, Task, SecretBundle, SecretBundleItemValue } from '$lib/types';


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

export async function writeDeployFiles(deployEnvId: number, repoId: number, localPath: string, files: RenderedFile[]): Promise<WriteResult> {
  return invoke<WriteResult>('write_deploy_files', { deployEnvId, repoId, localPath, files });
}

// ── v0.18.0: Multi-environment deploy ─────────────────────────────────────────

export const listDeployEnvironments = (repoId: number) =>
  invoke<DeployEnvironment[]>('list_deploy_environments', { repoId });

export const listDeployReport = () =>
  invoke<DeployReportRow[]>('list_deploy_report', {});

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

// ── v1.6.0 (F-000043): persisted deploy secret values ─────────────────────────

export const setDeploySecretValue = (deployEnvId: number, secretName: string, value: string) =>
  invoke<void>('set_deploy_secret_value', { deployEnvId, secretName, value });

export const deleteDeploySecretValue = (deployEnvId: number, secretName: string) =>
  invoke<void>('delete_deploy_secret_value', { deployEnvId, secretName });

export const getDeploySecretValues = (deployEnvId: number) =>
  invoke<DeploySecretValue[]>('get_deploy_secret_values', { deployEnvId });

export const renderDeployFilesForEnv = (deployEnvId: number) =>
  invoke<RenderedFile[]>('render_deploy_files_for_env', { deployEnvId });

// ── v1.3.0: Secret bundles ────────────────────────────────────────────────────

export const listSecretBundles = () =>
  invoke<SecretBundle[]>('list_secret_bundles', {});

export const createSecretBundle = (name: string, description: string) =>
  invoke<number>('create_secret_bundle', { name, description });

export const renameSecretBundle = (id: number, name: string, description: string) =>
  invoke<void>('rename_secret_bundle', { id, name, description });

export const deleteSecretBundle = (id: number) =>
  invoke<void>('delete_secret_bundle', { id });

export const upsertBundleItem = (bundleId: number, secretName: string, value: string) =>
  invoke<void>('upsert_bundle_item', { bundleId, secretName, value });

export const deleteBundleItem = (itemId: number) =>
  invoke<void>('delete_bundle_item', { itemId });

export const getBundleDecrypted = (bundleId: number) =>
  invoke<SecretBundleItemValue[]>('get_bundle_decrypted', { bundleId });
