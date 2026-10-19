import { invoke } from '@tauri-apps/api/core';
import type {
  TemplateFile,
  TemplateLanguage,
  TemplateSaveOutcome,
  RulesRefresh,
  RulesOverview,
  RuleEvent,
  RuleConflictSide,
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
