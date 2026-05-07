import { writable } from 'svelte/store';
import type { Project } from '$lib/types';
import {
  createProject as tauriCreateProject,
  listProjects as tauriListProjects,
  updateProject as tauriUpdateProject,
  deleteProject as tauriDeleteProject,
} from '$lib/api/tauri-commands';
import { addToast } from './ui';
import { t, tf } from '$lib/i18n';
import { loadAllRepos } from './repos';

export const projects = writable<Project[]>([]);

export async function loadProjects(): Promise<void> {
  try {
    const data = await tauriListProjects();
    projects.set(data);
  } catch (err) {
    addToast(tf('toast.failedToLoadProjects', String(err)), 'error');
  }
}

export async function addProject(
  name: string,
  description?: string,
  projectType: 'standard' | 'microservice' = 'standard',
): Promise<Project | null> {
  try {
    const project = await tauriCreateProject(name, description, projectType);
    // F-025: Rust sets sort_order = MIN-10 on INSERT (project goes to top).
    // Reload list to get fresh ordering from DB (sort_order-aware).
    await loadProjects();
    addToast(tf('toast.projectCreated', project.name), 'success');
    return project;
  } catch (err) {
    addToast(tf('toast.failedToCreateProject', String(err)), 'error');
    return null;
  }
}

export async function editProject(
  id: number,
  name: string,
  description?: string
): Promise<Project | null> {
  try {
    const updated = await tauriUpdateProject(id, name, description);
    projects.update((list) => list.map((p) => (p.id === id ? updated : p)));
    addToast(tf('toast.projectUpdated', updated.name), 'success');
    return updated;
  } catch (err) {
    addToast(tf('toast.failedToUpdateProject', String(err)), 'error');
    return null;
  }
}

export async function removeProject(id: number): Promise<boolean> {
  try {
    await tauriDeleteProject(id);
    projects.update((list) => list.filter((p) => p.id !== id));
    // VB-002: reload repos so unassigned repos become visible again
    await loadAllRepos();
    addToast(t('toast.projectDeleted'), 'success');
    return true;
  } catch (err) {
    addToast(tf('toast.failedToDeleteProject', String(err)), 'error');
    return false;
  }
}
