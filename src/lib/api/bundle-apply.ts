// v1.3.0: pure helpers to apply a secret bundle onto a push surface.
// Two surfaces: DeploySecretsTable holds a Record<name,value> (use
// mergeBundleValues); SecretsPanel holds a dotenv-style textarea (use
// mergeBundleIntoEnvText). Bundle always wins on name conflict.
import { parseEnvText } from './secrets-parser';
import type { SecretBundleItemValue } from '$lib/types';

export function mergeBundleValues(
  current: Record<string, string>,
  bundleItems: Pick<SecretBundleItemValue, 'secret_name' | 'value'>[],
): Record<string, string> {
  const out = { ...current };
  for (const item of bundleItems) out[item.secret_name] = item.value;
  return out;
}

function serializeEnvValue(v: string): string {
  // Triple-quote multiline values for readability — BUT only when the value
  // itself contains no `"""` (the triple-quote format has no escape for it;
  // parseEnvText would stop at the first `"""` and silently truncate).
  if (v.includes('\n') && !v.includes('"""')) return `"""${v}"""`;
  // Everything else that needs quoting → double-quoted with full escaping that
  // parseEnvText's double-quote decoder reverses exactly (\\, \", \n, \r, \t).
  // This path round-trips ANY value, including multiline-with-`"""`.
  if (/[\s#"']/.test(v)) {
    const escaped = v
      .replace(/\\/g, '\\\\')
      .replace(/"/g, '\\"')
      .replace(/\n/g, '\\n')
      .replace(/\r/g, '\\r')
      .replace(/\t/g, '\\t');
    return `"${escaped}"`;
  }
  return v;
}

export function mergeBundleIntoEnvText(
  currentText: string,
  bundleItems: Pick<SecretBundleItemValue, 'secret_name' | 'value'>[],
): string {
  // Parse existing entries into a map (later wins, matching parseEnvText order),
  // overlay the bundle, then re-serialize. Comments/formatting are not preserved
  // — acceptable: the user reviews the textarea before pushing.
  const parsed = parseEnvText(currentText).secrets;
  const map: Record<string, string> = {};
  for (const s of parsed) map[s.name] = s.value;
  const merged = mergeBundleValues(map, bundleItems);
  return Object.entries(merged)
    .map(([k, v]) => `${k}=${serializeEnvValue(v)}`)
    .join('\n');
}
