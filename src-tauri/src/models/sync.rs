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

/// T-000198: family of a non-fatal sync issue. Serialised as a snake_case
/// string (`"copy_failed"`); the frontend translates it via the i18n key
/// `syncIssue.<code>` (`formatSyncIssue` in `utils/sync-issues.ts`). Adding a
/// variant means adding that key in ru + en; an unknown code falls back to
/// English text built from `subject` / `detail`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncIssueCode {
    /// A file or folder could not be copied/mirrored. subject = file, target = destination.
    CopyFailed,
    /// A repo has no local_path, so it was skipped. subject = repo.
    NoLocalPath,
    /// A repo's root folder is missing or not a folder. subject = repo, detail = path.
    RootMissing,
    /// Rename replay found both the old and new folder. subject = old, target = new,
    /// detail = the parent folder they sit in.
    RenameCollision,
    /// Rename replay failed on the filesystem. subject = old, target = new.
    RenameReplayFailed,
    /// An internal read of app data failed. subject = the entity (may be empty),
    /// target = one of the `lookup::*` tokens naming what was being read.
    LookupFailed,
    /// A project lacks the repo of a role the sync needs. subject = project, target = role.
    MissingCounterpart,
    /// An app-managed file could not be written or reconciled. subject = repo, target = file.
    ManagedFileFailed,
    /// A peer-channel (InOutBox) step failed. subject = repo / file.
    PeerChannelFailed,
    /// Two writers left different versions of one item. subject = item, target = the sides.
    VersionConflict,
    /// Two repos handed back different texts of one rule (skill). subject = skill,
    /// target = the sides. Resolved in the app: Settings -> AI rules.
    RuleConflict,
    /// A `.impl.md` auto-close of REQ pairs failed. subject = counterpart.
    ReqAutoCloseFailed,
    /// A one-time folder/file migration failed or was left alone. subject = what.
    MigrationFailed,
    /// A flat legacy REQ file had no matching parent copy (possibly stale) and
    /// was left in the flat folder. subject = file.
    MigrationLeftInPlace,
    /// A flat legacy REQ file differs from its copy on the parent(s) and was
    /// left in the flat folder. subject = file, target = the parents.
    MigrationDiverged,
    /// The auto-commit of synced files failed. subject = repo, detail = git output.
    AutocommitFailed,
    /// A rule copy is flagged but names no known skill. subject = repo, target = marker.
    RuleUnknownSkill,
    /// A flagged rule copy was left as is: unresolved hand-back. subject = repo, target = skill.
    RuleHeld,
    /// A hand-back step (read / store / clear marker) failed. subject = skill or repo.
    RuleFailed,
}

/// T-000198: one non-fatal issue found by a sync-like pass. Replaces free
/// English strings so the UI can show each one translated, one per line.
/// `detail` is raw OS / git / DB text and is never translated.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SyncIssue {
    pub code: SyncIssueCode,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// T-000198: what a `lookup_failed` issue was reading — the fixed `target`
/// tokens the frontend translates (`syncIssueLookup.<token>`).
pub mod lookup {
    pub const RENAMES: &str = "renames";
    pub const REPOS: &str = "repos";
    pub const PROJECT: &str = "project";
    pub const SKILLS: &str = "skills";
    pub const SEEDING: &str = "seeding";
    pub const PEERS: &str = "peers";
    pub const CODEX_SOURCE: &str = "codex_source";
    pub const HOME_DIR: &str = "home_dir";
    pub const AUTOCOMMIT_REPOS: &str = "autocommit_repos";
}

impl SyncIssue {
    /// A failed internal read of `what` (a `lookup::*` token) for `subject`.
    pub fn lookup(what: &'static str, subject: impl Into<String>) -> Self {
        Self::new(SyncIssueCode::LookupFailed, subject).target(what)
    }

