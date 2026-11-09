import { invoke } from '@tauri-apps/api/core';
import type {
  TemplateFile,
  TemplateLanguage,
  TemplateSaveOutcome,
  RulesRefresh,
  RulesOverview,
  RuleEvent,
  RuleConflictSide,
  RuleProposalView,
  RuleAcceptance,
} from '$lib/types';


// ── Templates (0.6.0) ─────────────────────────────────────────────────────────

export async function listTemplateLanguages(): Promise<TemplateLanguage[]> {
  return invoke<TemplateLanguage[]>('list_template_languages');
}

export async function listTemplateFiles(languageKey: string): Promise<TemplateFile[]> {
  return invoke<TemplateFile[]>('list_template_files', { languageKey });
}

export async function getTemplateFile(languageKey: string, fileName: string): Promise<TemplateFile | null> {
  return invoke<TemplateFile | null>('get_template_file', { languageKey, fileName });
}

export async function saveTemplateFile(languageKey: string, fileName: string, content: string): Promise<TemplateSaveOutcome> {
  return invoke<TemplateSaveOutcome>('save_template_file', { languageKey, fileName, content });
}

/** T-000212: resetting a skill re-renders it everywhere, so the reset reports
 *  what reached the portfolio exactly as a save does. */
export async function resetTemplateFile(languageKey: string, fileName: string): Promise<TemplateSaveOutcome> {
  return invoke<TemplateSaveOutcome>('reset_template_file', { languageKey, fileName });
}

/** T-000207: what the startup rule delivery found — returned once, then cleared. */
export async function takeStartupRulesReport(): Promise<RulesRefresh | null> {
  return invoke<RulesRefresh | null>('take_startup_rules_report');
}

/** T-000174: every workflow rule with its status, plus how many repos receive them. */
export async function listRules(): Promise<RulesOverview> {
  return invoke<RulesOverview>('list_rules');
}

/** T-000174: a rule's history, newest first. */
export async function listRuleEvents(rule: string): Promise<RuleEvent[]> {
  return invoke<RuleEvent[]>('list_rule_events', { rule });
}

/** T-000206: the sides of a hand-back conflict, read live from the repos. */
export async function getRuleConflict(rule: string): Promise<RuleConflictSide[]> {
  return invoke<RuleConflictSide[]>('get_rule_conflict', { rule });
}

/** T-000206: decide a conflict. `accept` = a side's `repo` label to take its text;
 *  null rejects every side. Rejects with `rule_conflict_gone` (nothing open) or
 *  `rule_conflict_side_gone` (that side no longer exists). */
export async function resolveRuleConflict(rule: string, accept: string | null): Promise<TemplateSaveOutcome> {
  return invoke<TemplateSaveOutcome>('resolve_rule_conflict', { rule, accept });
}

// ── Rule proposals (T-000241, contributor mode) ─────────────────────────────

/** Proposals of one shared repo — the owner's read comes off the local dev
 *  tip, the contributor's off its own rows; the sweeps ran first. */
export async function listRuleProposals(repoId: number): Promise<RuleProposalView[]> {
  return invoke<RuleProposalView[]>('list_rule_proposals', { repoId });
}

/** Owner: the decision pipeline rows for the repo. */
export async function listRuleAcceptances(repoId: number): Promise<RuleAcceptance[]> {
  return invoke<RuleAcceptance[]>('list_rule_acceptances', { repoId });
}

/** Owner decision on an open proposal version. `expectedRev` is the blob sha
 *  of the version seen — the CAS of the decision; `action` = accept | reject. */
export async function decideRuleProposal(
  repoId: number,
  rule: string,
  proposerLogin: string,
  expectedRev: string,
  action: 'accept' | 'reject',
): Promise<RuleAcceptance> {
  return invoke<RuleAcceptance>('decide_rule_proposal', { repoId, rule, proposerLogin, expectedRev, action });
}

/** Contributor: withdraw an own open proposal — uploads on the next pass. */
export async function withdrawRuleProposal(repoId: number, fileName: string): Promise<void> {
  return invoke<void>('withdraw_rule_proposal', { repoId, fileName });
}

/** The explicit action a `conflicted` acceptance waits for:
 *  `apply` forces the proposal snapshot over the diverged canon, `dismiss`
 *  leaves the local application (`superseded`) while the receipt keeps
 *  publishing. */
export async function resolveRuleAcceptanceConflict(
  repoId: number,
  rule: string,
  proposerLogin: string,
  decidedRev: string,
  resolution: 'apply' | 'dismiss',
): Promise<RuleAcceptance> {
  return invoke<RuleAcceptance>('resolve_rule_acceptance_conflict', {
    repoId,
    rule,
    proposerLogin,
    decidedRev,
    resolution,
  });
}
