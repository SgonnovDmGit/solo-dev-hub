// T-000233: repos kept in the DB that the current GitHub token no longer sees
// (e.g. a helper's install once used the owner's token). Pure logic only; the
// sidebar owns the dialog and the removal.

export interface FetchedRepo {
  id: number;
  full_name: string;
}

export interface HiddenRepoInput {
  id: number;
  github_name: string | null;
  github_id: number | null;
  local_path: string | null;
}

export interface HiddenCandidate<T extends HiddenRepoInput = HiddenRepoInput> {
  repo: T;
  /** Pre-checked in the dialog: no local folder and not dismissed earlier. */
  preselected: boolean;
}

export interface HiddenRepos<T extends HiddenRepoInput = HiddenRepoInput> {
  candidates: HiddenCandidate<T>[];
  /** True when at least one candidate was not dismissed before. */
  show: boolean;
}

/** Stable identity of a repo for the dismissed set: github id, else lowercase name. */
export function dismissKey(r: Pick<HiddenRepoInput, 'github_id' | 'github_name'>): string {
  return r.github_id != null ? `id:${r.github_id}` : `name:${(r.github_name ?? '').toLowerCase()}`;
}

export function parseDismissed(raw: string | null | undefined): Set<string> {
  if (!raw) return new Set();
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? new Set(v.filter((x): x is string => typeof x === 'string')) : new Set();
  } catch {
    return new Set();
  }
}

/**
 * `fetched` must be the complete list the token sees; call only after a fully
 * successful fetch. A repo is hidden when it has a github_name and its
 * github_id (no id: its name, case-insensitive) is absent from `fetched`.
 * An empty `fetched` offers nothing: a token that sees no repo at all (a
 * fine-grained token with none selected, SSO not authorised) says more about
 * the token than about the repos.
 */
export function computeHiddenRepos<T extends HiddenRepoInput>(
  fetched: FetchedRepo[],
  dbRepos: T[],
  dismissed: Set<string>,
): HiddenRepos<T> {
  if (fetched.length === 0) return { candidates: [], show: false };
  const ids = new Set(fetched.map((f) => f.id));
  const names = new Set(fetched.map((f) => f.full_name.toLowerCase()));
  const candidates: HiddenCandidate<T>[] = [];
  let show = false;
  for (const repo of dbRepos) {
    if (!repo.github_name) continue;
    const visible =
      repo.github_id != null ? ids.has(repo.github_id) : names.has(repo.github_name.toLowerCase());
    if (visible) continue;
    const wasDismissed = dismissed.has(dismissKey(repo));
    if (!wasDismissed) show = true;
    candidates.push({ repo, preselected: !wasDismissed && !repo.local_path });
  }
  return { candidates, show };
}
