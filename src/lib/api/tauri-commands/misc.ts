import { invoke } from '@tauri-apps/api/core';


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


export async function readRepoFiles(repoId: number, relPaths: string[]): Promise<(string | null)[]> {
  return invoke<(string | null)[]>('read_repo_files', { repoId, relPaths });
}

export async function readRepoFile(repoId: number, relPath: string): Promise<string | null> {
  return invoke<string | null>('read_repo_file', { repoId, relPath });
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
