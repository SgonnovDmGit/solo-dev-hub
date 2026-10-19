import { describe, it, expect } from 'vitest';
import { parseBugRefusal, bugRefusalMessage } from '../bug-refusal';
import { translations } from '$lib/i18n/translations';

describe('parseBugRefusal', () => {
  it('maps every refusal code of the backend', () => {
    expect(parseBugRefusal('band_refused:pending')).toEqual({ key: 'bugs.refused.pending', args: [] });
    expect(parseBugRefusal('band_refused:no_login')).toEqual({ key: 'bugs.refused.noLogin', args: [] });
    expect(parseBugRefusal('band_refused:no_structure')).toEqual({ key: 'bugs.refused.noStructure', args: [] });
    expect(parseBugRefusal('band_refused:no_range')).toEqual({ key: 'bugs.refused.noRange', args: [] });
  });

  it('carries the reason of an untrusted structure, colons included', () => {
    expect(parseBugRefusal('band_refused:untrusted:.sdh/project.json: main is not the repo owner')).toEqual({
      key: 'bugs.refused.untrusted',
      args: ['.sdh/project.json: main is not the repo owner'],
    });
  });

  it('formats a full band as B-numbers', () => {
    expect(parseBugRefusal('band_refused:full:500000-599999')).toEqual({
      key: 'bugs.refused.full',
      args: ['500000', '599999'],
    });
    expect(parseBugRefusal('band_refused:full:1-499999')!.args).toEqual(['000001', '499999']);
    expect(parseBugRefusal('band_refused:full:garbage')!.key).toBe('bugs.refused.unknown');
  });

  it('keeps an unknown code visible', () => {
    expect(parseBugRefusal('band_refused:something_new')).toEqual({
      key: 'bugs.refused.unknown',
      args: ['something_new'],
    });
  });

  it('is null for any other error', () => {
    expect(parseBugRefusal('database is locked')).toBeNull();
    expect(parseBugRefusal(new Error('boom'))).toBeNull();
    expect(parseBugRefusal(null)).toBeNull();
  });

  it('survives a wrapper around the string', () => {
    expect(parseBugRefusal('Error: band_refused:pending')!.key).toBe('bugs.refused.pending');
  });
});

describe('bugRefusalMessage', () => {
  const translate = (key: string, ...args: string[]) =>
    args.reduce((s, a, i) => s.replace(`{${i}}`, a), (translations.ru as Record<string, string>)[key] ?? key);

  it('translates a refusal and ignores other errors', () => {
    expect(bugRefusalMessage('band_refused:full:500000-599999', translate as any)).toContain('B-500000..B-599999');
    expect(bugRefusalMessage('no such table', translate as any)).toBeNull();
  });

  it('has a Russian and an English text for every refusal key', () => {
    for (const key of [
      'bugs.refused.pending',
      'bugs.refused.noLogin',
      'bugs.refused.untrusted',
      'bugs.refused.noStructure',
      'bugs.refused.noRange',
      'bugs.refused.full',
      'bugs.refused.unknown',
    ]) {
      expect((translations.ru as Record<string, string>)[key], key).toBeTruthy();
      expect((translations.en as Record<string, string>)[key], key).toBeTruthy();
    }
  });
});
