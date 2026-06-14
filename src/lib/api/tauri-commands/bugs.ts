import { invoke } from '@tauri-apps/api/core';
import type { FileBugNote, ReadBugsResult, BugView, MigrationReport } from '$lib/types';


// ── File-based Bug commands ──────────────────────────────────────────────────

export async function readBugsFromFile(filePath: string): Promise<ReadBugsResult> {
  return invoke<ReadBugsResult>('read_bugs_from_file', { filePath });
}

export async function writeBugsToFile(
  filePath: string, repoRoot: string, bugs: FileBugNote[]
): Promise<void> {
  return invoke<void>('write_bugs_to_file', { filePath, repoRoot, bugs });
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
