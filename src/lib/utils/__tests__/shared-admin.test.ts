import { describe, it, expect } from 'vitest';
import {
  uniqueAdminChecks,
  shouldRerunAfterAdminChecks,
  splitFullName,
  needsOwnAdminCheck,
  mergeRerun,
} from '../shared-admin';
import type { SyncResult } from '$lib/types';

const check = (repo: string, login: string) => ({ repo, login });

describe('shouldRerunAfterAdminChecks', () => {
  it('re-runs only when a verification flipped someone to admin', () => {
    expect(shouldRerunAfterAdminChecks([])).toBe(false);
    expect(shouldRerunAfterAdminChecks([{ check: check('o/r', 'a'), newlyAdmin: false }])).toBe(false);
    expect(
      shouldRerunAfterAdminChecks([
        { check: check('o/r', 'a'), newlyAdmin: false },
        { check: check('o/s', 'b'), newlyAdmin: true },
      ]),
    ).toBe(true);
  });
});

describe('uniqueAdminChecks', () => {
  it('drops repeats regardless of case and tolerates a missing list', () => {
    expect(uniqueAdminChecks(undefined)).toEqual([]);
    expect(
      uniqueAdminChecks([check('Org/Web', 'Boss'), check('org/web', 'boss'), check('org/web', 'other')]),
    ).toEqual([check('Org/Web', 'Boss'), check('org/web', 'other')]);
  });
});

describe('needsOwnAdminCheck', () => {
  it('is false for the owner and for unknowns, true for somebody else', () => {
    expect(needsOwnAdminCheck('Owner/api', 'owner')).toBe(false);
    expect(needsOwnAdminCheck('Acme-Org/api', 'boss')).toBe(true);
    expect(needsOwnAdminCheck('Acme-Org/api', null)).toBe(false);
    expect(needsOwnAdminCheck(null, 'boss')).toBe(false);
    expect(needsOwnAdminCheck('local-only', 'boss')).toBe(false);
  });
});

describe('splitFullName', () => {
  it('accepts only owner/name', () => {
    expect(splitFullName('a/b')).toEqual({ owner: 'a', repo: 'b' });
    expect(splitFullName('a')).toBeNull();
    expect(splitFullName('a/b/c')).toBeNull();
    expect(splitFullName('/b')).toBeNull();
  });
});

describe('mergeRerun', () => {
  const base = (over: Partial<SyncResult>): SyncResult => ({
    copied: 0,
    responses: 0,
    migrated: 0,
    errors: [],
    auto_closed: [],
    committed: [],
    rules_taken: [],
    rules_lost: [],
    admin_checks: [],
    ...over,
  });

  it('adds the counters, joins the lists and keeps the second run issues and checks', () => {
    const issue = { code: 'copy_failed', subject: 'x' } as SyncResult['errors'][number];
    const merged = mergeRerun(
      base({ copied: 1, committed: ['a'], errors: [issue], admin_checks: [check('o/r', 'b')] }),
      base({ copied: 2, responses: 1, committed: ['a', 'c'] }),
    );
    expect(merged.copied).toBe(3);
    expect(merged.responses).toBe(1);
    expect(merged.committed).toEqual(['a', 'c']);
    expect(merged.errors).toEqual([]);
    expect(merged.admin_checks).toEqual([]);
  });
});
