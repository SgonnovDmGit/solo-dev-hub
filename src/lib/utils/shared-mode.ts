// F-000044 / T-000223: pure helpers of the contributor-mode UI (shared repos).
// No Svelte, no Tauri — everything here is unit-tested.
import type { RepoSharedState, ProjectStructure, JoinRequest } from '$lib/types';
import type { TranslationKey } from '$lib/i18n/translations';

/** Ids below this belong to the owner; participants start here (backend `FIRST_PARTICIPANT_BAND`). */
export const FIRST_PARTICIPANT_BAND = 500_000;
/** Numbers per participant (backend `BAND_SIZE`). */
export const BAND_SIZE = 100_000;

// ── Roles and ownership ──────────────────────────────────────────────────────

/** `owner` segment of `owner/name`; '' for a local-only repo. */
export function repoOwner(githubName: string | null | undefined): string {
  if (!githubName || !githubName.includes('/')) return '';
  return githubName.split('/')[0] ?? '';
}

export function sameLogin(a: string | null | undefined, b: string | null | undefined): boolean {
  const x = (a ?? '').trim().toLowerCase();
  const y = (b ?? '').trim().toLowerCase();
  return x !== '' && x === y;
}

/** Contributor or a helper whose request is not approved yet. */
export function isContributorRole(state: Pick<RepoSharedState, 'shared_role'> | null | undefined): boolean {
  const role = state?.shared_role;
  return role === 'contributor' || role === 'pending';
}

/**
 * Is the "Общий режим" switch offered here? Only when this install may be the
 * main one: the repo owner is this install's GitHub login, and the install is
 * not somebody's contributor.
 */
export function canToggleSharedMode(
  githubName: string | null | undefined,
  login: string | null | undefined,
  state: Pick<RepoSharedState, 'shared_role'> | null | undefined,
): boolean {
  if (isContributorRole(state)) return false;
  return sameLogin(repoOwner(githubName), login);
}

/** A repo that takes part in shared mode in any way (any role, or a status). */
export function isSharedRepo(state: RepoSharedState | null | undefined): boolean {
  return !!state && (state.shared_mode || state.shared_role !== null || state.shared_status !== null);
}

/**
 * Settings hint (R18): this install's login is known and at least one repo is
 * owned by somebody else, so a fine-grained token (one owner's repos only) is
 * not enough — a classic token with the `repo` scope is needed.
 */
export function needsClassicTokenHint(
  login: string | null | undefined,
  githubNames: (string | null | undefined)[],
): boolean {
  if ((login ?? '').trim() === '') return false;
  return githubNames.some((n) => {
    const owner = repoOwner(n);
    return owner !== '' && !sameLogin(owner, login);
  });
}

// ── Numbers ──────────────────────────────────────────────────────────────────

/** `500000–599999` for the participant band that starts at `start`. */
export function bandLabel(start: number): string {
  return `${start}–${start + BAND_SIZE - 1}`;
}

/** Top of the owner's band: the first participant band or the default ceiling. */
export function mainBandCeiling(structure: Pick<ProjectStructure, 'participants'>): number {
  const starts = structure.participants.map((p) => p.band_start);
  return (starts.length > 0 ? Math.min(...starts) : FIRST_PARTICIPANT_BAND) - 1;
}

/** This install's number range as text, or null when it has none (owner without structure, pending). */
export function myBandLabel(structure: ProjectStructure | null, login: string | null | undefined): string | null {
  if (!structure) return null;
  const me = structure.participants.find((p) => sameLogin(p.login, login));
  return me ? bandLabel(me.band_start) : null;
}

// ── Status codes -> UI ───────────────────────────────────────────────────────

/** `shared_status_detail` of `needs_manual_merge`: `{"branch":"dev","current":true}`. */
export function parseMergeDetail(detail: string | null | undefined): { branch: string; current: boolean } | null {
  const v = parseObject(detail);
  if (!v || typeof v.branch !== 'string' || v.branch === '') return null;
  return { branch: v.branch, current: v.current === true };
}

