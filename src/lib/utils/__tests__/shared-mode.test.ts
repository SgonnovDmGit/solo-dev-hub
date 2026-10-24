import { describe, it, expect } from 'vitest';
import type { RepoSharedState, ProjectStructure } from '$lib/types';
import {
  repoOwner,
  sameLogin,
  isContributorRole,
  canToggleSharedMode,
  isSharedRepo,
  bandLabel,
  mainBandCeiling,
  myBandLabel,
  parseMergeDetail,
  parseRejectedDetail,
  parseDevBranchChanged,
  describeSharedStatus,
  fillParams,
  sidebarBadge,
  buildParticipantRows,
  pendingRequestCount,
  pluralForm,
  requestsWaitingKey,
  needsClassicTokenHint,
  autoSyncOffHint,
  activityByLogin,
} from '../shared-mode';

function state(over: Partial<RepoSharedState> = {}): RepoSharedState {
  return {
    repo_id: 1,
    shared_mode: false,
    shared_role: null,
    shared_status: null,
    shared_status_detail: null,
    ...over,
  };
}

function structure(over: Partial<ProjectStructure> = {}): ProjectStructure {
  return {
    format: 1,
    project: { name: 'Acme', type: 'standard' },
    main: { login: 'Owner', dev_branch: 'dev' },
    repos: [{ github: 'Owner/api', role: 'server' }],
    microservices: [],
    participants: [],
    ...over,
  };
}

describe('ownership and roles', () => {
  it('takes the owner segment of owner/name', () => {
    expect(repoOwner('Owner/api')).toBe('Owner');
    expect(repoOwner('api')).toBe('');
    expect(repoOwner(null)).toBe('');
  });

  it('compares logins case-insensitively and never matches empty', () => {
    expect(sameLogin('Owner', 'owner')).toBe(true);
    expect(sameLogin(' owner ', 'OWNER')).toBe(true);
    expect(sameLogin('', '')).toBe(false);
    expect(sameLogin(null, 'owner')).toBe(false);
  });

  it('knows contributor roles', () => {
    expect(isContributorRole(state({ shared_role: 'contributor' }))).toBe(true);
    expect(isContributorRole(state({ shared_role: 'pending' }))).toBe(true);
    expect(isContributorRole(state({ shared_role: 'main' }))).toBe(false);
    expect(isContributorRole(null)).toBe(false);
  });

  it('offers the switch to the owner and to an administrator of the repo', () => {
    expect(canToggleSharedMode('Owner/api', 'owner', null)).toBe(true);
    expect(canToggleSharedMode('Owner/api', 'OWNER', null)).toBe(true);
    expect(canToggleSharedMode('Owner/api', 'owner', state({ shared_role: 'main' }))).toBe(true);
    // A collaborator without admin rights never sees it.
    expect(canToggleSharedMode('Owner/api', 'helper', null)).toBe(false);
    expect(canToggleSharedMode('Owner/api', 'helper', null, new Set(['other/repo']))).toBe(false);
    // An organization's repo the login administers.
    expect(canToggleSharedMode('Acme/api', 'helper', null, new Set(['acme/api']))).toBe(true);
    // Main already: stays, so it can be switched off.
    expect(canToggleSharedMode('Acme/api', 'helper', state({ shared_role: 'main' }))).toBe(true);
    expect(canToggleSharedMode('Owner/api', 'helper', state({ shared_role: 'contributor' }))).toBe(false);
    expect(canToggleSharedMode('Owner/api', null, null)).toBe(false);
    expect(canToggleSharedMode('Owner/api', 'owner', state({ shared_role: 'contributor' }))).toBe(false);
    expect(canToggleSharedMode('Owner/api', 'owner', state({ shared_role: 'pending' }))).toBe(false);
    expect(canToggleSharedMode(null, 'owner', null)).toBe(false);
  });

  it('calls a repo shared by the switch, a role or a status', () => {
    expect(isSharedRepo(null)).toBe(false);
    expect(isSharedRepo(state())).toBe(false);
    expect(isSharedRepo(state({ shared_mode: true }))).toBe(true);
    expect(isSharedRepo(state({ shared_role: 'pending' }))).toBe(true);
    expect(isSharedRepo(state({ shared_status: 'fetch_failed' }))).toBe(true);
  });
});

