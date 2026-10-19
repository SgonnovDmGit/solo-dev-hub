import { diffArrays } from 'diff';

export type DiffRow =
  | { kind: 'ctx'; oldNo: number; newNo: number; text: string }
  | { kind: 'del'; oldNo: number; text: string }
  | { kind: 'add'; newNo: number; text: string }
  | { kind: 'gap'; hidden: number };

export interface RuleDiff {
  rows: DiffRow[];
  added: number;
  removed: number;
}

/** CRLF-safe split into lines; a trailing newline does not make an extra line. */
function splitLines(text: string): string[] {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  if (lines[lines.length - 1] === '') lines.pop();
  return lines;
}

function pushAll<T>(target: T[], items: readonly T[]): void {
  for (const item of items) target.push(item);
}

/**
 * Collapse runs of unchanged lines longer than 2 * context. A run at the start
 * of the file keeps only its last `context` lines, a run at the end only its
 * first `context` lines, a run in the middle both ends.
 */
function collapse(rows: DiffRow[], context: number): DiffRow[] {
  const out: DiffRow[] = [];
  let i = 0;
  while (i < rows.length) {
    if (rows[i].kind !== 'ctx') {
      out.push(rows[i]);
      i++;
      continue;
    }
    let j = i;
    while (j < rows.length && rows[j].kind === 'ctx') j++;
    const run = rows.slice(i, j);
    const len = run.length;
    const atStart = i === 0;
    const atEnd = j === rows.length;

    if (len <= 2 * context) {
      pushAll(out, run);
    } else if (atStart) {
      out.push({ kind: 'gap', hidden: len - context });
      pushAll(out, run.slice(len - context));
    } else if (atEnd) {
      pushAll(out, run.slice(0, context));
      out.push({ kind: 'gap', hidden: len - context });
    } else {
      pushAll(out, run.slice(0, context));
      out.push({ kind: 'gap', hidden: len - 2 * context });
      pushAll(out, run.slice(len - context));
    }
    i = j;
  }
  return out;
}

/**
 * Line-based unified diff of `canon` (old) against `current` (new), for the
 * rules screen. Both inputs are normalised to LF before diffing.
 */
export function buildRuleDiff(canon: string, current: string, context = 3): RuleDiff {
  const oldLines = splitLines(canon);
  const newLines = splitLines(current);

  const rows: DiffRow[] = [];
  let oldNo = 0;
  let newNo = 0;
  let added = 0;
  let removed = 0;

  for (const part of diffArrays(oldLines, newLines)) {
    for (const text of part.value) {
      if (part.added) {
        newNo++;
        added++;
        rows.push({ kind: 'add', newNo, text });
      } else if (part.removed) {
        oldNo++;
        removed++;
        rows.push({ kind: 'del', oldNo, text });
      } else {
        oldNo++;
        newNo++;
        rows.push({ kind: 'ctx', oldNo, newNo, text });
      }
    }
  }

  if (added === 0 && removed === 0) {
    return { rows: [], added, removed };
  }
  return { rows: collapse(rows, context), added, removed };
}
