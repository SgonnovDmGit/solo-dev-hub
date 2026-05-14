use serde::{Deserialize, Serialize};

use super::Repository;

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
pub struct SyncResult {
    pub copied: usize,
    pub responses: usize,
    pub migrated: usize,
    pub errors: Vec<String>,
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
