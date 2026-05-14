// T-000109: SemVer-aware comparison for the `version` column in TasksTab /
// DoneTab. Plain lexicographic sort would order "v0.10.0" before "v0.9.0",
// which is wrong. This helper parses MAJOR.MINOR.PATCH and falls back to
// lexicographic compare when the value isn't recognizably semver.
//
// Accepts: `vX.Y.Z`, `X.Y.Z`, with optional leading whitespace. Anything
// trailing (e.g. `-rc1`, " — release notes") is ignored for the numeric
// compare and used only as a tie-breaker.

const SEMVER_RE = /^\s*v?(\d+)\.(\d+)\.(\d+)(.*)$/;

/**
 * Compare two version strings as SemVer triples. Returns negative if `a < b`,
 * positive if `a > b`, zero if equal.
 *
 * Non-semver values sort lexicographically (after any semver-looking values
 * if mixed). `null` / `undefined` / empty strings sort to the end.
 */
export function compareSemVer(a: string | null | undefined, b: string | null | undefined): number {
  const sa = (a ?? '').trim();
  const sb = (b ?? '').trim();
  if (sa === '' && sb === '') return 0;
  if (sa === '') return 1;
  if (sb === '') return -1;

  const ma = SEMVER_RE.exec(sa);
  const mb = SEMVER_RE.exec(sb);
  if (ma && mb) {
    const aMaj = +ma[1], aMin = +ma[2], aPat = +ma[3];
    const bMaj = +mb[1], bMin = +mb[2], bPat = +mb[3];
    if (aMaj !== bMaj) return aMaj - bMaj;
    if (aMin !== bMin) return aMin - bMin;
    if (aPat !== bPat) return aPat - bPat;
    // Same triple — fall back to trailing-tag compare (so `1.0.0-rc1` < `1.0.0`).
    const aTail = (ma[4] ?? '').trim();
    const bTail = (mb[4] ?? '').trim();
    if (aTail === '' && bTail !== '') return 1;   // empty tail = release > pre-release
    if (aTail !== '' && bTail === '') return -1;
    return aTail.localeCompare(bTail);
  }
  if (ma) return -1;  // semver values sort before non-semver
  if (mb) return 1;
  return sa.localeCompare(sb);
}
