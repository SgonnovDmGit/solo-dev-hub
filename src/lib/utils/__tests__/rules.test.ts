/**
 * T-000174: a `_global` template is a rule when it is a skill template or the
 * global CLAUDE.md section — the same rule the backend keeps history for.
 */
import { describe, it, expect } from 'vitest';
import { GLOBAL_SECTION_FILE, ruleNameOf, isRuleFile } from '../rules';

describe('ruleNameOf', () => {
  it('names the global CLAUDE.md section', () => {
    expect(GLOBAL_SECTION_FILE).toBe('claude.md.global.tmpl');
    expect(ruleNameOf('claude.md.global.tmpl')).toBe('claude.md.global');
  });

  it('names a skill by its middle segment', () => {
    expect(ruleNameOf('skill.sdh-retro.md.tmpl')).toBe('sdh-retro');
  });

  it('is null for a skill template with an empty name', () => {
    expect(ruleNameOf('skill.md.tmpl')).toBeNull();
  });

  it('is null for files that are not rules', () => {
    expect(ruleNameOf('.gitignore.tmpl')).toBeNull();
    expect(ruleNameOf('claude.md.section.tmpl')).toBeNull();
  });
});

describe('isRuleFile', () => {
  it('is true for the global section and for skills', () => {
    expect(isRuleFile('claude.md.global.tmpl')).toBe(true);
    expect(isRuleFile('skill.sdh-retro.md.tmpl')).toBe(true);
  });

  it('is false for an empty skill name and other templates', () => {
    expect(isRuleFile('skill.md.tmpl')).toBe(false);
    expect(isRuleFile('.gitignore.tmpl')).toBe(false);
    expect(isRuleFile('claude.md.section.tmpl')).toBe(false);
  });
});