describe('number bands', () => {
  it('labels a participant band', () => {
    expect(bandLabel(500000)).toBe('500000–599999');
    expect(bandLabel(600000)).toBe('600000–699999');
  });

  it('puts the owner ceiling under the first participant band', () => {
    expect(mainBandCeiling(structure())).toBe(499999);
    const s = structure({
      participants: [
        { login: 'b', branch: 'b-dev', band_start: 600000 },
        { login: 'a', branch: 'a-dev', band_start: 500000 },
      ],
    });
    expect(mainBandCeiling(s)).toBe(499999);
  });

  it('shows a contributor their own band, nobody else', () => {
    const s = structure({ participants: [{ login: 'Helper', branch: 'h', band_start: 500000 }] });
    expect(myBandLabel(s, 'helper')).toBe('500000–599999');
    expect(myBandLabel(s, 'owner')).toBeNull();
    expect(myBandLabel(null, 'helper')).toBeNull();
  });
});

describe('status details', () => {
  it('parses the merge detail', () => {
    expect(parseMergeDetail('{"branch":"dev","current":true}')).toEqual({ branch: 'dev', current: true });
    expect(parseMergeDetail('{"branch":"feat","current":false}')).toEqual({ branch: 'feat', current: false });
    expect(parseMergeDetail('{"branch":"dev"}')).toEqual({ branch: 'dev', current: false });
    expect(parseMergeDetail('not json')).toBeNull();
    expect(parseMergeDetail('{"current":true}')).toBeNull();
    expect(parseMergeDetail('[1]')).toBeNull();
    expect(parseMergeDetail(null)).toBeNull();
  });

  it('parses the rejected-push detail', () => {
    expect(parseRejectedDetail('{"branch":"dev","message":"GH006"}')).toEqual({ branch: 'dev', message: 'GH006' });
    expect(parseRejectedDetail('{"branch":"dev"}')).toEqual({ branch: 'dev', message: '' });
    expect(parseRejectedDetail('GH006 plain')).toBeNull();
  });

  it('parses the coded dev branch change', () => {
    expect(parseDevBranchChanged('dev_branch_changed:dev→next')).toEqual({ pin: 'dev', next: 'next' });
    expect(parseDevBranchChanged('dev_branch_changed:a/b→c-d')).toEqual({ pin: 'a/b', next: 'c-d' });
    // the old English text no longer counts
    expect(parseDevBranchChanged('.sdh/project.json: dev branch changed from dev to next')).toBeNull();
    expect(parseDevBranchChanged('dev_branch_changed:nodelimiter')).toBeNull();
    expect(parseDevBranchChanged('main is not the repo owner')).toBeNull();
    expect(parseDevBranchChanged(null)).toBeNull();
  });
});

describe('describeSharedStatus', () => {
  it('is null for a healthy repo', () => {
    expect(describeSharedStatus(state())).toBeNull();
    expect(describeSharedStatus(null)).toBeNull();
  });

  it('offers Merge for a diverged current branch', () => {
    const l = describeSharedStatus(
      state({ shared_status: 'needs_manual_merge', shared_status_detail: '{"branch":"dev","current":true}' }),
    )!;
    expect(l.textKey).toBe('shared.status.needsMergeCurrent');
    expect(l.params).toEqual(['dev']);
    expect(l.action).toBe('merge');
    expect(l.severity).toBe('warn');
    expect(l.icon).toBe('⚠');
  });

  it('has no button for a diverged branch that is not checked out', () => {
    const l = describeSharedStatus(
      state({ shared_status: 'needs_manual_merge', shared_status_detail: '{"branch":"feat","current":false}' }),
    )!;
    expect(l.textKey).toBe('shared.status.needsMergeOther');
    expect(l.params).toEqual(['feat']);
    expect(l.action).toBeNull();
  });

  it('keeps the Merge button when the branch is unknown', () => {
    const l = describeSharedStatus(state({ shared_status: 'needs_manual_merge' }))!;
    expect(l.textKey).toBe('shared.status.needsMergeUnknown');
    expect(l.action).toBe('merge');
  });

  it('names the protected branch of a rejected push', () => {
    const l = describeSharedStatus(
      state({ shared_status: 'push_rejected', shared_status_detail: '{"branch":"dev","message":"GH006"}' }),
    )!;
    expect(l.textKey).toBe('shared.status.pushRejected');
    expect(l.params).toEqual(['dev']);
    expect(l.severity).toBe('error');
    expect(l.icon).toBe('✕');
    expect(describeSharedStatus(state({ shared_status: 'push_rejected' }))!.textKey).toBe(
      'shared.status.pushRejectedUnknown',
    );
  });

  it('maps network, branch, approval and login statuses', () => {
    expect(describeSharedStatus(state({ shared_status: 'fetch_failed' }))!.textKey).toBe('shared.status.fetchFailed');
    expect(describeSharedStatus(state({ shared_status: 'push_failed' }))!.textKey).toBe('shared.status.pushFailed');
    expect(describeSharedStatus(state({ shared_status: 'no_own_branch' }))!.textKey).toBe('shared.status.noOwnBranch');
    expect(describeSharedStatus(state({ shared_status: 'shared_no_login' }))!.textKey).toBe('shared.status.noLogin');
    const p = describeSharedStatus(state({ shared_status: 'pending_approval' }), 'Owner')!;
    expect(p.textKey).toBe('shared.status.pendingApproval');
    expect(p.params).toEqual(['Owner']);
    expect(p.icon).toBe('⏳');
    expect(p.severity).toBe('info');
  });

  it('offers to accept a changed dev branch only for that untrusted code', () => {
    const changed = describeSharedStatus(
      state({ shared_status: 'shared_untrusted', shared_status_detail: 'dev_branch_changed:dev→next' }),
    )!;
    expect(changed.action).toBe('accept_branch');
    expect(changed.textKey).toBe('shared.status.devBranchChanged');
    expect(changed.params).toEqual(['dev', 'next']);
    const other = describeSharedStatus(
      state({ shared_status: 'shared_untrusted', shared_status_detail: 'main is not the repo owner' }),
    )!;
    expect(other.action).toBeNull();
    expect(other.textKey).toBe('shared.status.untrusted');
    // the English wording of an old backend does not unlock the button
    const legacy = describeSharedStatus(
      state({ shared_status: 'shared_untrusted', shared_status_detail: 'origin/dev: dev branch changed from dev to next' }),
    )!;
    expect(legacy.action).toBeNull();
  });

  it('shows a code of a newer backend rather than hiding it', () => {
    const l = describeSharedStatus(state({ shared_status: 'brand_new' }))!;
    expect(l.textKey).toBe('shared.status.unknown');
    expect(l.params).toEqual(['brand_new']);
  });
});

