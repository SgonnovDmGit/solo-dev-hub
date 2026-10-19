import { describe, it, expect } from 'vitest';
import { shouldBackfillAutocommit } from '../autocommit-backfill';

const shared = (shared_role: 'main' | 'contributor' | 'pending' | null) => ({ shared_role });

describe('shouldBackfillAutocommit', () => {
  it('backfills an own repo without a shared role', () => {
    expect(shouldBackfillAutocommit('me/app', undefined, 'me')).toBe(true);
    expect(shouldBackfillAutocommit('me/app', shared(null), 'me')).toBe(true);
  });

  it('compares the owner case-insensitively', () => {
    expect(shouldBackfillAutocommit('Me/app', undefined, 'mE')).toBe(true);
  });

  it('backfills an own repo where this install is main', () => {
    expect(shouldBackfillAutocommit('me/app', shared('main'), 'me')).toBe(true);
  });

  it('role main wins over the owner check', () => {
    expect(shouldBackfillAutocommit('org/app', shared('main'), 'me')).toBe(true);
  });

  it('skips contributor and pending repos', () => {
    expect(shouldBackfillAutocommit('me/app', shared('contributor'), 'me')).toBe(false);
    expect(shouldBackfillAutocommit('me/app', shared('pending'), 'me')).toBe(false);
    expect(shouldBackfillAutocommit('me/app', shared('pending'), null)).toBe(false);
  });

  it("skips repos owned by someone else", () => {
    expect(shouldBackfillAutocommit('owner/app', undefined, 'me')).toBe(false);
    expect(shouldBackfillAutocommit('owner/app', shared(null), 'me')).toBe(false);
  });

  it('with an unknown login keeps the old behaviour only for repos without a shared role', () => {
    for (const login of [null, undefined, '', '  ']) {
      expect(shouldBackfillAutocommit('owner/app', undefined, login)).toBe(true);
      expect(shouldBackfillAutocommit('owner/app', shared('main'), login)).toBe(true);
      expect(shouldBackfillAutocommit('owner/app', shared('contributor'), login)).toBe(false);
    }
  });

  it('skips repos without a github full name', () => {
    expect(shouldBackfillAutocommit(null, undefined, 'me')).toBe(false);
    expect(shouldBackfillAutocommit('local-only', undefined, 'me')).toBe(false);
  });
});