    pub fn new(code: SyncIssueCode, subject: impl Into<String>) -> Self {
        Self {
            code,
            subject: subject.into(),
            target: None,
            detail: None,
        }
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn detail(mut self, detail: impl std::fmt::Display) -> Self {
        self.detail = Some(detail.to_string());
        self
    }
}

/// English rendering — for logs and test failure messages, not for the UI.
impl std::fmt::Display for SyncIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = serde_json::to_value(self.code)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        write!(f, "{}: {}", code, self.subject)?;
        if let Some(t) = &self.target {
            write!(f, " -> {}", t)?;
        }
        if let Some(d) = &self.detail {
            write!(f, " ({})", d)?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SyncResult {
    pub copied: usize,
    pub responses: usize,
    pub migrated: usize,
    pub errors: Vec<SyncIssue>,
    /// F-000039: base REQ filenames (`REQ-NNN_slug.md`) whose pair the sender
    /// acknowledged via a `.impl.md` drop and was auto-torn-down this sync.
    #[serde(default)]
    pub auto_closed: Vec<String>,
    /// T-000137: display names of repos that received an SDH auto-commit of
    /// synced cross-repo files this sync (opted-in repos with a set
    /// `autocommit_branch`, on the correct branch, with actual changes).
    #[serde(default)]
    pub committed: Vec<String>,
    /// T-000205: names of skills whose text was taken back out of a repo's own
    /// copy this sync, because an agent edited it and flagged it approved.
    /// Reported on purpose: nothing verifies the approval, so the uptake has to
    /// at least be visible after the fact.
    #[serde(default)]
    pub rules_taken: Vec<String>,
    /// T-000207: `<skill> (<repo>)` — markers found beside a copy already equal
    /// to the rule. The edit they stood for was lost before it was collected;
    /// the marker is cleared and the rule left as it was.
    #[serde(default)]
    pub rules_lost: Vec<String>,
}

/// T-000207: result of `init_docs_for_repo` — the files written plus what the
/// rule refresh found, since that command renders the repo's rule copies too.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InitDocsOutcome {
    pub updated: Vec<String>,
    pub rules: super::RulesRefresh,
}

/// F-000041: report from `untrack_files`. `untracked` = total file count
/// successfully removed from the git index across all chunks; `errors` =
/// per-chunk stderr captures (chunk-level granularity, not per-file).
#[derive(Debug, Clone, Serialize)]
pub struct UntrackReport {
    pub untracked: usize,
    pub errors: Vec<String>,
}

/// F-000041: payload for the list-gitignored Tauri command. `repo_state` is
/// the stringified `git_ops::RepoState` — UI gates the Untrack button on
/// `"clean"`. `other_staged_count` excludes files about to be untracked.
#[derive(Debug, Clone, Serialize)]
pub struct GitignoredListing {
    pub files: Vec<String>,
    pub repo_state: String,
    pub other_staged_count: usize,
}

// Read-side DTO for sync_events — currently consumed only by tests that
// verify the event log; gated so it doesn't read as dead code in release builds.
#[cfg(test)]
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
    /// (sender = parent server, recipient = текущий ms). После B-000021 confirm-✓
    /// работает с обеих сторон одинаково — поле осталось informational/audit-flag
    /// и в UI больше не гейтит кнопку.
    #[serde(default)]
    pub is_reverse_lookup: bool,
    /// F-000039: true when the sender dropped a `REQ-NNN_slug.impl.md`
    /// acknowledgement alongside this REQ — signals the pair is about to be
    /// auto-closed on next sync.
    #[serde(default)]
    pub has_impl: bool,
}

/// T-000173: one codex BROADCAST edge — a codex svod delivered to a consumer
/// repo on a given lane. Distinct from REQ pairs (`RequirementInfo`): this is
/// one-way broadcast visibility for the sync screen. snake_case JSON keys to
/// match `RequirementInfo` (frontend reads snake_case).
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct CodexEdgeInfo {
    pub codex_name: String,    // codex project name
    pub consumer_name: String, // consumer project name
    pub lane: String,          // "server" | "admin_client"
    pub source_repo: String,   // codex lane repo display_name()
    pub target_repo: String,   // consumer lane repo display_name()
    pub inbox: String,         // "docs/codex/<codex_name>/"
    pub file_count: usize,     // files in the codex source folder (0 if unresolved)
    pub direction: String,     // "incoming" | "outgoing"
    pub status: String,        // "synced" (inbox mirrors source) | "pending"
}

/// T-000191: an explicit peer link between two repositories — the InOutBox
/// channel between equals. Every pair is explicit — nothing is inferred from a
/// shared role, so opening a channel is always a decision. Stored normalised
/// (`repo_a_id < repo_b_id`) because the relation is symmetric.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeerLink {
    pub id: i64,
    pub repo_a_id: i64,
    pub repo_b_id: i64,
    pub created_at: String,
}

