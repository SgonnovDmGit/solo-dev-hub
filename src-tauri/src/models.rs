use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub project_type: String, // "standard" | "microservice"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Repository {
    pub id: i64,
    pub project_id: Option<i64>,
    pub github_name: Option<String>,
    pub github_url: Option<String>,
    pub role: Option<String>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub last_pushed_at: Option<String>,
    pub added_at: String,
    pub updated_at: String,
    pub local_path: Option<String>,
    pub github_id: Option<i64>,
    pub deploy_target: Option<String>,
}

impl Repository {
    /// Display-friendly name. For GitHub repos returns the last segment of
    /// `github_name` (mirrors frontend `getDisplayName`). Falls back to
    /// `description`, then `<local>`.
    pub fn display_name(&self) -> String {
        if let Some(ref gh) = self.github_name {
            gh.rsplit('/').next().unwrap_or("").to_string()
        } else if let Some(ref desc) = self.description {
            desc.clone()
        } else {
            "<local>".to_string()
        }
    }

    /// F-033: canonical folder name used in cross-repo sync directory paths.
    /// For GitHub repos → last segment after '/' (e.g. `owner/foo-bar` → `foo-bar`).
    /// For local-only repos → `description` if set, else `local-<id>`.
    /// This is the single source-of-truth for naming sync subfolders like
    /// `client-requirements/<name>/` or `server-requirements/<parent-name>/`.
    pub fn canonical_folder_name(&self) -> String {
        if let Some(ref gh) = self.github_name {
            gh.rsplit('/').next().unwrap_or("").to_string()
        } else if let Some(ref desc) = self.description {
            desc.clone()
        } else {
            format!("local-{}", self.id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_repo(id: i64, github_name: Option<&str>, description: Option<&str>) -> Repository {
        Repository {
            id,
            project_id: None,
            github_name: github_name.map(String::from),
            github_url: None,
            role: None,
            description: description.map(String::from),
            language: None,
            last_pushed_at: None,
            added_at: String::new(),
            updated_at: String::new(),
            local_path: None,
            github_id: None,
            deploy_target: None,
        }
    }

    #[test]
    fn display_name_strips_owner_prefix() {
        let r = mk_repo(1, Some("SgonnovDM/swanqu"), None);
        assert_eq!(r.display_name(), "swanqu");
    }

    #[test]
    fn display_name_handles_no_slash() {
        let r = mk_repo(1, Some("solo"), None);
        assert_eq!(r.display_name(), "solo");
    }

    #[test]
    fn display_name_falls_back_to_description() {
        let r = mk_repo(1, None, Some("local-only-tool"));
        assert_eq!(r.display_name(), "local-only-tool");
    }

    #[test]
    fn display_name_final_fallback() {
        let r = mk_repo(1, None, None);
        assert_eq!(r.display_name(), "<local>");
    }
}

/// v0.18.0: one deploy environment per row. 1:N with repositories.
/// `name` is a user-chosen slug (prod/test/staging/custom). `extras` JSON
/// holds non-core placeholders (APP_PORT, NETWORK_NAME, COMPOSE_PROJECT, …).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployEnvironment {
    pub id: i64,
    pub repository_id: i64,
    pub name: String,
    pub workflow_name: String,
    pub image_tag: String,
    pub compose_service: String,
    pub domain: String,
    pub deploy_branch: String,
    pub sort_order: i64,
    #[serde(default)]
    pub extras: std::collections::HashMap<String, String>,
    pub updated_at: String,
}

/// v0.18.0: per-deploy per-secret flags. Values are NOT stored here —
/// they live in GitHub Secrets API (repo-scoped or env-scoped).
/// `role` is `Option<String>` because it's meaningful only when `included=true`;
/// in DB it's NULL when included=false (CHECK constraint still allows this).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploySecret {
    pub id: i64,
    pub deploy_env_id: i64,
    pub secret_name: String,
    pub role: Option<String>,
    pub included: bool,
    pub override_enabled: bool,
    pub sort_order: i64,
}

/// Args for creating a deploy environment via Tauri command.
/// `extras` optional; defaults to empty map.
#[derive(Debug, Deserialize, Clone)]
pub struct CreateDeployEnvironmentArgs {
    pub repository_id: i64,
    pub name: String,
    pub workflow_name: String,
    pub image_tag: String,
    pub compose_service: String,
    pub domain: String,
    pub deploy_branch: String,
    #[serde(default)]
    pub extras: std::collections::HashMap<String, String>,
}

/// Args for updating a deploy environment. `name` is read-only post-create,
/// so NOT present in this struct. Only placeholders + extras are mutable.
#[derive(Debug, Deserialize, Clone)]
pub struct UpdateDeployEnvironmentArgs {
    pub id: i64,
    pub workflow_name: String,
    pub image_tag: String,
    pub compose_service: String,
    pub domain: String,
    pub deploy_branch: String,
    #[serde(default)]
    pub extras: std::collections::HashMap<String, String>,
}

/// v0.18.0: meta.json v4 hint shape — role + scope per required_secret.
/// Passed from lib.rs (which parses meta.json) into db.rs ensure_deploy_secrets_populated.
#[derive(Debug, Clone, Deserialize)]
pub struct MetaSecretHint {
    pub name: String,
    pub role: String,     // "build" | "deploy" | "runtime"
    pub scope: String,    // "repo" | "environment"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RenderedFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WriteError {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WriteResult {
    pub written: Vec<String>,
    pub errors: Vec<WriteError>,
}

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
    pub id: String,              // display_id, "B-000042"
    pub date: String,            // YYYY-MM-DD (date portion of created_at)
    pub description: String,
    pub severity: String,
    pub category: String,
    pub status: String,
    pub fix_attempts: i32,
    pub comment: Option<String>,
    pub confirmed_at: Option<String>,  // YYYY-MM-DD or None
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

/// F-021: a single open/in-progress task parsed from `docs/todo.md`.
/// Format: `- [ ] <id> | <description> | <effort> | <priority> | <status>`
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TodoTask {
    pub id: String,
    pub description: String,
    pub effort: String,
    pub priority: String,
    pub status: String,
    pub created_at: String,  // YYYY-MM-DD; "" if 5-field legacy
}

#[derive(Debug, Serialize)]
pub struct ReadTodoResult {
    pub tasks: Vec<TodoTask>,
    pub warnings: Vec<String>,
}

/// F-021: a completed task parsed from `docs/done.md`.
/// Format (v0.13.9+):
///   `## <YYYY-MM-DD>`       ← date section header
///   `- <id> | <description> | <version>`  ← 3 pipe-separated fields, id may be empty
/// No `[x]` checkbox — the file itself is the "done" list.
/// If `id` is empty, the parser assigns `D-NNN` sequentially (in-memory only, file untouched).
/// `date` is inherited from the nearest preceding `## YYYY-MM-DD` header.
/// `version` is a free-form version tag (e.g. `v0.13.9`, `0.13.9`, or empty).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DoneTask {
    pub id: String,
    pub description: String,
    pub date: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct ReadDoneResult {
    pub tasks: Vec<DoneTask>,
    pub warnings: Vec<String>,
}

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
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyncResult {
    pub copied: usize,
    pub responses: usize,
    pub migrated: usize,
    pub errors: Vec<String>,
}

/// F-033: history record of a repository rename.
/// Written when `upsert_repository_with_outcome` detects a `github_name` change
/// for an existing repo (matched by `github_id`). Used by sync-preamble to
/// rename counterparty-side folders on the filesystem, and by the Settings UI
/// rename-log viewer. No `applied_at` column — sync is idempotent and replays
/// all entries each time, checking fs state rather than DB state.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoRename {
    pub id: i64,
    pub repository_id: i64,
    pub old_canonical: String,
    pub new_canonical: String,
    pub renamed_at: String,
}

