/**
 * T-000174: the rules screen shows canon → current as a collapsed line diff.
 * Unchanged runs longer than 2 * context collapse into one gap row.
 */
import { describe, it, expect } from 'vitest';
import { buildRuleDiff, type DiffRow } from '../rule-diff';

function lines(n: number, change?: (i: number) => string | undefined): string {
  return Array.from({ length: n }, (_, k) => `line ${k + 1}`)
    .map((text, k) => (change ? change(k + 1) ?? text : text))
    .join('\n');
}

describe('buildRuleDiff', () => {
  it('is empty for identical texts', () => {
    const text = lines(20);
    expect(buildRuleDiff(text, text)).toEqual({ rows: [], added: 0, removed: 0 });
  });

  it('shows one changed line in the middle with gaps on both sides', () => {
    const canon = lines(20);
    const current = lines(20, (n) => (n === 10 ? 'changed' : undefined));
    const diff = buildRuleDiff(canon, current);

    expect(diff.added).toBe(1);
    expect(diff.removed).toBe(1);
    expect(diff.rows).toEqual<DiffRow[]>([
      { kind: 'gap', hidden: 6 },
      { kind: 'ctx', oldNo: 7, newNo: 7, text: 'line 7' },
      { kind: 'ctx', oldNo: 8, newNo: 8, text: 'line 8' },
      { kind: 'ctx', oldNo: 9, newNo: 9, text: 'line 9' },
      { kind: 'del', oldNo: 10, text: 'line 10' },
      { kind: 'add', newNo: 10, text: 'changed' },
      { kind: 'ctx', oldNo: 11, newNo: 11, text: 'line 11' },
      { kind: 'ctx', oldNo: 12, newNo: 12, text: 'line 12' },
      { kind: 'ctx', oldNo: 13, newNo: 13, text: 'line 13' },
      { kind: 'gap', hidden: 7 },
    ]);
  });

  it('keeps both ends of a long run between two changes', () => {
    const canon = lines(20);
    const current = lines(20, (n) => (n === 5 || n === 15 ? `changed ${n}` : undefined));
    const rows = buildRuleDiff(canon, current).rows;

    // lines 6..14 are unchanged between the two edits: 9 lines, 3 kept on each side
    expect(rows).toContainEqual({ kind: 'gap', hidden: 3 });
    expect(rows.flatMap((r) => (r.kind === 'ctx' ? [r.oldNo] : []))).toEqual([
      1, 2, 3, 4, 6, 7, 8, 12, 13, 14, 16, 17, 18, 19, 20,
    ]);
  });

  it('shows no leading gap when the first line changes', () => {
    const canon = lines(20);
    const current = lines(20, (n) => (n === 1 ? 'first' : undefined));
    const rows = buildRuleDiff(canon, current).rows;

    expect(rows[0]).toEqual({ kind: 'del', oldNo: 1, text: 'line 1' });
    expect(rows[1]).toEqual({ kind: 'add', newNo: 1, text: 'first' });
    expect(rows[rows.length - 1]).toEqual({ kind: 'gap', hidden: 16 });
  });

  it('treats CRLF and LF of the same text as equal', () => {
    const lf = 'a\nb\nc\n';
    const crlf = 'a\r\nb\r\nc\r\n';
    expect(buildRuleDiff(lf, crlf)).toEqual({ rows: [], added: 0, removed: 0 });
  });

  it('does not count a trailing newline as a line', () => {
    expect(buildRuleDiff('a\n', 'a')).toEqual({ rows: [], added: 0, removed: 0 });
  });

  it('counts added and removed rows and numbers both sides', () => {
    const diff = buildRuleDiff('a\nb\nc\n', 'a\nB\nc\nd\n');

    expect(diff.added).toBe(2);
    expect(diff.removed).toBe(1);
    expect(diff.rows).toEqual<DiffRow[]>([
      { kind: 'ctx', oldNo: 1, newNo: 1, text: 'a' },
      { kind: 'del', oldNo: 2, text: 'b' },
      { kind: 'add', newNo: 2, text: 'B' },
      { kind: 'ctx', oldNo: 3, newNo: 3, text: 'c' },
      { kind: 'add', newNo: 4, text: 'd' },
    ]);
  });
});
