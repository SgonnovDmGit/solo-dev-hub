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
    pub errors: Vec<super::SyncIssue>,
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
    pub errors: Vec<super::SyncIssue>,
    /// Skills written to `~/.claude/skills/` (0 when that target was not rendered).
    pub skills: usize,
    /// Repos whose `docs/sdh_skills/` was rendered.
    pub repos: usize,
    /// T-000206: skills whose hand-back conflict was already open before this
    /// pass. It was reported once, when it opened; a repeat is not reported
    /// again — neither as `RuleConflict` nor as `RuleHeld` for its held
    /// copies — so a conflict no longer fires a toast on every sync.
    #[serde(default)]
    pub quiet_conflicts: Vec<String>,
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

/// T-000174: one entry of a rule's history (`rule_events`, migration v33).
/// `rule` is a skill name or `claude.md.global`; `kind` is one of the
/// `db::rules::rule_event` constants.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RuleEvent {
    pub id: i64,
    pub rule: String,
    pub kind: String,
    /// `rule_copy_label` of the repo copy involved; `None` for an edit made in
    /// the app.
    pub repo: Option<String>,
    /// The approval line from the marker (`taken`), or how a conflict was
    /// resolved (`conflict_resolved`).
    pub note: Option<String>,
    pub created_at: String,
}

/// T-000174: one workflow rule as the Rules screen lists it. A rule is a skill
/// or the global CLAUDE.md section — see `sync::rule_name_of`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RuleInfo {
    /// `sdh-*` skill name, or `claude.md.global`.
    pub rule: String,
    /// The `_global` template file that stores it.
    pub file_name: String,
    /// The text in force.
    pub content: String,
    /// The text bundled with this build. `None` when the bundle has no such file.
    pub canon: Option<String>,
    /// The text differs from `canon` — not the same as `is_custom`, which stays
    /// set after the owner saves text that happens to equal the bundle.
    pub changed: bool,
    /// The rule has an open hand-back conflict.
    pub conflict: bool,
    /// T-000224: login of the owner whose repo this install takes the rule's
    /// text from; the rule is read-only here. `None` when the install owns it.
    #[serde(default)]
    pub follows_main: Option<String>,
    /// Where the text came from (`db::rules::latest_origin_event`); `None` when
    /// it predates the history.
    pub origin: Option<RuleEvent>,
}

/// T-000174: the Rules screen's whole listing.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RulesOverview {
    /// The global CLAUDE.md section first, then the skills alphabetically.
    pub rules: Vec<RuleInfo>,
    /// Repos a rule is delivered to: those with a local checkout that exists on
    /// this machine.
    pub repos: usize,
}

/// T-000206: one side of a hand-back conflict, read live from a repo's
/// `docs/sdh_skills/`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct RuleConflictSide {
    /// `rule_copy_label` of the repo. Also the key `resolve_rule_conflict`
    /// accepts a side by.
    pub repo: String,
    /// The approval marker's modification time, RFC 3339. `None` when the
    /// filesystem would not say.
    pub marked_at: Option<String>,
    /// The approval line the agent put in the marker, trimmed.
    pub note: Option<String>,
    /// The handed-back text.
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateLanguage {
    pub language_key: String,
    pub display_name: String,
    pub file_count: i64,
}
