import { t } from '$lib/i18n';

export interface FileBugNote {
  id: string;
  date: string;
  description: string;
  severity: string;
  category: string;
  status: string;
  fix_attempts: number;
  comment: string | null;
}

export interface ReadBugsResult {
  bugs: FileBugNote[];
  warnings: string[];
}

/// v0.16.0: UI-facing bug view (SQLite SoT). Includes `confirmed_at` which is
/// not in MD-format `FileBugNote`. All date fields are YYYY-MM-DD (date portion
/// of the underlying ISO timestamps).
export interface BugView {
  id: string;             // display_id, e.g. "B-000042"
  date: string;           // YYYY-MM-DD (created)
  description: string;
  severity: string;
  category: string;
  status: string;
  fix_attempts: number;
  comment: string | null;
  confirmed_at: string | null;  // YYYY-MM-DD when status=='confirmed'
}

/// v0.16.0: result of lazy MD→DB bug migration for a repo.
export interface MigrationReport {
  imported: number;
  confirmed_archived: number;
  already: boolean;  // true = already migrated, no-op
}

/** Must match DB CHECK constraint on `bugs.category` (see migration v18 in src-tauri/src/db.rs) */
export const BUG_CATEGORIES = [
  'ui_ux', 'ux_flow', 'logic', 'auth', 'database',
  'performance', 'security', 'integration', 'other',
] as const;
export type BugCategory = typeof BUG_CATEGORIES[number];

export const CATEGORY_COLORS: Record<BugCategory, string> = {
  ui_ux: '#8b5cf6',
  ux_flow: '#a855f7',
  logic: '#3b82f6',
  auth: '#06b6d4',
  database: '#14b8a6',
  security: '#f43f5e',
  performance: '#f59e0b',
  integration: '#0ea5e9',
  other: '#6b7280',
};

export function getCategoryLabel(category: string): string {
  return t(`category.${category}` as any);
}
