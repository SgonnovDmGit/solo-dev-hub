// v0.18.0: Multi-environment deploy

export type DeploySecretRole = 'build' | 'deploy' | 'runtime';

export interface DeployEnvironment {
  id: number;
  repository_id: number;
  name: string;
  workflow_name: string;
  image_tag: string;
  compose_service: string;
  domain: string;
  deploy_branch: string;
  sort_order: number;
  extras: Record<string, string>;
  updated_at: string;
}

export interface DeploySecret {
  id: number;
  deploy_env_id: number;
  secret_name: string;
  role: DeploySecretRole | null;
  included: boolean;
  override_enabled: boolean;
  sort_order: number;
}

// v1.6.0 (F-000043): one decrypted deploy secret name+value. Persisted
// encrypted-at-rest server-side (mirrors Rust DeploySecretValue, snake_case).
export interface DeploySecretValue {
  secret_name: string;
  value: string;
}

export interface CreateDeployEnvironmentArgs {
  repository_id: number;
  name: string;
  workflow_name: string;
  image_tag: string;
  compose_service: string;
  domain: string;
  deploy_branch: string;
  extras?: Record<string, string>;
}

export interface UpdateDeployEnvironmentArgs {
  id: number;
  workflow_name: string;
  image_tag: string;
  compose_service: string;
  domain: string;
  deploy_branch: string;
  extras: Record<string, string>;
}

// v1.2.0 (deploy report): one row of the portfolio-wide deploy inventory.
// Mirrors Rust DeployReportRow (snake_case). project_id/project_name are null
// for orphan repos (no project assigned). repo_name is the display form.
export interface DeployReportRow {
  deploy_env_id: number;
  repository_id: number;
  repo_name: string;
  project_id: number | null;
  project_name: string | null;
  env_name: string;
  domain: string;
  deploy_branch: string;
  image_tag: string;
  secrets_count: number;
  updated_at: string;
}
