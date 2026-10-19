/**
 * T-000174: a `_global` template is a rule when it is a skill template or the
 * global CLAUDE.md section — the same rule the backend keeps history for.
 */
import { describe, it, expect } from 'vitest';
import {
  GLOBAL_SECTION_FILE,
  ruleNameOf,
  isRuleFile,
  shortRepoName,
  formatRuleDate,
  noteKeyOf,
  ruleStats,
} from '../rules';

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

describe('rules screen helpers', () => {
  it('shortRepoName takes the last path segment of a label', () => {
    expect(shortRepoName('SgonnovDmGit/solo-dev-hub')).toBe('solo-dev-hub');
    expect(shortRepoName(String.raw`C:\Dev\local-only`)).toBe('local-only');
    expect(shortRepoName('plain')).toBe('plain');
  });

  it('formatRuleDate gives local DD.MM.YYYY HH:MM and passes junk through', () => {
    const d = new Date(2026, 9, 5, 7, 3);
    expect(formatRuleDate(d.toISOString())).toBe('05.10.2026 07:03');
    expect(formatRuleDate('not a date')).toBe('not a date');
  });

  it('noteKeyOf maps the four owner notes and leaves free text alone', () => {
    expect(noteKeyOf('accepted by the owner')).toBe('rules.note.accepted');
    expect(noteKeyOf('rejected by the owner')).toBe('rules.note.rejected');
    expect(noteKeyOf("superseded by the owner's edit")).toBe('rules.note.superseded');
    expect(noteKeyOf('resolved outside the app')).toBe('rules.note.outside');
    expect(noteKeyOf('please merge')).toBeNull();
  });

  it('ruleStats counts changed rules and conflicts', () => {
    expect(
      ruleStats([
        { changed: false, conflict: false },
        { changed: true, conflict: false },
        { changed: true, conflict: true },
      ]),
    ).toEqual({ total: 3, changed: 2, conflicts: 1 });
  });
});
