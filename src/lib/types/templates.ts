export interface TemplateFile {
  language_key: string;
  file_name: string;
  content: string;
  is_custom: boolean;
  updated_at: string;
}

/** T-000203: saving a `skill.*.md.tmpl` also delivers the rule to every place an
 *  agent reads it from, so the save reports what actually reached the portfolio. */
export interface TemplateSaveOutcome {
  rendered: boolean;
  skills: number;
  repos: number;
  errors: string[];
}

export interface TemplateLanguage {
  language_key: string;
  display_name: string;
  file_count: number;
}
