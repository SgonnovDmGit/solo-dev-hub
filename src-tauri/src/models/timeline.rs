use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct TimelineFilter {
    pub start_date: String,
    pub end_date: String,
    pub event_kinds: Option<Vec<String>>,
    pub project_ids: Option<Vec<i64>>,
    pub repo_ids: Option<Vec<i64>>,
    pub search: Option<String>,
    /// F-000044: `"me"` = events of this install (no author), otherwise a
    /// GitHub login. `None` = everybody.
    #[serde(default)]
    pub author: Option<String>,
}
