use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileBugNote {
    pub id: String,
    pub date: String,
    pub description: String,
    pub severity: String,
    pub category: String,
    pub status: String,
    pub fix_attempts: i32,
    pub comment: Option<String>,
}

/// v0.16.0: Full row from `bugs` table (SQLite = SoT for bugs).
/// `numeric_id` is the integer part of `display_id` (e.g. "B-000042" → 42);
/// stored denormalized for MD I/O speed and query indexing.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Bug {
    pub id: i64,
    pub repository_id: i64,
    pub numeric_id: i64,
    pub display_id: String,
    pub created_at: String,
    pub description: String,
    pub severity: String,
    pub category: String,
    pub status: String,
    pub fix_attempts: i32,
    pub comment: Option<String>,
    pub confirmed_at: Option<String>,
    /// v0.21.1: timestamp when LLM removed confirmed-row from MD (acknowledgement
    /// that LLM saw the confirmation). Set by reconcile when confirmed bug is
    /// missing from MD. NULL = not yet acknowledged → still visible in MD.
    pub archived_from_md_at: Option<String>,
}

/// v0.16.0: Frontend DTO for bugs list. 9 fields including `confirmed_at`
/// (not present in MD-facing `FileBugNote`). UI-only — never serialized to MD.
#[derive(Debug, Serialize, Clone)]
pub struct BugView {
    pub id: String,   // display_id, "B-000042"
    pub date: String, // YYYY-MM-DD (date portion of created_at)
    pub description: String,
    pub severity: String,
    pub category: String,
    pub status: String,
    pub fix_attempts: i32,
    pub comment: Option<String>,
    pub confirmed_at: Option<String>, // YYYY-MM-DD or None
}

impl Bug {
    /// Convert a full `Bug` row to frontend DTO. Date fields are truncated to
    /// `YYYY-MM-DD` — UI shows dates, not timestamps. Full ISO timestamps stay in DB
    /// for future use (e.g. "time in testing" metrics).
    pub fn to_view(&self) -> BugView {
        BugView {
            id: self.display_id.clone(),
            date: date_part(&self.created_at),
            description: self.description.clone(),
            severity: self.severity.clone(),
            category: self.category.clone(),
            status: self.status.clone(),
            fix_attempts: self.fix_attempts,
            comment: self.comment.clone(),
            confirmed_at: self.confirmed_at.as_deref().map(date_part),
        }
    }
}

/// v0.16.0: Extract YYYY-MM-DD from an ISO8601 or free-form timestamp.
/// `"2026-03-29T00:00:00Z"` → `"2026-03-29"`; `"2026-03-29"` → `"2026-03-29"`.
/// Works on anything whose first 10 chars are the date.
fn date_part(ts: &str) -> String {
    ts.get(..10).unwrap_or(ts).to_string()
}

/// v0.16.0: Result of lazy MD→DB migration for a repo. Frontend shows a toast
/// based on this. `already=true` means the repo was already migrated, no-op.
#[derive(Debug, Serialize, Clone)]
pub struct MigrationReport {
    pub imported: u32,
    pub confirmed_archived: u32,
    pub already: bool,
}

#[derive(Debug, Serialize)]
pub struct ReadBugsResult {
    pub bugs: Vec<FileBugNote>,
    pub warnings: Vec<String>,
}
