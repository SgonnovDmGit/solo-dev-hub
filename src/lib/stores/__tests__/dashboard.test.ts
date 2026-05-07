import { describe, it, expect } from 'vitest';
import { resolvePeriod, resolveComparePeriod } from '../dashboard';
import type { Period } from '$lib/types';

describe('resolvePeriod', () => {
  it('Thu mid-week Неделя returns Пн..Чт', () => {
    const ref = new Date(2026, 3, 24, 12, 0, 0); // April 24 Thu
    const p = resolvePeriod('week', ref);
    expect(p.start).toBe('2026-04-20');
    expect(p.end).toBe('2026-04-24');
  });

  it('Monday week gives single-day period', () => {
    const ref = new Date(2026, 3, 20, 12, 0, 0);
    const p = resolvePeriod('week', ref);
    expect(p.start).toBe('2026-04-20');
    expect(p.end).toBe('2026-04-20');
  });

  it('Месяц returns 1st..today', () => {
    const ref = new Date(2026, 3, 24, 12, 0, 0);
    const p = resolvePeriod('month', ref);
    expect(p.start).toBe('2026-04-01');
    expect(p.end).toBe('2026-04-24');
  });

  it('Квартал Q2 returns Apr 1..today', () => {
    const ref = new Date(2026, 3, 24, 12, 0, 0);
    const p = resolvePeriod('quarter', ref);
    expect(p.start).toBe('2026-04-01');
    expect(p.end).toBe('2026-04-24');
  });
});

describe('resolveComparePeriod', () => {
  it('Чт Неделя Mon..Thu -> prev-week Mon..Thu', () => {
    const cur: Period = { start: '2026-04-20', end: '2026-04-24' };
    const c = resolveComparePeriod('week', cur);
    expect(c).not.toBeNull();
    expect(c!.start).toBe('2026-04-13');
    expect(c!.end).toBe('2026-04-17');
  });

  it('Пн d=0 -> compare null', () => {
    const cur: Period = { start: '2026-04-20', end: '2026-04-20' };
    expect(resolveComparePeriod('week', cur)).toBeNull();
  });

  it('Custom always null', () => {
    const cur: Period = { start: '2026-04-01', end: '2026-04-15' };
    expect(resolveComparePeriod('custom', cur)).toBeNull();
  });

  it('End-of-March Месяц clamps to Feb 28 (non-leap 2026)', () => {
    const cur: Period = { start: '2026-03-01', end: '2026-03-31' };
    const c = resolveComparePeriod('month', cur);
    expect(c).not.toBeNull();
    expect(c!.start).toBe('2026-02-01');
    expect(c!.end).toBe('2026-02-28');
  });

  it('Q1 compares to prev year Q4', () => {
    const cur: Period = { start: '2026-01-01', end: '2026-01-15' };
    const c = resolveComparePeriod('quarter', cur);
    expect(c).not.toBeNull();
    expect(c!.start).toBe('2025-10-01');
    expect(c!.end).toBe('2025-10-15');
  });

  it('Q2 compares to Q1', () => {
    const cur: Period = { start: '2026-04-01', end: '2026-04-24' };
    const c = resolveComparePeriod('quarter', cur);
    expect(c).not.toBeNull();
    expect(c!.start).toBe('2026-01-01');
    expect(c!.end).toBe('2026-01-24');
  });

  it('Full prev week when current is complete Mon..Sun', () => {
    const cur: Period = { start: '2026-04-20', end: '2026-04-26' };
    const c = resolveComparePeriod('week', cur);
    expect(c).not.toBeNull();
    expect(c!.start).toBe('2026-04-13');
    expect(c!.end).toBe('2026-04-19');
  });

  it('d=7 (8 calendar days) still resolves', () => {
    const cur: Period = { start: '2026-04-20', end: '2026-04-27' };
    const c = resolveComparePeriod('week', cur);
    expect(c).not.toBeNull();
    expect(c!.start).toBe('2026-04-13');
    expect(c!.end).toBe('2026-04-20');
  });
});