describe('fillParams', () => {
  it('fills every occurrence of a placeholder', () => {
    expect(fillParams('{0} vs origin/{0} ({1})', ['dev', 'x'])).toBe('dev vs origin/dev (x)');
    expect(fillParams('no params', [])).toBe('no params');
  });
});

describe('sidebarBadge', () => {
  it('shows nothing for an ordinary repo', () => {
    expect(sidebarBadge(null)).toBeNull();
    expect(sidebarBadge(state())).toBeNull();
  });

  it('shows ⇄ for a shared repo, including a pending one', () => {
    expect(sidebarBadge(state({ shared_mode: true }))).toEqual({ glyph: '⇄', problem: false });
    expect(sidebarBadge(state({ shared_role: 'contributor' }))).toEqual({ glyph: '⇄', problem: false });
    expect(sidebarBadge(state({ shared_role: 'pending', shared_status: 'pending_approval' }))).toEqual({
      glyph: '⇄',
      problem: false,
    });
  });

  it('shows ⚠ for a problem status', () => {
    for (const status of ['needs_manual_merge', 'push_rejected', 'fetch_failed', 'push_failed', 'no_own_branch', 'shared_untrusted', 'shared_no_login']) {
      expect(sidebarBadge(state({ shared_mode: true, shared_status: status }))).toEqual({ glyph: '⚠', problem: true });
    }
  });
});

describe('buildParticipantRows', () => {
  const s = structure({
    participants: [
      { login: 'second', branch: 'second-dev', band_start: 600000 },
      { login: 'Helper', branch: 'helper-dev', band_start: 500000 },
    ],
  });

  it('lists the owner, participants by band, then requests', () => {
    const rows = buildParticipantRows(s, [{ login: 'newbie', branch: 'newbie-dev' }], 'owner', true);
    expect(rows.map((r) => [r.kind, r.login])).toEqual([
      ['owner', 'Owner'],
      ['participant', 'Helper'],
      ['participant', 'second'],
      ['request', 'newbie'],
    ]);
    expect(rows[0]).toMatchObject({ branch: 'dev', ownerCeiling: 499999, band: null, isMe: true, canApprove: false });
    expect(rows[1]).toMatchObject({ band: '500000–599999', ownerCeiling: null, isMe: false });
    expect(rows[2].band).toBe('600000–699999');
    expect(rows[3]).toMatchObject({ band: null, branch: 'newbie-dev', canApprove: true });
    expect(pendingRequestCount(rows)).toBe(1);
  });

  it('marks the contributor and offers approval to the owner only', () => {
    const rows = buildParticipantRows(s, [{ login: 'newbie', branch: 'n' }], 'helper', false);
    expect(rows.find((r) => r.login === 'Helper')!.isMe).toBe(true);
    expect(rows.find((r) => r.login === 'Owner')!.isMe).toBe(false);
    expect(rows.every((r) => !r.canApprove)).toBe(true);
  });

  it('drops a request of somebody already listed and tolerates no structure', () => {
    const rows = buildParticipantRows(s, [{ login: 'HELPER', branch: 'x' }, { login: 'owner', branch: 'y' }], 'owner', true);
    expect(pendingRequestCount(rows)).toBe(0);
    expect(rows).toHaveLength(3);
    expect(buildParticipantRows(null, [], 'owner', true)).toEqual([]);
  });
});

