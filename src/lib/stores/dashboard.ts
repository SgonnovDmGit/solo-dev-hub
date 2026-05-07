import { writable, get } from 'svelte/store';
import type {
  Period,
  PeriodPreset,
  DashboardFilter,
  DashboardData,
} from '$lib/types';
import { readDashboard } from '$lib/api/tauri-commands';
import { addToast } from './ui';

// ── Helpers ─────────────────────────────────────────────────────────────────

function formatDate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function parseDate(s: string): Date {
  const [y, m, d] = s.split('-').map(Number);
  return new Date(y, m - 1, d);
}

function addDays(d: Date, n: number): Date {
  const r = new Date(d);
  r.setDate(r.getDate() + n);
  return r;
}

function daysBetween(a: string, b: string): number {
  return Math.round((parseDate(b).getTime() - parseDate(a).getTime()) / 86400000);
}

function firstOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

function lastOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth() + 1, 0);
}

function firstOfQuarter(d: Date): Date {
  const q = Math.floor(d.getMonth() / 3);
  return new Date(d.getFullYear(), q * 3, 1);
}

function firstOfPrevQuarter(d: Date): Date {
  const q = Math.floor(d.getMonth() / 3);
  if (q === 0) {
    return new Date(d.getFullYear() - 1, 9, 1);  // Q4 prev year
  }
  return new Date(d.getFullYear(), (q - 1) * 3, 1);
}

function mondayOf(d: Date): Date {
  const dow = d.getDay();  // 0=Sun..6=Sat
  const offset = dow === 0 ? -6 : 1 - dow;
  return addDays(d, offset);
}

/**
 * Compute the current period bounds for a preset, given a reference date.
 * Rules from spec §Period semantics (locale-agnostic: Mon is week start).
 */
export function resolvePeriod(preset: PeriodPreset, ref: Date = new Date()): Period {
  switch (preset) {
    case 'week': {
      const start = mondayOf(ref);
      return { start: formatDate(start), end: formatDate(ref) };
    }
    case 'month': {
      return { start: formatDate(firstOfMonth(ref)), end: formatDate(ref) };
    }
    case 'quarter': {
      return { start: formatDate(firstOfQuarter(ref)), end: formatDate(ref) };
    }
    case 'custom': {
      return { start: formatDate(ref), end: formatDate(ref) };
    }
  }
}

/**
 * Compute the comparison window using "calendar-aligned partial same-length" rule.
 * Returns null for 'custom' preset OR when current period has d<1 (single-day).
 */
export function resolveComparePeriod(preset: PeriodPreset, current: Period): Period | null {
  if (preset === 'custom') return null;
  const d = daysBetween(current.start, current.end);
  if (d < 1) return null;

  const curStart = parseDate(current.start);

  let prevStart: Date;
  let clampEnd: Date | null = null;

  switch (preset) {
    case 'week': {
      prevStart = addDays(curStart, -7);
      break;
    }
    case 'month': {
      prevStart = new Date(curStart.getFullYear(), curStart.getMonth() - 1, 1);
      clampEnd = lastOfMonth(prevStart);
      break;
    }
    case 'quarter': {
      prevStart = firstOfPrevQuarter(curStart);
      const q = Math.floor(prevStart.getMonth() / 3);
      clampEnd = new Date(prevStart.getFullYear(), q * 3 + 3, 0);
      break;
    }
    default:
      return null;
  }

  let prevEnd = addDays(prevStart, d);
  if (clampEnd && prevEnd > clampEnd) {
    prevEnd = clampEnd;
  }
  return { start: formatDate(prevStart), end: formatDate(prevEnd) };
}

// ── Stores ──────────────────────────────────────────────────────────────────

export const currentPreset = writable<PeriodPreset>('week');
export const currentPeriod = writable<Period>(resolvePeriod('week'));
export const selectedProjectIds = writable<number[] | null>(null);  // null = all
export const dashboardData = writable<DashboardData | null>(null);
export const dashboardLoading = writable<boolean>(false);

// ── Actions ─────────────────────────────────────────────────────────────────

export async function loadDashboard(): Promise<void> {
  const preset = get(currentPreset);
  const period = get(currentPeriod);
  const compare = resolveComparePeriod(preset, period);
  const projIds = get(selectedProjectIds);

  const filter: DashboardFilter = {
    period,
    compare_period: compare,
    project_ids: projIds,
  };

  dashboardLoading.set(true);
  try {
    const data = await readDashboard(filter);
    dashboardData.set(data);
  } catch (err) {
    addToast(`Dashboard error: ${String(err)}`, 'error');
    dashboardData.set(null);
  } finally {
    dashboardLoading.set(false);
  }
}

export async function setPreset(preset: PeriodPreset): Promise<void> {
  currentPreset.set(preset);
  if (preset !== 'custom') {
    currentPeriod.set(resolvePeriod(preset));
  }
  await loadDashboard();
}

export async function setCustomPeriod(period: Period): Promise<void> {
  currentPreset.set('custom');
  currentPeriod.set(period);
  await loadDashboard();
}

export async function setProjectFilter(ids: number[] | null): Promise<void> {
  selectedProjectIds.set(ids);
  await loadDashboard();
}
