import { tf } from '$lib/i18n';
import type { RulesOverview } from '$lib/types';
import { ruleStats } from './rules';

/** One-line portfolio summary shown in the Rules header and on the Settings card. */
export function ruleSummary(overview: RulesOverview): string {
  const s = ruleStats(overview.rules);
  if (s.changed === 0 && s.conflicts === 0) {
    return tf('rules.summaryClean', s.total, overview.repos);
  }
  return tf('rules.summary', s.total, s.changed, s.conflicts, overview.repos);
}

/** The first rule with an open conflict, for the Settings card link. */
export function firstConflictRule(overview: RulesOverview): string | null {
  return overview.rules.find((r) => r.conflict)?.rule ?? null;
}