describe('plural forms of "N requests"', () => {
  it('follows the Russian rules', () => {
    const ru = (n: number) => pluralForm(n, 'ru');
    expect([1, 21, 101].map(ru)).toEqual(['one', 'one', 'one']);
    expect([2, 3, 4, 22, 24, 102].map(ru)).toEqual(['few', 'few', 'few', 'few', 'few', 'few']);
    expect([0, 5, 11, 12, 13, 14, 15, 20, 25, 111, 112].map(ru).every((f) => f === 'many')).toBe(true);
  });

  it('follows the English rules', () => {
    expect(pluralForm(1, 'en')).toBe('one');
    expect(pluralForm(0, 'en')).toBe('many');
    expect(pluralForm(2, 'en')).toBe('many');
  });

  it('picks the matching key', () => {
    expect(requestsWaitingKey(1, 'ru')).toBe('shared.requestsWaitingOne');
    expect(requestsWaitingKey(3, 'ru')).toBe('shared.requestsWaitingFew');
    expect(requestsWaitingKey(5, 'ru')).toBe('shared.requestsWaitingMany');
    expect(requestsWaitingKey(3, 'en')).toBe('shared.requestsWaitingMany');
  });
});

describe('needsClassicTokenHint', () => {
  it('needs a known login and a repo of another owner', () => {
    expect(needsClassicTokenHint('me', ['me/a', 'Me/b'])).toBe(false);
    expect(needsClassicTokenHint('me', ['me/a', 'other/b'])).toBe(true);
    expect(needsClassicTokenHint(null, ['other/b'])).toBe(false);
    expect(needsClassicTokenHint('', ['other/b'])).toBe(false);
    expect(needsClassicTokenHint('me', [null, undefined, 'local-only'])).toBe(false);
  });
});

describe('autoSyncOffHint', () => {
  const projects = [
    { id: 1, auto_sync_enabled: true },
    { id: 2, auto_sync_enabled: false },
  ];

  it('is null for a main install and for a repo that is not shared', () => {
    expect(autoSyncOffHint(state({ shared_role: 'main' }), false, 1, projects)).toBeNull();
    expect(autoSyncOffHint(state({ shared_role: null }), false, 1, projects)).toBeNull();
    expect(autoSyncOffHint(null, false, 1, projects)).toBeNull();
  });

  it('is null when auto-sync will run', () => {
    expect(autoSyncOffHint(state({ shared_role: 'contributor' }), true, 1, projects)).toBeNull();
    expect(autoSyncOffHint(state({ shared_role: 'pending' }), true, 1, projects)).toBeNull();
  });

  it('is due when the global switch is off', () => {
    expect(autoSyncOffHint(state({ shared_role: 'contributor' }), false, 1, projects)).toEqual({ projectId: 1 });
  });

  it('is due when the project has auto-sync off', () => {
    expect(autoSyncOffHint(state({ shared_role: 'pending' }), true, 2, projects)).toEqual({ projectId: 2 });
  });

  it('is due without a project, with no project to switch on', () => {
    expect(autoSyncOffHint(state({ shared_role: 'contributor' }), true, null, projects)).toEqual({ projectId: null });
    expect(autoSyncOffHint(state({ shared_role: 'contributor' }), false, undefined, [])).toEqual({ projectId: null });
  });

  it('waits while the repo project is not in the list yet', () => {
    expect(autoSyncOffHint(state({ shared_role: 'contributor' }), false, 9, projects)).toBeNull();
  });
});

describe('activityByLogin', () => {
  it('keys by lower-cased login and keeps nulls', () => {
    const m = activityByLogin([
      { login: 'Helper', last_at: '2026-10-20T10:00:00Z' },
      { login: 'other', last_at: null },
    ]);
    expect(m.get('helper')).toBe('2026-10-20T10:00:00Z');
    expect(m.get('other')).toBeNull();
    expect(m.has('nobody')).toBe(false);
  });
});
