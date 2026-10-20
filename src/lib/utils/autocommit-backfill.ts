import type { RepoSharedState } from '$lib/types';

/**
 * F-000044 (R5): may the portfolio-wide sweep set the auto-commit branch of a
 * repo to `dev`? Never for a repo where this install is a contributor or a
 * not-yet-approved helper, and never for a repo owned by someone else — `dev`
 * there is the owner's branch, the helper must pick their own.
 *
 * When this install's GitHub login is unknown ownership cannot be judged, so
 * only repos without a shared role keep the old behaviour.
 */
export function shouldBackfillAutocommit(
  githubName: string | null | undefined,
  shared: Pick<RepoSharedState, 'shared_role'> | null | undefined,
  login: string | null | undefined,
): boolean {
  if (!githubName || !githubName.includes('/')) return false;
  const role = shared?.shared_role ?? null;
  if (role === 'contributor' || role === 'pending') return false;
  // The owner's own shared repo: this install is main, the owner check is moot
  // (e.g. a repo in an organisation the owner belongs to).
  if (role === 'main') return true;
  const me = (login ?? '').trim();
  // Unknown login: ownership cannot be judged, so a repo without a shared role
  // keeps the old behaviour (it is not known to be somebody else's).
  if (me === '') return true;
  const owner = githubName.split('/')[0] ?? '';
  return owner.toLowerCase() === me.toLowerCase();
}