/// T-000193: one peer channel as shown in the sync screen — a pair of repos and
/// the state of the three folders between them. snake_case keys to match the
/// other sync DTOs the frontend reads.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct PeerChannelInfo {
    pub self_repo_id: i64,
    pub peer_repo_id: i64,
    pub self_repo: String,
    pub peer_repo: String,
    /// Project the peer belongs to; empty when the peer has none.
    pub peer_project: String,
    /// Role shared by the pair; empty when unknown.
    pub role: String,
    /// Channel folder on this side, e.g. "docs/InOutBox/.../beta/".
    pub folder: String,
    pub outbox_count: usize,
    pub inbox_count: usize,
    pub pact_count: usize,
    /// "synced" | "pending" | "unresolved" (a side is missing from this disk).
    pub status: String,
    /// Whether this side's `registry.md` exists. The app never creates it — the
    /// agent does, on the first exchange — so its absence is worth surfacing.
    pub registry_present: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// T-000198: the frontend reads `code` as a plain snake_case string and
    /// treats absent `target` / `detail` as missing keys — a mismatch with
    /// `types/sync.ts` would fail silently, so the shape is pinned here.
    #[test]
    fn test_sync_issue_serde_shape() {
        let full = SyncIssue::new(SyncIssueCode::CopyFailed, "REQ-001.md")
            .target("srv")
            .detail("Access is denied. (os error 5)");
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            serde_json::json!({
                "code": "copy_failed",
                "subject": "REQ-001.md",
                "target": "srv",
                "detail": "Access is denied. (os error 5)"
            })
        );

        let bare = SyncIssue::new(SyncIssueCode::NoLocalPath, "alpha");
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            serde_json::json!({ "code": "no_local_path", "subject": "alpha" })
        );

        for (code, wire) in [
            (SyncIssueCode::RootMissing, "root_missing"),
            (SyncIssueCode::RenameCollision, "rename_collision"),
            (SyncIssueCode::RenameReplayFailed, "rename_replay_failed"),
            (SyncIssueCode::LookupFailed, "lookup_failed"),
            (SyncIssueCode::MissingCounterpart, "missing_counterpart"),
            (SyncIssueCode::ManagedFileFailed, "managed_file_failed"),
            (SyncIssueCode::PeerChannelFailed, "peer_channel_failed"),
            (SyncIssueCode::VersionConflict, "version_conflict"),
            (SyncIssueCode::RuleConflict, "rule_conflict"),
            (SyncIssueCode::ReqAutoCloseFailed, "req_auto_close_failed"),
            (SyncIssueCode::MigrationFailed, "migration_failed"),
            (
                SyncIssueCode::MigrationLeftInPlace,
                "migration_left_in_place",
            ),
            (SyncIssueCode::MigrationDiverged, "migration_diverged"),
            (SyncIssueCode::AutocommitFailed, "autocommit_failed"),
            (SyncIssueCode::RuleUnknownSkill, "rule_unknown_skill"),
            (SyncIssueCode::RuleHeld, "rule_held"),
            (SyncIssueCode::RuleFailed, "rule_failed"),
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), wire);
        }
    }

    /// Every code, listed through an exhaustive match: a new variant does not
    /// compile until it is added here, and the test below then demands it in TS.
    fn all_codes() -> Vec<SyncIssueCode> {
        use SyncIssueCode::*;
        let all = vec![
            CopyFailed,
            NoLocalPath,
            RootMissing,
            RenameCollision,
            RenameReplayFailed,
            LookupFailed,
            MissingCounterpart,
            ManagedFileFailed,
            PeerChannelFailed,
            VersionConflict,
            RuleConflict,
            ReqAutoCloseFailed,
            MigrationFailed,
            MigrationLeftInPlace,
            MigrationDiverged,
            AutocommitFailed,
            RuleUnknownSkill,
            RuleHeld,
            RuleFailed,
        ];
        for code in &all {
            match code {
                CopyFailed | NoLocalPath | RootMissing | RenameCollision | RenameReplayFailed
                | LookupFailed | MissingCounterpart | ManagedFileFailed | PeerChannelFailed
                | VersionConflict | RuleConflict | ReqAutoCloseFailed | MigrationFailed
                | MigrationLeftInPlace | MigrationDiverged | AutocommitFailed
                | RuleUnknownSkill | RuleHeld | RuleFailed => {}
            }
        }
        all
    }

    /// T-000198 review: the TS mirror (`SYNC_ISSUE_CODES` / `SYNC_ISSUE_LOOKUPS`
    /// in `types/sync.ts`) must list every Rust wire name — the i18n table is
    /// checked against those lists at compile time, so together a new code
    /// cannot ship without a translation.
    #[test]
    fn test_ts_mirror_lists_every_code_and_lookup_token() {
        let ts = include_str!("../../../src/lib/types/sync.ts");
        for code in all_codes() {
            let wire = serde_json::to_value(code).unwrap();
            let wire = wire.as_str().unwrap();
            assert!(
                ts.contains(&format!("'{}'", wire)),
                "types/sync.ts SYNC_ISSUE_CODES lacks '{}'",
                wire
            );
        }
        for token in [
            lookup::RENAMES,
            lookup::REPOS,
            lookup::PROJECT,
            lookup::SKILLS,
            lookup::SEEDING,
            lookup::PEERS,
            lookup::CODEX_SOURCE,
            lookup::HOME_DIR,
            lookup::AUTOCOMMIT_REPOS,
        ] {
            assert!(
                ts.contains(&format!("'{}'", token)),
                "types/sync.ts SYNC_ISSUE_LOOKUPS lacks '{}'",
                token
            );
        }
    }
}
