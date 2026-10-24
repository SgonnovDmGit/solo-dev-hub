/** What the repo screen's Sync button runs for a repo. */
export type SyncAction = { kind: 'project'; projectId: number } | { kind: 'probe' };

/**
 * A repo in a project syncs that project; a repo without one (a contributor's
 * repo not yet linked) is probed for the owner's structure instead.
 */
export function chooseSyncAction(projectId: number | null | undefined): SyncAction {
  return projectId != null ? { kind: 'project', projectId } : { kind: 'probe' };
}