/// T-000092: project-rename log. Symmetric to `RepoRename` but scoped to a
/// project rather than a repository. Used to replay `microservice-api/<X>/`
/// folder renames on parent server side when a microservice project is
/// renamed (the folder is keyed by project name, not repo canonical name).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectRename {
    pub id: i64,
    pub project_id: i64,
    pub old_name: String,
    pub new_name: String,
    pub renamed_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateFile {
    pub language_key: String,
    pub file_name: String,
    pub content: String,
    pub is_custom: bool,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TemplateLanguage {
    pub language_key: String,
    pub display_name: String,
    pub file_count: i64,
}

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct RequirementInfo {
    pub filename: String,
    pub direction: String,
    pub status: String,
    pub source_repo: String,
    pub target_repo: String,
    /// B-000018: true когда строка собрана reverse-lookup'ом со стороны ms-проекта
    /// (sender = parent server, recipient = текущий ms). UI скрывает ✓-кнопку для таких
    /// строк — confirm должен делать sender из своего собственного SyncScreen.
    #[serde(default)]
    pub is_reverse_lookup: bool,
}

/// Outcome of upserting a GitHub repo during sync.
/// - `Inserted` — brand new repo record written.
/// - `Merged` — existing local-only record updated with GitHub data (B-007 fix).
///   `merged_with_local_id` points at the DB id of the local-only row before
///   merge (same as `repo.id`), kept explicit for clarity; `local_path` is the
///   folder path the GitHub data was attached to (used by frontend toast).
/// - `Ambiguous` — 2+ local-only rows matched by basename. Nothing was written.
///   Frontend must prompt the user and call `resolve_merge_with_local` (picks
///   one) or `force_insert_github_repo` (creates a brand-new record).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpsertRepoOutcome {
    Inserted {
        repo: Repository,
    },
    Merged {
        repo: Repository,
        merged_with_local_id: i64,
        local_path: String,
    },
    Ambiguous {
        github_name: String,
        github_url: Option<String>,
        description: Option<String>,
        language: Option<String>,
        last_pushed_at: Option<String>,
        github_id: Option<i64>,
        candidates: Vec<Repository>,
    },
}

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
    pub sync_type: Option<String>,           // sync_event only: project_sync/tasks/secret/requirements
    pub deploy_action: Option<String>,       // deploy_event only: render/env_secret_set/env_secret_delete
    pub deploy_env_name: Option<String>,     // deploy_event only: environment name
    pub change_count: Option<i64>,           // sync_event only
}

