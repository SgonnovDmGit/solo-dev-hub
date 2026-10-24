// F-000044 / T-000220: thin Tauri commands over `sync::shared`. All of them
// spawn git, so all run off the UI thread.

use crate::db::AppDb;
use crate::git_ops::cached_git;
use crate::models::{ParticipantActivity, SharedDisableResult, SharedMergeResult, SharedPreflight};
use crate::sync::shared;
use crate::sync::shared_structure::{JoinRequest, ProjectStructure};
use tauri::State;

fn need_git() -> Result<crate::git_ops::GitBinary, String> {
    cached_git().ok_or_else(|| "git is not available on this machine".to_string())
}

/// What stands between the owner and switching shared mode on for a repo.
#[tauri::command(async)]
pub fn shared_mode_preflight(db: State<AppDb>, repo_id: i64) -> Result<SharedPreflight, String> {
    shared::shared_mode_preflight(&db, &need_git()?, repo_id)
}

/// Switch shared mode on. Errors when the repo has no project / dev branch, the
/// login is unknown, or `.gitignore` lines still ignore shared files and
/// `remove_lines` is false; with `remove_lines` those exact lines are removed.
/// The commit and the push happen on the next sync.
#[tauri::command(async)]
pub fn enable_shared_mode(
    db: State<AppDb>,
    repo_id: i64,
    remove_lines: bool,
) -> Result<(), String> {
    shared::enable_shared_mode(&db, &need_git()?, repo_id, remove_lines)
}

/// Switch shared mode off; the deletion of `.sdh/` is committed and pushed now.
#[tauri::command(async)]
pub fn disable_shared_mode(db: State<AppDb>, repo_id: i64) -> Result<SharedDisableResult, String> {
    shared::disable_shared_mode(&db, &need_git()?, repo_id)
}

/// Owner only: approve the join request of `login`; the next sync commits it.
#[tauri::command(async)]
pub fn approve_join_request(
    db: State<AppDb>,
    repo_id: i64,
    login: String,
) -> Result<ProjectStructure, String> {
    shared::approve_join_request(&db, cached_git().as_ref(), repo_id, &login)
}

/// Contributor: trust the owner's changed dev branch.
#[tauri::command]
pub fn accept_new_main_branch(db: State<AppDb>, repo_id: i64) -> Result<(), String> {
    shared::accept_new_main_branch(&db, repo_id)
}

/// The "merge" button: merge `origin/<current branch>` into the current branch.
#[tauri::command(async)]
pub fn merge_shared_upstream(db: State<AppDb>, repo_id: i64) -> Result<SharedMergeResult, String> {
    shared::merge_shared_upstream(&db, &need_git()?, repo_id)
}

#[tauri::command(async)]
pub fn list_join_requests(db: State<AppDb>, repo_id: i64) -> Result<Vec<JoinRequest>, String> {
    shared::list_join_requests(&db, cached_git().as_ref(), repo_id)
}

#[tauri::command(async)]
pub fn get_repo_structure(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Option<ProjectStructure>, String> {
    shared::get_repo_structure(&db, cached_git().as_ref(), repo_id)
}

/// Owner's view: when each participant's install last pushed a sync commit.
#[tauri::command(async)]
pub fn shared_participants_activity(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<ParticipantActivity>, String> {
    shared::participants_activity(&db, &need_git()?, repo_id)
}
