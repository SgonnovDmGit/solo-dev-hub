use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateFile {
    pub language_key: String,
    pub file_name: String,
    pub content: String,
    pub is_custom: bool,
    pub updated_at: String,
}

/// T-000203: outcome of saving a template. Saving a `skill.*.md.tmpl` also
/// renders the rule to every place an agent reads it from, so the caller can
/// report what actually reached the portfolio instead of just "saved".
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TemplateSaveOutcome {
    /// True when the saved file was a skill template and the render ran.
    pub rendered: bool,
    /// Skills written to `~/.claude/skills/`.
    pub skills: usize,
    /// Repos whose `docs/sdh_skills/` was refreshed.
    pub repos: usize,
    /// Per-target render failures. The DB write already succeeded — an edit is
    /// never lost because one render target was unwritable.
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateLanguage {
    pub language_key: String,
    pub display_name: String,
    pub file_count: i64,
}
