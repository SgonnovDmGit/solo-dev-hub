import { invoke } from '@tauri-apps/api/core';
import type { TemplateFile, TemplateLanguage, TemplateSaveOutcome, RulesRefresh } from '$lib/types';


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

export async function resetTemplateFile(languageKey: string, fileName: string): Promise<void> {
  return invoke<void>('reset_template_file', { languageKey, fileName });
}

/** T-000207: what the startup rule delivery found — returned once, then cleared. */
export async function takeStartupRulesReport(): Promise<RulesRefresh | null> {
  return invoke<RulesRefresh | null>('take_startup_rules_report');
}
