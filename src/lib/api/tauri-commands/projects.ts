import { invoke } from '@tauri-apps/api/core';
import type { Project, Repository } from '$lib/types';

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

// T-000136: toggle the per-project auto-sync opt-in flag.
export async function setProjectAutoSync(projectId: number, enabled: boolean): Promise<void> {
  return invoke<void>('set_project_auto_sync', { projectId, enabled });
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
