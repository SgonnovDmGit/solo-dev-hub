import { describe, it, expect } from 'vitest';
import { normalizeSecretValue } from '../secrets-crypto';

describe('normalizeSecretValue', () => {
  it('trims single-line values', () => {
    expect(normalizeSecretValue('  foo  ')).toBe('foo');
    expect(normalizeSecretValue('/home/user/app.env')).toBe('/home/user/app.env');
  });

  it('strips CR from Windows-pasted single-line', () => {
    expect(normalizeSecretValue('abc\r')).toBe('abc');
  });

  it('converts CRLF to LF in multi-line, preserves body, ensures trailing LF', () => {
    const pem = '-----BEGIN OPENSSH PRIVATE KEY-----\r\nabc\r\ndef\r\n-----END OPENSSH PRIVATE KEY-----\r\n';
    expect(normalizeSecretValue(pem)).toBe(
      '-----BEGIN OPENSSH PRIVATE KEY-----\nabc\ndef\n-----END OPENSSH PRIVATE KEY-----\n',
    );
  });

  it('adds trailing LF for multi-line without one', () => {
    const pem = '-----BEGIN KEY-----\nbody\n-----END KEY-----';
    expect(normalizeSecretValue(pem)).toBe('-----BEGIN KEY-----\nbody\n-----END KEY-----\n');
  });

  it('strips leading/trailing blank lines in multi-line', () => {
    expect(normalizeSecretValue('\n\n  line1\nline2\n\n\n')).toBe('line1\nline2\n');
  });

  it('preserves inner blank lines', () => {
    expect(normalizeSecretValue('a\n\nb')).toBe('a\n\nb\n');
  });
});
