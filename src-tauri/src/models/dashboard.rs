use serde::{Deserialize, Serialize};

// ── Dashboard types (v0.17.0) ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Period {
    pub start: String,  // YYYY-MM-DD, inclusive
    pub end: String,    // YYYY-MM-DD, inclusive
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardFilter {
    pub period: Period,
    pub compare_period: Option<Period>,  // None for Custom or d<1
    /// None or empty => aggregate over ALL repos (deselect-all behaves same as all).
    /// Some(ids) => only repos belonging to these project_ids.
    pub project_ids: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KpiCard {
    pub value: Option<f64>,          // None => display "—"
    pub prev_value: Option<f64>,     // None => no compare arrow
    pub critical_count: Option<i64>, // for KPI 1 subtitle only
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopHotProject {
    pub project_id: i64,
    pub name: String,
    pub critical: i64,
    pub major: i64,
    pub active: i64,
    pub bugs_closed: i64,
    pub tasks_done: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyFlowDay {
    pub date: String,         // YYYY-MM-DD
    pub opened: Option<i64>,  // bugs opened (only for bugs chart; None for tasks)
    pub closed: Option<i64>,  // bugs closed (only for bugs chart)
    pub done: Option<i64>,    // tasks done (only for tasks chart)
    pub is_future: bool,      // if date > today
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryEfficiencyRow {
    pub category: String,              // enum string
    pub touched_in_period: i64,
    pub closed_in_period: i64,
    pub attempts_in_period: i64,
    pub resolution_rate: Option<f64>,  // None if touched=0 (hidden in UI)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub active_bugs: KpiCard,
    pub closed_in_period: KpiCard,
    pub tasks_done: KpiCard,
    pub solve_rate: KpiCard,
    pub attempts_per_closed: KpiCard,
    pub top_hot: Vec<TopHotProject>,  // empty if single project selected
    pub bugs_daily: Vec<DailyFlowDay>,
    pub tasks_daily: Vec<DailyFlowDay>,
    pub categories: Vec<CategoryEfficiencyRow>,
}

/// v0.19.0: lightweight event row for the Dashboard activity feed.
/// Sourced from `bug_events` (status transitions), `repo_renames` (rename log),
/// `task_events`, `sync_events`, `deploy_events` via UNION ALL.
/// `kind` distinguishes the source; not all fields are populated for each kind.
#[derive(Debug, Serialize, Clone)]
pub struct ActivityEvent {
    pub kind: String,                        // "bug_event" | "repo_rename" | "task_event" | "sync_event" | "deploy_event"
    pub event_type: String,                  // bug: created/taken/entered_testing/confirmed/rejected/reopened; rename: "renamed"; task: created/taken/review/done/reopened
    pub ts: String,                          // ISO8601 string
    pub repo_id: Option<i64>,               // None for portfolio-wide sync_events
    pub repo_display_name: Option<String>,   // canonical (last segment of github_name); None if repo deleted
    pub bug_display_id: Option<String>,      // "B-NNNNNN" for bug_event; None otherwise
    pub task_display_id: Option<String>,     // "T-NNN" / "F-NNN" for task_event; None otherwise
    pub old_canonical: Option<String>,       // rename only
    pub new_canonical: Option<String>,       // rename only
    pub sync_type: Option<String>,           // sync_event only: project_sync/tasks/secret/requirements/migration
    pub deploy_action: Option<String>,       // deploy_event only: render/env_secret_set/env_secret_delete
    pub deploy_env_name: Option<String>,     // deploy_event only: environment name
    pub change_count: Option<i64>,           // sync_event only
    /// v0.31.0 (T-000103 Task 6): structured JSON payload for sync_events.
    /// Currently used by `sync_type='migration'` to surface v25 placeholder
    /// conflict info on the activity-feed render branch:
    ///   {"conflicts":[{"key":"GO_VERSION","kept_env":"prod","kept_value":"1.26-alpine","discarded":[{"env":"test","value":"alpine"}]}]}
    /// Other sync_type values may add structured payload here later.
    /// `None` for non-sync events and for older sync_events with NULL details.
    pub details: Option<String>,
}
