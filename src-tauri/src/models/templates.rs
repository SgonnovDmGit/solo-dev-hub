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
    /// T-000207: hand-backs of OTHER skills taken up before the render.
    #[serde(default)]
    pub taken: Vec<String>,
    /// T-000207: markers found beside a copy already equal to the rule —
    /// `<skill> (<repo>)`; the edit they stood for never reached the database.
    #[serde(default)]
    pub lost: Vec<String>,
    /// T-000207: pending hand-backs of the saved skill, set aside because the
    /// owner's Settings edit wins — `<skill> (<repo>)`.
    #[serde(default)]
    pub superseded: Vec<String>,
}

/// T-000207: report of one `sync::refresh_rules` pass — collect the approved
/// hand-backs across the portfolio, then render. Field names are the JSON keys
/// the frontend reads (`RulesRefresh` in `types/templates.ts`).
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct RulesRefresh {
    /// Skills whose text was taken back out of a flagged repo copy.
    pub taken: Vec<String>,
    /// `<skill> (<repo>)`: a marker beside a copy already equal to the rule.
    /// The edit was lost before collection; the marker is cleared, the rule
    /// left untouched.
    pub lost: Vec<String>,
    /// `<skill> (<repo>)`: a pending hand-back of the skill the owner just
    /// saved in Settings — not taken, marker cleared.
    pub superseded: Vec<String>,
    /// Non-fatal per-item failures (collection and render).
    pub errors: Vec<String>,
    /// Skills written to `~/.claude/skills/` (0 when that target was not rendered).
    pub skills: usize,
    /// Repos whose `docs/sdh_skills/` was rendered.
    pub repos: usize,
}

impl RulesRefresh {
    /// True when there is nothing worth telling the owner about.
    pub fn is_quiet(&self) -> bool {
        self.taken.is_empty()
            && self.lost.is_empty()
            && self.superseded.is_empty()
            && self.errors.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateLanguage {
    pub language_key: String,
    pub display_name: String,
    pub file_count: i64,
}
