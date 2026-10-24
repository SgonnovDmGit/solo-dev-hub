import { describe, it, expect } from 'vitest';
import { chooseSyncAction } from '../sync-target';

describe('chooseSyncAction', () => {
  it('syncs the project of a repo that has one', () => {
    expect(chooseSyncAction(7)).toEqual({ kind: 'project', projectId: 7 });
  });

  it('keeps project id 0 as a project (only null/undefined mean none)', () => {
    expect(chooseSyncAction(0)).toEqual({ kind: 'project', projectId: 0 });
  });

  it('probes a repo without a project', () => {
    expect(chooseSyncAction(null)).toEqual({ kind: 'probe' });
    expect(chooseSyncAction(undefined)).toEqual({ kind: 'probe' });
  });
});
