use crate::db::AppDb;
use chrono;
use tauri::State;

// ── v0.20.0: Event recording commands (called from TS after GitHub API calls) ─

#[tauri::command]
pub fn record_deploy_secret_event(
    db: State<AppDb>,
    deploy_env_id: i64,
    repo_id: i64,
    action: String,
    secret_name: String,
) -> Result<(), String> {
    let details = serde_json::json!({ "name": secret_name }).to_string();
    db.insert_deploy_event(
        Some(deploy_env_id),
        repo_id,
        action.as_str(),
        &chrono::Utc::now().to_rfc3339(),
        Some(&details),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn record_secret_event(
    db: State<AppDb>,
    repo_id: i64,
    action: String,
    secret_name: String,
) -> Result<(), String> {
    let details = serde_json::json!({ "action": action, "name": secret_name }).to_string();
    db.insert_sync_event(
        Some(repo_id),
        "secret",
        &chrono::Utc::now().to_rfc3339(),
        1,
        Some(&details),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn read_timeline(
    db: State<AppDb>,
    filter: crate::models::TimelineFilter,
    offset: u32,
    limit: u32,
) -> Result<Vec<crate::models::ActivityEvent>, String> {
    db.read_timeline_filtered(&filter, offset, limit)
        .map_err(|e| e.to_string())
}
