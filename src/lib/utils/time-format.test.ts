import { describe, it, expect } from 'vitest';
import { formatRelativeTime } from './time-format';

describe('formatRelativeTime', () => {
  const now = Date.now();
  const ago = (ms: number) => new Date(now - ms).toISOString();

  it('returns "только что" for <1 minute', () => {
    expect(formatRelativeTime(ago(30 * 1000), now)).toBe('только что');
  });

  it('returns "{N} мин назад" for 1-59 minutes', () => {
    expect(formatRelativeTime(ago(2 * 60 * 1000), now)).toBe('2 мин назад');
    expect(formatRelativeTime(ago(59 * 60 * 1000), now)).toBe('59 мин назад');
  });

  it('returns "{N} ч назад" for 1-23 hours', () => {
    expect(formatRelativeTime(ago(2 * 60 * 60 * 1000), now)).toBe('2 ч назад');
    expect(formatRelativeTime(ago(23 * 60 * 60 * 1000), now)).toBe('23 ч назад');
  });

  it('returns "{N} дн назад" for ≥24 hours', () => {
    expect(formatRelativeTime(ago(2 * 24 * 60 * 60 * 1000), now)).toBe('2 дн назад');
    expect(formatRelativeTime(ago(59 * 24 * 60 * 60 * 1000), now)).toBe('59 дн назад');
  });

  it('handles future timestamps gracefully (clamps to "только что")', () => {
    expect(formatRelativeTime(ago(-5000), now)).toBe('только что');
  });

  it('renders English when locale param = "en"', () => {
    expect(formatRelativeTime(ago(30 * 1000), now, 'en')).toBe('just now');
    expect(formatRelativeTime(ago(2 * 60 * 1000), now, 'en')).toBe('2m ago');
    expect(formatRelativeTime(ago(2 * 60 * 60 * 1000), now, 'en')).toBe('2h ago');
    expect(formatRelativeTime(ago(2 * 24 * 60 * 60 * 1000), now, 'en')).toBe('2d ago');
  });
});
