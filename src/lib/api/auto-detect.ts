// T-000110: auto-detect runner for deploy template placeholders.
//
// meta.json placeholders may declare an `auto_detect` block that pre-fills the
// value from a file in the user's repo. Two modes:
//
//   1. Capture mode (default) — `regex` includes a capture group; matched
//      group [1] (or whole match if no group) becomes the value.
//      Used for: NODE_VERSION from `.nvmrc`, GO_VERSION from `go.mod`,
//      BUILD_OUTPUT_DIR from `vite.config.js` (`build.outDir: "build"`).
//
//   2. Predicate mode — `value_if_match` is set; the regex acts as a boolean
//      predicate. On match the static `value_if_match` becomes the value;
//      capture groups are ignored. Used for cases where presence-of-something
//      implies a fixed action: e.g. `@inlang/paraglide-js` in `package.json`
//      → `PRE_BUILD_COMMAND = "npm run paraglide:compile"`.
//
// `path` may be a single string or an ordered array. The runner tries each in
// turn and stops on the first hit. Files that don't exist are skipped.

export interface AutoDetectSpec {
  source: 'file';
  path: string | string[];
  regex: string;
  /** T-000110: optional. When present, switches the runner to predicate mode. */
  value_if_match?: string;
}

/**
 * Reads files via `readFile` and applies the regex. Returns the detected
 * value, or `null` on no match / bad input / unreadable files.
 *
 * `readFile` returns `null` for missing files (matches the existing
 * `readRepoFile` Tauri command contract) and the file content otherwise.
 */
export async function runAutoDetect(
  spec: AutoDetectSpec | null | undefined,
  readFile: (path: string) => Promise<string | null>,
): Promise<string | null> {
  if (!spec || spec.source !== 'file') return null;
  if (typeof spec.regex !== 'string' || spec.regex === '') return null;

  let re: RegExp;
  try {
    re = new RegExp(spec.regex, 'm');
  } catch {
    return null;
  }

  const paths = Array.isArray(spec.path)
    ? spec.path
    : typeof spec.path === 'string'
      ? [spec.path]
      : [];
  if (paths.length === 0) return null;

  for (const path of paths) {
    const content = await readFile(path);
    if (content == null) continue;
    const m = re.exec(content);
    if (!m) continue;

    // Predicate mode: regex is a boolean filter, value comes from the spec.
    if (typeof spec.value_if_match === 'string') return spec.value_if_match;

    // Capture mode: prefer group [1], fall back to whole match.
    return m[1] ?? m[0];
  }
  return null;
}
