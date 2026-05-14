use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct TimelineFilter {
    pub start_date: String,
    pub end_date: String,
    pub event_kinds: Option<Vec<String>>,
    pub project_ids: Option<Vec<i64>>,
    pub repo_ids: Option<Vec<i64>>,
    pub search: Option<String>,
}
