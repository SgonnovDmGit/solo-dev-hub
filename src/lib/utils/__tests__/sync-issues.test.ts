/**
 * T-000198: sync issues arrive as codes and are translated here, one line per
 * issue. The raw detail (OS / git text) is never translated.
 */
import { describe, it, expect } from 'vitest';
import { formatSyncIssue, formatSyncIssues } from '../sync-issues';
import type { SyncIssue } from '$lib/types';

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
