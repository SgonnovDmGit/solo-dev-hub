import { describe, it, expect } from 'vitest';
import { compareSemVer } from '../semver';

describe('compareSemVer', () => {
  it('returns 0 for equal versions', () => {
    expect(compareSemVer('v0.10.0', 'v0.10.0')).toBe(0);
    expect(compareSemVer('0.10.0', 'v0.10.0')).toBe(0);
  });

  it('sorts by major, then minor, then patch', () => {
    expect(compareSemVer('v0.9.0', 'v0.10.0')).toBeLessThan(0);
    expect(compareSemVer('v1.0.0', 'v0.99.99')).toBeGreaterThan(0);
    expect(compareSemVer('v1.2.3', 'v1.2.10')).toBeLessThan(0);
  });

  it('treats vX.Y.Z and X.Y.Z as equivalent', () => {
    expect(compareSemVer('0.31.0', 'v0.31.0')).toBe(0);
  });

  it('sorts pre-release before release of same triple', () => {
    expect(compareSemVer('v1.0.0-rc1', 'v1.0.0')).toBeLessThan(0);
    expect(compareSemVer('v1.0.0', 'v1.0.0-rc1')).toBeGreaterThan(0);
  });

  it('sorts pre-release tags lexicographically when same triple', () => {
    expect(compareSemVer('v1.0.0-alpha', 'v1.0.0-beta')).toBeLessThan(0);
  });

  it('empty / null / undefined sort to the end', () => {
    expect(compareSemVer('', 'v0.1.0')).toBeGreaterThan(0);
    expect(compareSemVer('v0.1.0', '')).toBeLessThan(0);
    expect(compareSemVer(null, 'v0.1.0')).toBeGreaterThan(0);
    expect(compareSemVer(undefined, undefined)).toBe(0);
  });

  it('semver values sort before non-semver values', () => {
    expect(compareSemVer('v0.1.0', 'tbd')).toBeLessThan(0);
    expect(compareSemVer('tbd', 'v0.1.0')).toBeGreaterThan(0);
  });

  it('non-semver values fall back to localeCompare', () => {
    expect(compareSemVer('alpha', 'beta')).toBeLessThan(0);
    expect(compareSemVer('beta', 'alpha')).toBeGreaterThan(0);
  });

  it('Array.sort produces SemVer order, not lexicographic', () => {
    const versions = ['v0.10.0', 'v0.9.0', 'v0.2.0', 'v1.0.0', 'v0.30.1'];
    const sorted = [...versions].sort(compareSemVer);
    expect(sorted).toEqual(['v0.2.0', 'v0.9.0', 'v0.10.0', 'v0.30.1', 'v1.0.0']);
  });
});
