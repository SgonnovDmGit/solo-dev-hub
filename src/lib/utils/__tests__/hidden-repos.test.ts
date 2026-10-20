import { describe, it, expect } from 'vitest';
import {
  computeHiddenRepos,
  parseDismissed,
  dismissKey,
  type FetchedRepo,
  type HiddenRepoInput,
} from '../hidden-repos';

const fetched = (id: number, full_name: string): FetchedRepo => ({ id, full_name });
const db = (
  id: number,
  github_name: string | null,
  github_id: number | null,
  local_path: string | null = null,
): HiddenRepoInput => ({ id, github_name, github_id, local_path });

// The token sees some other repo: an empty list offers nothing by design.
const SEEN = [fetched(99, 'me/visible')];

describe('computeHiddenRepos', () => {
  it('lists a repo whose github id is absent from the fetched list', () => {
    const r = computeHiddenRepos([fetched(1, 'me/a')], [db(10, 'me/a', 1), db(11, 'boss/x', 2)], new Set());
    expect(r.candidates.map((c) => c.repo.id)).toEqual([11]);
    expect(r.show).toBe(true);
  });

  it('offers nothing when the token sees no repo at all', () => {
    const r = computeHiddenRepos([], [db(10, 'me/a', 1), db(11, 'boss/x', 2)], new Set());
    expect(r.candidates).toEqual([]);
    expect(r.show).toBe(false);
  });

  it('does not list a renamed repo: the id still matches', () => {
    const r = computeHiddenRepos([fetched(1, 'me/new-name')], [db(10, 'me/old-name', 1)], new Set());
    expect(r.candidates).toEqual([]);
    expect(r.show).toBe(false);
  });

  it('never lists a local-only repo without a github name', () => {
    const r = computeHiddenRepos(SEEN, [db(10, null, null, 'C:/x')], new Set());
    expect(r.candidates).toEqual([]);
  });

  it('falls back to a case-insensitive name when the repo has no github id', () => {
    const r = computeHiddenRepos([fetched(1, 'Me/App')], [db(10, 'me/app', null), db(11, 'me/gone', null)], new Set());
    expect(r.candidates.map((c) => c.repo.id)).toEqual([11]);
  });

  it('preselects repos without a local folder only', () => {
    const r = computeHiddenRepos(SEEN, [db(10, 'a/x', 1), db(11, 'a/y', 2, 'C:/y')], new Set());
    expect(r.candidates.map((c) => [c.repo.id, c.preselected])).toEqual([
      [10, true],
      [11, false],
    ]);
  });

  it('stays quiet while every candidate was dismissed', () => {
    const rows = [db(10, 'a/x', 1), db(11, 'a/y', 2)];
    const dismissed = new Set(rows.map(dismissKey));
    const r = computeHiddenRepos(SEEN, rows, dismissed);
    expect(r.show).toBe(false);
  });

  it('shows again when a new one appears, listing all with dismissed unchecked', () => {
    const rows = [db(10, 'a/x', 1), db(11, 'a/y', 2), db(12, 'a/z', 3)];
    const dismissed = new Set([dismissKey(rows[0]), dismissKey(rows[1])]);
    const r = computeHiddenRepos(SEEN, rows, dismissed);
    expect(r.show).toBe(true);
    expect(r.candidates.map((c) => [c.repo.id, c.preselected])).toEqual([
      [10, false],
      [11, false],
      [12, true],
    ]);
  });

  it('a removed repo drops out of the dismissed comparison', () => {
    const dismissed = new Set([dismissKey(db(10, 'a/x', 1))]);
    const r = computeHiddenRepos(SEEN, [db(11, 'a/y', 2)], dismissed);
    expect(r.show).toBe(true);
  });
});

describe('parseDismissed', () => {
  it('reads a JSON array and tolerates junk', () => {
    expect(parseDismissed('["id:1","name:a/b"]')).toEqual(new Set(['id:1', 'name:a/b']));
    expect(parseDismissed(null)).toEqual(new Set());
    expect(parseDismissed('nope')).toEqual(new Set());
    expect(parseDismissed('{"a":1}')).toEqual(new Set());
  });
});