/// v0.20.0: DB row for `tasks` table (mini-SoT for parsed todo.md/done.md).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Task {
    pub id: i64,
    pub repository_id: i64,
    pub task_id: String,
    pub prefix: String,
    pub description: String,
    pub effort: Option<f64>,
    pub priority: Option<String>,
    pub status: Option<String>,
    pub version: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TaskEvent {
    pub id: i64,
    pub task_id: i64,
    pub event_type: String,
    pub ts: String,
    pub from_status: Option<String>,
    pub to_status: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyncEvent {
    pub id: i64,
    pub repository_id: Option<i64>,
    pub sync_type: String,
    pub ts: String,
    pub change_count: i64,
    pub details: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeployEvent {
    pub id: i64,
    pub deploy_env_id: Option<i64>,
    pub repository_id: i64,
    pub action: String,
    pub ts: String,
    pub details: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TimelineFilter {
    pub start_date: String,
    pub end_date: String,
    pub event_kinds: Option<Vec<String>>,
    pub project_ids: Option<Vec<i64>>,
    pub repo_ids: Option<Vec<i64>>,
    pub search: Option<String>,
}

// ── F-013 Project graph DTOs ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    Repo,
    Project,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    InProject,
    CrossProjectMs,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphNode {
    pub id: String,                  // "repo:42" or "project:7"
    pub label: String,               // display_name (repo) or project.name
    pub kind: GraphNodeKind,
    pub role: Option<String>,        // 'server' | 'client' | 'landing' | 'tool' | 'microservice' | None
    pub repo_id: Option<i64>,
    pub project_id: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub kind: GraphEdgeKind,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectGraph {
    pub center: Option<GraphNode>,
    pub ring: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[cfg(test)]
mod deploy_types_tests {
    use super::*;

    #[test]
    fn test_deploy_environment_serde_roundtrip() {
        let env = DeployEnvironment {
            id: 42,
            repository_id: 1,
            name: "prod".to_string(),
            workflow_name: "Deploy".to_string(),
            image_tag: "latest".to_string(),
            compose_service: "backend".to_string(),
            domain: "x.com".to_string(),
            deploy_branch: "master".to_string(),
            sort_order: 0,
            extras: Default::default(),
            updated_at: "2026-04-25T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&env).unwrap();
        let back: DeployEnvironment = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 42);
        assert_eq!(back.name, "prod");
    }

    #[test]
    fn test_deploy_secret_serde_roundtrip() {
        let s = DeploySecret {
            id: 1,
            deploy_env_id: 42,
            secret_name: "SSH_HOST".to_string(),
            role: Some("deploy".to_string()),
            included: true,
            override_enabled: true,
            sort_order: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: DeploySecret = serde_json::from_str(&json).unwrap();
        assert_eq!(back.secret_name, "SSH_HOST");
        assert!(back.included);
    }

    #[test]
    fn test_deploy_secret_role_none_when_not_included() {
        // Role is Option<String> — NULL in DB when included=false.
        let s = DeploySecret {
            id: 1,
            deploy_env_id: 1,
            secret_name: "X".to_string(),
            role: None,
            included: false,
            override_enabled: false,
            sort_order: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"role\":null"));
    }
}
