// F-000044 / T-000223: `create_bug` in a shared repo may be refused by the
// backend (`allocate_bug_numeric_id`). The refusal is a stable machine-readable
// string `band_refused:<code>[:<payload>]`; this maps it to a translated text.
// Keep the codes in step with the constants in sync/shared_structure.rs.
import type { TranslationKey } from '$lib/i18n/translations';

const PREFIX = 'band_refused:';

export interface BugRefusal {
  key: TranslationKey;
  args: string[];
}

/**
 * The refusal behind a `create_bug` error, or null when the error is not a band
 * refusal (it is then shown as is). Unknown codes of a newer backend map to a
 * generic text that still carries the code.
 */
export function parseBugRefusal(err: unknown): BugRefusal | null {
  const text = String(err ?? '');
  const at = text.indexOf(PREFIX);
  if (at < 0) return null;
  const rest = text.slice(at + PREFIX.length);
  const colon = rest.indexOf(':');
  const code = colon < 0 ? rest : rest.slice(0, colon);
  const payload = colon < 0 ? '' : rest.slice(colon + 1);
  switch (code.trim()) {
    case 'pending':
      return { key: 'bugs.refused.pending', args: [] };
    case 'no_login':
      return { key: 'bugs.refused.noLogin', args: [] };
    case 'untrusted':
      return { key: 'bugs.refused.untrusted', args: [payload.trim()] };
    case 'no_structure':
      return { key: 'bugs.refused.noStructure', args: [] };
    case 'no_range':
      return { key: 'bugs.refused.noRange', args: [] };
    case 'full': {
      const m = /^(\d+)-(\d+)$/.exec(payload.trim());
      return m
        ? { key: 'bugs.refused.full', args: [pad(m[1]), pad(m[2])] }
        : { key: 'bugs.refused.unknown', args: ['full'] };
    }
    default:
      return { key: 'bugs.refused.unknown', args: [code.trim()] };
  }
}

function pad(n: string): string {
  return n.padStart(6, '0');
}

/** The toast text of a refusal, or null when `err` is not one. */
export function bugRefusalMessage(
  err: unknown,
  translate: (key: TranslationKey, ...args: string[]) => string,
): string | null {
  const r = parseBugRefusal(err);
  return r ? translate(r.key, ...r.args) : null;
}
