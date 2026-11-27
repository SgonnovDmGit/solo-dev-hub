/**
 * T-000263: `rule_unknown_skill` issues carrying their repo are actionable —
 * each becomes its own toast with a "Delete marker" action; everything else
 * bundles under the template exactly as before.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { toastSyncIssues } from '../sync-issues';
import type { SyncIssue } from '$lib/types';

const addToast = vi.fn();
vi.mock('$lib/stores/ui', () => ({
  addToast: (...args: unknown[]) => addToast(...args),
}));

vi.mock('$lib/i18n', () => ({
  tf: (key: string, ...args: unknown[]) => (args.length ? `${key}: ${args.join(', ')}` : key),
  locale: {
    subscribe: (fn: (v: string) => void) => {
      fn('en');
      return () => {};
    },
  },
}));

const removeUnknownSkillMarker = vi.fn();
vi.mock('$lib/api/tauri-commands/templates', () => ({
  removeUnknownSkillMarker: (...args: unknown[]) => removeUnknownSkillMarker(...args),
}));

const staleMarker: SyncIssue = {
  code: 'rule_unknown_skill',
  subject: 'owner/repo',
  target: '1c-development.md.approved',
  repo_id: 7,
};

const plain: SyncIssue = {
  code: 'copy_failed',
  subject: 'REQ-001_x.md',
  target: 'backend',
};

interface ToastActionShape {
  label: string;
  confirmTitle: string;
  confirmMessage: string;
  run: () => Promise<boolean>;
}

function actionableCall(): [string, string, { action: ToastActionShape }] {
  const call = addToast.mock.calls.find((c) => c[2]?.action);
  expect(call, 'an actionable toast').toBeTruthy();
  return call as [string, string, { action: ToastActionShape }];
}

describe('toastSyncIssues', () => {
  beforeEach(() => {
    addToast.mockReset();
    removeUnknownSkillMarker.mockReset();
  });

  it('splits actionable issues out of the bundle, keeping the rest bundled', () => {
    // An unknown-skill issue WITHOUT a repo id (an older backend) degrades
    // to a plain bundled line — it is not actionable.
    const withoutRepo: SyncIssue = { ...staleMarker, repo_id: undefined };
    toastSyncIssues('HEADER\n{0}', [plain, staleMarker, withoutRepo], 'error');

    const bundle = addToast.mock.calls.find((c) => !c[2]?.action);
    expect(bundle).toBeTruthy();
    expect(bundle![0]).toMatch(/^HEADER\n/);
    expect(bundle![0]).toContain('REQ-001_x.md');
    expect(bundle![0]).toContain('1c-development.md.approved');
    expect(bundle![1]).toBe('error');

    const [message, type] = actionableCall();
    expect(type).toBe('warning');
    expect(message).toContain('owner/repo');
  });

  it('shows no bundle at all when every issue was actionable', () => {
    toastSyncIssues('HEADER\n{0}', [staleMarker], 'warning');
    expect(addToast.mock.calls.filter((c) => !c[2]?.action)).toHaveLength(0);
    expect(addToast.mock.calls.filter((c) => c[2]?.action)).toHaveLength(1);
  });

  it('bundles everything when nothing is actionable', () => {
    toastSyncIssues('HEADER\n{0}', [plain], 'error');
    expect(addToast.mock.calls.filter((c) => c[2]?.action)).toHaveLength(0);
    expect(addToast).toHaveBeenCalledTimes(1);
  });

  it('arms the action with a confirmation naming the file and the repo', () => {
    toastSyncIssues('HEADER\n{0}', [staleMarker], 'warning');
    const [, , opts] = actionableCall();
    expect(opts.action.label).toBe('syncIssue.unknownSkillRemove.button');
    expect(opts.action.confirmTitle).toBe('syncIssue.unknownSkillRemove.confirmTitle');
    expect(opts.action.confirmMessage).toBe(
      'syncIssue.unknownSkillRemove.confirmMessage: 1c-development.md.approved, owner/repo',
    );
  });

  it('deletes the marker on confirm and reports success, dismissing the toast', async () => {
    removeUnknownSkillMarker.mockResolvedValue(undefined);
    toastSyncIssues('HEADER\n{0}', [staleMarker], 'warning');
    const [, , opts] = actionableCall();

    addToast.mockReset();
    const dismiss = await opts.action.run();

    expect(removeUnknownSkillMarker).toHaveBeenCalledWith(7, '1c-development.md.approved');
    expect(dismiss).toBe(true);
    expect(addToast).toHaveBeenCalledWith(
      'syncIssue.unknownSkillRemove.done: 1c-development.md.approved',
      'success',
    );
  });

  it('explains the refusal when the rule reached the canon meanwhile', async () => {
    removeUnknownSkillMarker.mockRejectedValue(new Error('rule_marker_now_known'));
    toastSyncIssues('HEADER\n{0}', [staleMarker], 'warning');
    const [, , opts] = actionableCall();

    addToast.mockReset();
    const dismiss = await opts.action.run();

    expect(dismiss).toBe(false);
    expect(addToast).toHaveBeenCalledWith('syncIssue.unknownSkillRemove.nowKnown', 'warning');
  });

  it('shows the raw error of any other failure and keeps the toast', async () => {
    removeUnknownSkillMarker.mockRejectedValue(new Error('Access is denied. (os error 5)'));
    toastSyncIssues('HEADER\n{0}', [staleMarker], 'warning');
    const [, , opts] = actionableCall();

    addToast.mockReset();
    const dismiss = await opts.action.run();

    expect(dismiss).toBe(false);
    const [text, type] = addToast.mock.calls[0];
    expect(text).toBe('syncIssue.unknownSkillRemove.failed: Error: Access is denied. (os error 5)');
    expect(type).toBe('error');
  });
});
