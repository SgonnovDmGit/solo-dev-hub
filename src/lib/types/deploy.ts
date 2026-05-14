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
