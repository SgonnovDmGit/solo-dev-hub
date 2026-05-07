import { describe, it, expect } from 'vitest';
import { parseEnvText } from '../secrets-parser';

describe('parseEnvText', () => {
  it('parses single-line KEY=value', () => {
    const r = parseEnvText('FOO=bar\nBAZ=qux');
    expect(r.errors).toEqual([]);
    expect(r.secrets).toEqual([
      { name: 'FOO', value: 'bar' },
      { name: 'BAZ', value: 'qux' },
    ]);
  });

  it('skips comments and blank lines', () => {
    const r = parseEnvText('# header\n\nFOO=bar\n# trailing');
    expect(r.errors).toEqual([]);
    expect(r.secrets).toEqual([{ name: 'FOO', value: 'bar' }]);
  });

  it('rejects invalid key names', () => {
    const r = parseEnvText('foo-bar=x');
    expect(r.secrets).toEqual([]);
    expect(r.errors.length).toBe(1);
    expect(r.errors[0]).toContain("invalid key 'foo-bar'");
  });

  it('rejects GITHUB_ prefix', () => {
    const r = parseEnvText('GITHUB_TOKEN=x');
    expect(r.secrets).toEqual([]);
    expect(r.errors[0]).toContain('GITHUB_');
  });

  it('parses triple-quoted multi-line value', () => {
    const input = [
      'SSH_KEY="""',
      '-----BEGIN OPENSSH PRIVATE KEY-----',
      'abc',
      'def',
      '-----END OPENSSH PRIVATE KEY-----',
      '"""',
    ].join('\n');
    const r = parseEnvText(input);
    expect(r.errors).toEqual([]);
    expect(r.secrets).toHaveLength(1);
    expect(r.secrets[0].name).toBe('SSH_KEY');
    expect(r.secrets[0].value).toBe(
      '-----BEGIN OPENSSH PRIVATE KEY-----\nabc\ndef\n-----END OPENSSH PRIVATE KEY-----'
    );
  });

  it('parses triple-quoted inline value', () => {
    const r = parseEnvText('KEY="""value"""');
    expect(r.errors).toEqual([]);
    expect(r.secrets).toEqual([{ name: 'KEY', value: 'value' }]);
  });

  it('reports unclosed triple-quoted value', () => {
    const r = parseEnvText('KEY="""\nline1\nline2');
    expect(r.secrets).toEqual([]);
    expect(r.errors[0]).toContain('unclosed triple-quoted');
  });

  it('mixes triple-quoted and single-line', () => {
    const input = [
      'A=1',
      'B="""',
      'multi',
      'line',
      '"""',
      'C=3',
    ].join('\n');
    const r = parseEnvText(input);
    expect(r.errors).toEqual([]);
    expect(r.secrets).toEqual([
      { name: 'A', value: '1' },
      { name: 'B', value: 'multi\nline' },
      { name: 'C', value: '3' },
    ]);
  });

  it('rejects empty triple-quoted value', () => {
    const r = parseEnvText('K="""\n"""');
    expect(r.secrets).toEqual([]);
    expect(r.errors[0]).toContain("empty value for 'K'");
  });
});
