export interface TemplateFile {
  language_key: string;
  file_name: string;
  content: string;
  is_custom: boolean;
  updated_at: string;
}

export interface TemplateLanguage {
  language_key: string;
  display_name: string;
  file_count: number;
}
