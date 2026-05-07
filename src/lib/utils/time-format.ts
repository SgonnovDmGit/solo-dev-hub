import { readable } from 'svelte/store';
import { get } from 'svelte/store';
import { locale } from '$lib/i18n';

export type Locale = 'ru' | 'en';

/**
 * Format an ISO8601 timestamp as relative time string.
 * Thresholds:
 *   <1 min   → "только что" / "just now"
 *   <60 min  → "{N} мин назад" / "{N}m ago"
 *   <24 h    → "{N} ч назад" / "{N}h ago"
 *   ≥24 h    → "{N} дн назад" / "{N}d ago"
 *
 * `loc` — pass current $locale from a reactive context to make derived values
 * recompute on language switch. Defaults to a one-shot read via get(locale).
 */
export function formatRelativeTime(
  iso: string,
  nowMs: number = Date.now(),
  loc?: Locale,
): string {
  const language: Locale = loc ?? (get(locale) === 'en' ? 'en' : 'ru');
  const then = new Date(iso).getTime();
  const deltaMs = nowMs - then;

  if (deltaMs < 60_000) {
    return language === 'en' ? 'just now' : 'только что';
  }
  if (deltaMs < 60 * 60_000) {
    const mins = Math.floor(deltaMs / 60_000);
    return language === 'en' ? `${mins}m ago` : `${mins} мин назад`;
  }
  if (deltaMs < 24 * 60 * 60_000) {
    const hours = Math.floor(deltaMs / (60 * 60_000));
    return language === 'en' ? `${hours}h ago` : `${hours} ч назад`;
  }
  const days = Math.floor(deltaMs / (24 * 60 * 60_000));
  return language === 'en' ? `${days}d ago` : `${days} дн назад`;
}

/**
 * Reactive "now" tick — emits `Date.now()` every minute.
 * Subscribe alongside a timestamp store to make `formatRelativeTime`
 * results auto-refresh as time passes (e.g. "только что" → "1 мин назад").
 */
export const nowTick = readable<number>(Date.now(), (set) => {
  const id = setInterval(() => set(Date.now()), 60_000);
  return () => clearInterval(id);
});
