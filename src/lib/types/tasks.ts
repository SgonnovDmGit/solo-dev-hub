/** v0.20.0: DB row for `tasks` table. */
export interface Task {
  id: number;
  repository_id: number;
  task_id: string;
  prefix: 'T' | 'F' | 'D';
  description: string;
  effort: number | null;
  priority: string | null;
  status: string | null;
  version: string | null;
  source: 'todo' | 'done';
  created_at: string;
  updated_at: string;
}

/** v0.20.0: SyncTasks report. */
export interface SyncTasksReport {
  imported: number;
  events_emitted: number;
}
