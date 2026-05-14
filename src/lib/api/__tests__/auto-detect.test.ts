import { describe, it, expect } from 'vitest';
import { runAutoDetect, type AutoDetectSpec } from '../auto-detect';

function readerOf(files: Record<string, string>) {
  return async (path: string) => (path in files ? files[path] : null);
}

describe('runAutoDetect', () => {
  it('capture mode: returns group [1] when regex matches', async () => {
    const spec: AutoDetectSpec = { source: 'file', path: '.nvmrc', regex: '^v?(\\d+)' };
    const read = readerOf({ '.nvmrc': 'v22.11.0\n' });
    expect(await runAutoDetect(spec, read)).toBe('22');
  });

  it('capture mode: returns whole match when regex has no group', async () => {
    const spec: AutoDetectSpec = { source: 'file', path: '.nvmrc', regex: '\\d+' };
    const read = readerOf({ '.nvmrc': 'node 22 alpine' });
    expect(await runAutoDetect(spec, read)).toBe('22');
  });

  it('predicate mode: returns value_if_match on match, ignores capture group', async () => {
    const spec: AutoDetectSpec = {
      source: 'file',
      path: 'package.json',
      regex: '"@inlang/paraglide-js"',
      value_if_match: 'npm run paraglide:compile',
    };
    const read = readerOf({
      'package.json': '{"dependencies":{"@inlang/paraglide-js":"^1.0.0"}}',
    });
    expect(await runAutoDetect(spec, read)).toBe('npm run paraglide:compile');
  });

  it('predicate mode: returns null when regex does not match', async () => {
    const spec: AutoDetectSpec = {
      source: 'file',
      path: 'package.json',
      regex: '"@inlang/paraglide-js"',
      value_if_match: 'npm run paraglide:compile',
    };
    const read = readerOf({ 'package.json': '{"dependencies":{"react":"18"}}' });
    expect(await runAutoDetect(spec, read)).toBeNull();
  });

  it('multi-path: tries paths in order, returns first hit', async () => {
    const spec: AutoDetectSpec = {
      source: 'file',
      path: ['vite.config.js', 'vite.config.ts', 'svelte.config.js'],
      regex: "outDir[\\s:]*['\"]([^'\"]+)['\"]",
    };
    const read = readerOf({
      // vite.config.js absent; vite.config.ts hits.
      'vite.config.ts': "export default {\n  build: { outDir: 'build' }\n}",
    });
    expect(await runAutoDetect(spec, read)).toBe('build');
  });

  it('multi-path: returns null when no path matches', async () => {
    const spec: AutoDetectSpec = {
      source: 'file',
      path: ['vite.config.js', 'svelte.config.js'],
      regex: 'outDir',
    };
    const read = readerOf({});
    expect(await runAutoDetect(spec, read)).toBeNull();
  });

  it('returns null for null spec', async () => {
    expect(await runAutoDetect(null, async () => '')).toBeNull();
  });

  it('returns null for non-file source', async () => {
    const spec: any = { source: 'env', regex: '\\d+' };
    expect(await runAutoDetect(spec, async () => '22')).toBeNull();
  });

  it('returns null for malformed regex', async () => {
    const spec: AutoDetectSpec = { source: 'file', path: 'x', regex: '[invalid(' };
    expect(await runAutoDetect(spec, async () => 'x')).toBeNull();
  });

  it('returns null when path is empty array', async () => {
    const spec: AutoDetectSpec = { source: 'file', path: [], regex: '.' };
    expect(await runAutoDetect(spec, async () => 'x')).toBeNull();
  });
});
