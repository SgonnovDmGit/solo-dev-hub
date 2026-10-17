/**
 * T-000198: sync issues arrive as codes and are translated here, one line per
 * issue. The raw detail (OS / git text) is never translated.
 */
import { describe, it, expect } from 'vitest';
import { formatSyncIssue, formatSyncIssues } from '../sync-issues';
import { SYNC_ISSUE_CODES, SYNC_ISSUE_LOOKUPS, type SyncIssue } from '$lib/types';
import { translations } from '$lib/i18n/translations';

describe('formatSyncIssue', () => {
  const copy: SyncIssue = {
    code: 'copy_failed',
    subject: 'REQ-001_x.md',
    target: 'backend',
    detail: 'Access is denied. (os error 5)',
  };

  it('translates a known code in Russian and keeps the raw detail', () => {
    expect(formatSyncIssue(copy, 'ru')).toBe(
      'Не удалось скопировать REQ-001_x.md → backend: Access is denied. (os error 5)',
    );
  });

  it('translates a known code in English', () => {
    expect(formatSyncIssue(copy, 'en')).toBe(
      'Could not copy REQ-001_x.md → backend: Access is denied. (os error 5)',
    );
  });

  it('places the detail where the template asks for it', () => {
    const collision: SyncIssue = {
      code: 'rename_collision',
      subject: 'old',
      target: 'new',
      detail: 'client-requirements/',
    };
    expect(formatSyncIssue(collision, 'ru')).toBe(
      'Переименование old/ → new/: обе папки уже есть в client-requirements/ — объедините вручную',
    );
  });

  it('shows a missing role by its label', () => {
    const issue: SyncIssue = { code: 'missing_counterpart', subject: 'Lensa', target: 'server' };
    expect(formatSyncIssue(issue, 'ru')).toBe('В проекте Lensa нет репозитория с ролью «Сервер»');
  });

  it('translates what a lookup was reading, with or without a subject', () => {
    const renames: SyncIssue = {
      code: 'lookup_failed',
      subject: 'alpha',
      target: 'renames',
      detail: 'database is locked',
    };
    expect(formatSyncIssue(renames, 'ru')).toBe(
      'Внутренняя ошибка чтения: история переименований alpha: database is locked',
    );
    const home: SyncIssue = { code: 'lookup_failed', subject: '', target: 'home_dir' };
    expect(formatSyncIssue(home, 'en')).toBe('Internal read failed: home directory');
    const unknown: SyncIssue = { code: 'lookup_failed', subject: 'x', target: 'new_token' };
    expect(formatSyncIssue(unknown, 'en')).toBe('Internal read failed: new_token x');
  });

  it('reports a stale flat file as left in place, without prose in detail', () => {
    const issue: SyncIssue = { code: 'migration_left_in_place', subject: 'REQ-009.md' };
    expect(formatSyncIssue(issue, 'ru')).toBe(
      'Файл REQ-009.md оставлен в server-requirements/: у родителей нет такой же копии — возможно, устарел',
    );
  });

  it('appends a target the template does not place, in parentheses', () => {
    const issue: SyncIssue = {
      code: 'peer_channel_failed',
      subject: 'PACT-001_x.md',
      target: 'alpha, beta',
      detail: 'Access is denied. (os error 5)',
    };
    expect(formatSyncIssue(issue, 'ru')).toBe(
      'Канал между равными: сбой на PACT-001_x.md (alpha, beta): Access is denied. (os error 5)',
    );
  });

  it('never renders an empty target as "()"', () => {
    const issue: SyncIssue = { code: 'peer_channel_failed', subject: 'alpha', target: '' };
    expect(formatSyncIssue(issue, 'en')).toBe('Peer channel failed at alpha');
  });

  it('has a ru and en line for every code and lookup token', () => {
    for (const loc of ['ru', 'en'] as const) {
      const table = translations[loc] as Record<string, string>;
      for (const code of SYNC_ISSUE_CODES) {
        expect(table[`syncIssue.${code}`], `${loc} syncIssue.${code}`).toBeTruthy();
      }
      for (const token of SYNC_ISSUE_LOOKUPS) {
        expect(table[`syncIssueLookup.${token}`], `${loc} syncIssueLookup.${token}`).toBeTruthy();
      }
    }
  });

  it('falls back to English from the raw fields for an unknown code', () => {
    const issue = {
      code: 'from_a_newer_backend',
      subject: 'alpha',
      target: 'beta',
      detail: 'boom',
    } as unknown as SyncIssue;
    expect(formatSyncIssue(issue, 'ru')).toBe('from_a_newer_backend: alpha → beta: boom');
  });

  it('renders one line per issue', () => {
    const held: SyncIssue = { code: 'rule_held', subject: 'alpha', target: 'sdh-retro' };
    expect(formatSyncIssues([copy, held], 'en').split('\n')).toEqual([
      'Could not copy REQ-001_x.md → backend: Access is denied. (os error 5)',
      'alpha: sdh-retro.md copy left as is — unresolved hand-back',
    ]);
  });
});