/** `shared_status_detail` of `push_rejected`: `{"branch":"dev","message":"..."}`. */
export function parseRejectedDetail(detail: string | null | undefined): { branch: string; message: string } | null {
  const v = parseObject(detail);
  if (!v || typeof v.branch !== 'string' || v.branch === '') return null;
  return { branch: v.branch, message: typeof v.message === 'string' ? v.message : '' };
}

function parseObject(detail: string | null | undefined): Record<string, unknown> | null {
  if (!detail) return null;
  try {
    const v = JSON.parse(detail);
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

/** Start of the `shared_untrusted` detail when the owner's dev branch changed (backend `DEV_BRANCH_CHANGED_PREFIX`). */
export const DEV_BRANCH_CHANGED_PREFIX = 'dev_branch_changed:';

/**
 * `shared_untrusted` detail `dev_branch_changed:<pin>→<new>`: the owner's dev
 * branch differs from the pinned one. Then the contributor may accept the new
 * branch.
 */
export function parseDevBranchChanged(detail: string | null | undefined): { pin: string; next: string } | null {
  if (!detail || !detail.startsWith(DEV_BRANCH_CHANGED_PREFIX)) return null;
  const rest = detail.slice(DEV_BRANCH_CHANGED_PREFIX.length);
  const at = rest.indexOf('\u2192');
  if (at < 0) return null;
  return { pin: rest.slice(0, at), next: rest.slice(at + 1) };
}

export type StatusAction = 'merge' | 'accept_branch';
export type StatusSeverity = 'warn' | 'error' | 'info';

export interface StatusLine {
  code: string;
  severity: StatusSeverity;
  icon: string;
  textKey: TranslationKey;
  /** Values for the `{0}`, `{1}`… placeholders of the text. */
  params: string[];
  action: StatusAction | null;
}

/**
 * One status line of a repo (spec R13), or null for a healthy repo. `ownerLogin`
 * is the owner's login for the pending-approval line (from the structure).
 */
export function describeSharedStatus(
  state: Pick<RepoSharedState, 'shared_status' | 'shared_status_detail'> | null | undefined,
  ownerLogin: string | null | undefined = null,
): StatusLine | null {
  const code = state?.shared_status ?? null;
  if (!code) return null;
  const detail = state?.shared_status_detail ?? null;
  switch (code) {
    case 'needs_manual_merge': {
      const m = parseMergeDetail(detail);
      if (!m) {
        return line(code, 'warn', '⚠', 'shared.status.needsMergeUnknown', [], 'merge');
      }
      return m.current
        ? line(code, 'warn', '⚠', 'shared.status.needsMergeCurrent', [m.branch], 'merge')
        : line(code, 'warn', '⚠', 'shared.status.needsMergeOther', [m.branch], null);
    }
    case 'push_rejected': {
      const r = parseRejectedDetail(detail);
      return r
        ? line(code, 'error', '✕', 'shared.status.pushRejected', [r.branch], null)
        : line(code, 'error', '✕', 'shared.status.pushRejectedUnknown', [], null);
    }
    case 'fetch_failed':
      return line(code, 'warn', '⚠', 'shared.status.fetchFailed', [], null);
    case 'push_failed':
      return line(code, 'warn', '⚠', 'shared.status.pushFailed', [], null);
    case 'no_own_branch':
      return line(code, 'warn', '⚠', 'shared.status.noOwnBranch', [], null);
    case 'pending_approval':
      return line(code, 'info', '⏳', 'shared.status.pendingApproval', [ownerLogin ?? '?'], null);
    case 'shared_untrusted': {
      const changed = parseDevBranchChanged(detail);
      return changed
        ? line(code, 'error', '✕', 'shared.status.devBranchChanged', [changed.pin, changed.next], 'accept_branch')
        : line(code, 'error', '✕', 'shared.status.untrusted', [detail ?? '?'], null);
    }
    case 'shared_no_login':
      return line(code, 'warn', '⚠', 'shared.status.noLogin', [], null);
    default:
      // A code of a newer backend: show it rather than hide the problem.
      return line(code, 'warn', '⚠', 'shared.status.unknown', [code], null);
  }
}

function line(
  code: string,
  severity: StatusSeverity,
  icon: string,
  textKey: TranslationKey,
  params: string[],
  action: StatusAction | null,
): StatusLine {
  return { code, severity, icon, textKey, params, action };
}

/** Fill the `{0}`, `{1}`… placeholders. */
export function fillParams(template: string, params: string[]): string {
  return params.reduce((s, p, i) => s.split(`{${i}}`).join(p), template);
}

// ── Sidebar badge ────────────────────────────────────────────────────────────

export interface SidebarBadge {
  glyph: '⇄' | '⚠';
  /** A status needs attention: warning colour, the tooltip is the status text. */
  problem: boolean;
}

/**
 * Badge after the repo name: ⇄ for a shared repo, ⚠ when it has a problem
 * status. A pending join request is waiting, not a problem: it keeps ⇄.
 */
export function sidebarBadge(state: RepoSharedState | null | undefined): SidebarBadge | null {
  if (!isSharedRepo(state)) return null;
  const status = state!.shared_status;
  if (status && status !== 'pending_approval') return { glyph: '⚠', problem: true };
  return { glyph: '⇄', problem: false };
}

// ── Participants tab ─────────────────────────────────────────────────────────

export type ParticipantKind = 'owner' | 'participant' | 'request';

export interface ParticipantRow {
  kind: ParticipantKind;
  login: string;
  branch: string;
  /** Number range as text; null for a request (none assigned yet). */
  band: string | null;
  /** Upper bound of the owner's band, for the "до N" text. */
  ownerCeiling: number | null;
  /** This row is this install. */
  isMe: boolean;
  /** The "Подтвердить" button is offered (owner's install, requests only). */
  canApprove: boolean;
}

/**
 * Table rows: the owner first, then participants in band order, then join
 * requests. `isOwnerInstall` is true on the owner's machine (approve buttons).
 */
export function buildParticipantRows(
  structure: ProjectStructure | null,
  requests: JoinRequest[],
  login: string | null | undefined,
  isOwnerInstall: boolean,
): ParticipantRow[] {
  if (!structure) return [];
  const rows: ParticipantRow[] = [
    {
      kind: 'owner',
      login: structure.main.login,
      branch: structure.main.dev_branch,
      band: null,
      ownerCeiling: mainBandCeiling(structure),
      isMe: sameLogin(structure.main.login, login),
      canApprove: false,
    },
  ];
  const sorted = [...structure.participants].sort((a, b) => a.band_start - b.band_start);
  for (const p of sorted) {
    rows.push({
      kind: 'participant',
      login: p.login,
      branch: p.branch,
      band: bandLabel(p.band_start),
      ownerCeiling: null,
      isMe: sameLogin(p.login, login),
      canApprove: false,
    });
  }
  for (const r of requests) {
    // A request of somebody already listed is stale (the approval is on its way).
    if (rows.some((x) => sameLogin(x.login, r.login))) continue;
    rows.push({
      kind: 'request',
      login: r.login,
      branch: r.branch,
      band: null,
      ownerCeiling: null,
      isMe: sameLogin(r.login, login),
      canApprove: isOwnerInstall,
    });
  }
  return rows;
}

/** Number of requests waiting for the owner's approval (rows that are requests). */
export function pendingRequestCount(rows: ParticipantRow[]): number {
  return rows.filter((r) => r.kind === 'request').length;
}

/** Number shown in the tab title "Участники (N)": owner + participants + requests. */
export function participantsTabCount(rows: ParticipantRow[]): number {
  return rows.length;
}

// ── Plural forms of "N requests" ─────────────────────────────────────────────

export type PluralForm = 'one' | 'few' | 'many';

/** Plural category of `n` (Russian: one / few / many; English: one / many). */
export function pluralForm(n: number, locale: 'ru' | 'en'): PluralForm {
  const abs = Math.abs(Math.trunc(n));
  if (locale === 'en') return abs === 1 ? 'one' : 'many';
  const m10 = abs % 10;
  const m100 = abs % 100;
  if (m10 === 1 && m100 !== 11) return 'one';
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return 'few';
  return 'many';
}

export function requestsWaitingKey(n: number, locale: 'ru' | 'en'): TranslationKey {
  const form = pluralForm(n, locale);
  return form === 'one'
    ? 'shared.requestsWaitingOne'
    : form === 'few'
      ? 'shared.requestsWaitingFew'
      : 'shared.requestsWaitingMany';
}
