use serde::{Deserialize, Serialize};

/// v0.22.0 (T-000054): one-shot summary for the redesigned Stats tab.
/// Lifetime-only — no period filter. Fetched via `get_repo_stats_summary`
/// or `get_project_stats_summary` Tauri commands.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatsSummary {
    pub kpi: StatsKpi,
    pub categories: Vec<CategoryBar>,
    /// Some only for project-scope (top-3 hot repos within the project).
    /// None for repo-scope.
    pub top_hot_repos: Option<Vec<HotRepo>>,
    /// ISO date (YYYY-MM-DD) of MIN(bugs.created_at) within scope.
    /// Falls back to repositories.added_at (or projects.created_at for project-scope)
    /// when no bugs exist. None only when scope is fully empty.
    pub lifetime_since: Option<String>,
    pub days_history: i64,
    /// Some only for project-scope (count of repositories in project).
    pub repo_count: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatsKpi {
    pub active: i64,
    pub active_critical: i64,
    pub closed_total: i64,
    pub avg_attempts: f64,
    pub median_attempts: f64,
    /// 0..1 (multiply by 100 for %)
    pub fix_rate: f64,
    pub created_total: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CategoryBar {
    pub category: String,
    pub total: i64,
    pub closed: i64,
    /// 0..100 (UI displays as percentage)
    pub percent: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HotRepo {
    pub repo_id: i64,
    pub github_name: Option<String>,
    /// Mirrors Repository.description (Option<String>). Matches source-of-truth
    /// nullability — frontend can fallback to description if reposLookup misses.
    pub description: Option<String>,
    pub critical: i64,
    pub major: i64,
    pub active: i64,
    pub bugs_closed: i64,
    pub tasks_done: i64,
}
