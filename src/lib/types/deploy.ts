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
  // v1.6.0 (T-000134): DB/SSH connection inventory per deploy env, assembled
  // from local data. Sensitive values are withheld (value === null). Both
  // default to [] (never null). Mirrors Rust DeployInventoryField.
  db_fields: DeployInventoryField[];
  ssh_fields: DeployInventoryField[];
}

// v1.6.0 (T-000134): one DB- or SSH-related inventory field. value is null when
// the field is sensitive (withheld) or github-only (no local value). origin is
// where the name was sourced. Mirrors Rust DeployInventoryField (snake_case).
export interface DeployInventoryField {
  name: string;
  value: string | null;
  origin: 'persisted' | 'placeholder' | 'github_only';
  sensitive: boolean;
}

// v1.8.0 (T-000135): one normalized secret-push audit event. Mirrors Rust
// SecretPushEvent (snake_case). source is 'repo' (sync_events, sync_type='secret')
// or 'env' (deploy_events, env_secret_*); action normalized to 'set' | 'delete'.
// deploy_env_id/env_name are null for repo-level events. Values are never carried.
export interface SecretPushEvent {
  source: 'repo' | 'env';
  repository_id: number;
  repo_name: string;
  deploy_env_id: number | null;
  env_name: string | null;
  secret_name: string;
  action: 'set' | 'delete';
  ts: string;
}

// v1.8.0 (T-000140): flattened CSV-export row for the deploy report. The frontend
// builds this from the displayed row (db_name via dbNameOf) and passes it to
// export_deploy_report_csv. Mirrors Rust DeployReportCsvRow (snake_case).
export interface DeployReportCsvRow {
  repo: string;
  environment: string;
  domain: string;
  branch: string;
  image_tag: string;
  db_name: string;
  secrets_count: number;
  updated_at: string;
}
