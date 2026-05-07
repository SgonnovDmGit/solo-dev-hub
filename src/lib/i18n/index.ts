import { writable, derived, get } from 'svelte/store';
import { translations, type Locale, type TranslationKey } from './translations';
import { getSetting, setSetting } from '$lib/api/tauri-commands';

export const locale = writable<Locale>('ru');

export const tStore = derived(locale, ($locale) => {
  return (key: TranslationKey) => translations[$locale][key] ?? key;
});

export function t(key: TranslationKey): string {
  return translations[get(locale)][key] ?? key;
}

export function tf(key: TranslationKey, ...args: (string | number)[]): string {
  let s = t(key);
  args.forEach((a, i) => { s = s.replace(`{${i}}`, String(a)); });
  return s;
}

export async function initLocale(): Promise<void> {
  const saved = await getSetting('language');
  if (saved === 'en' || saved === 'ru') {
    locale.set(saved);
  }
}

export async function setLocale(newLocale: Locale): Promise<void> {
  locale.set(newLocale);
  await setSetting('language', newLocale);
}

export type { Locale, TranslationKey };
