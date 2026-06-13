import { describe, it, expect } from 'vitest';
import { mergeBundleValues, mergeBundleIntoEnvText } from './bundle-apply';
import { parseEnvText } from './secrets-parser';

const items = [
  { secret_name: 'SSH_HOST', value: '1.2.3.4' },
  { secret_name: 'DB_PASSWORD', value: 'p@ss word' },
];

describe('mergeBundleValues', () => {
  it('overlays bundle onto current map, bundle wins on conflict', () => {
    const out = mergeBundleValues({ SSH_HOST: 'old', KEEP: 'me' }, items);
    expect(out).toEqual({ SSH_HOST: '1.2.3.4', DB_PASSWORD: 'p@ss word', KEEP: 'me' });
  });

  it('does not mutate the input', () => {
    const current = { A: '1' };
    mergeBundleValues(current, items);
    expect(current).toEqual({ A: '1' });
  });
});

describe('mergeBundleIntoEnvText', () => {
  it('appends new entries as KEY=value lines', () => {
    const out = mergeBundleIntoEnvText('EXISTING=keep', [
      { secret_name: 'NEW', value: 'val' },
    ]);
    expect(out).toContain('EXISTING=keep');
    expect(out).toContain('NEW=val');
  });

  it('replaces an existing key in place (bundle wins)', () => {
    const out = mergeBundleIntoEnvText('SSH_HOST=old', [
      { secret_name: 'SSH_HOST', value: 'new' },
    ]);
    expect(out).toMatch(/SSH_HOST=new/);
    expect(out).not.toMatch(/SSH_HOST=old/);
  });

  it('quotes values with spaces', () => {
    const out = mergeBundleIntoEnvText('', [{ secret_name: 'K', value: 'a b' }]);
    expect(out).toContain('K="a b"');
  });

  it('wraps multi-line values in triple quotes', () => {
    const out = mergeBundleIntoEnvText('', [{ secret_name: 'KEY', value: 'l1\nl2' }]);
    expect(out).toContain('KEY="""l1\nl2"""');
  });
});

describe('round-trip through parseEnvText', () => {
  const cases: Array<[string, string]> = [
    ['PLAIN', '1.2.3.4'],
    ['SPACED', 'p@ss word'],
    ['HASH', 'foo#bar'],
    ['MULTILINE', 'line1\nline2'],
    ['QUOTE', 'a"b"c'],
    ['TRIPLEQUOTE_MULTILINE', 'l1\na"""b\nl3'],  // the bug case
    ['BACKSLASH_N', 'literal\\nbackslash'],       // literal backslash-n, must NOT become newline
    ['TAB', 'a\tb'],
  ];
  for (const [name, value] of cases) {
    it(`round-trips ${name}`, () => {
      const text = mergeBundleIntoEnvText('', [{ secret_name: name, value }]);
      const parsed = parseEnvText(text);
      expect(parsed.errors).toEqual([]);
      const got = parsed.secrets.find((s) => s.name === name)?.value;
      expect(got).toBe(value);
    });
  }
});
