use crate::db::AppDb;
use crate::models::*;
use crate::{export, keyring_store, sync};
use chrono;
use tauri::State;

// ── PAT / Keyring commands ────────────────────────────────────────────────────

#[tauri::command]
pub fn store_pat(token: String) -> Result<(), String> {
    keyring_store::store_pat(&token)
}

#[tauri::command]
pub fn get_pat() -> Result<Option<String>, String> {
    keyring_store::get_pat()
}

#[tauri::command]
pub fn delete_pat() -> Result<(), String> {
    keyring_store::delete_pat()
}

// ── Settings commands ─────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_setting(db: State<AppDb>, key: String) -> Result<Option<String>, String> {
    db.get_setting(&key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_setting(db: State<AppDb>, key: String, value: String) -> Result<(), String> {
    db.set_setting(&key, &value).map_err(|e| e.to_string())
}

// ── F-021 Docs viewer commands ───────────────────────────────────────────────

#[tauri::command]
pub fn read_repo_todo(db: State<AppDb>, repo_id: i64) -> Result<ReadTodoResult, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(ReadTodoResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    };
    let full = std::path::Path::new(&local_path).join("docs/todo.md");
    if !full.exists() {
        return Ok(ReadTodoResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    }
    let content = std::fs::read_to_string(&full).map_err(|e| e.to_string())?;
    let (tasks, warnings) = export::parse_todo_tasks(&content);
    Ok(ReadTodoResult { tasks, warnings })
}

#[tauri::command]
pub fn read_repo_done(db: State<AppDb>, repo_id: i64) -> Result<ReadDoneResult, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(ReadDoneResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    };
    let full = std::path::Path::new(&local_path).join("docs/done.md");
    if !full.exists() {
        return Ok(ReadDoneResult {
            tasks: Vec::new(),
            warnings: Vec::new(),
        });
    }
    let content = std::fs::read_to_string(&full).map_err(|e| e.to_string())?;
    let (tasks, warnings) = export::parse_done_tasks(&content);
    Ok(ReadDoneResult { tasks, warnings })
}

/// Debug/testing command: parse done.md for a single repo in a period.
/// Mainly for diagnostics; Dashboard aggregates this internally via read_dashboard.
#[tauri::command]
pub fn parse_done_entries_in_period_cmd(
    db: State<AppDb>,
    repo_id: i64,
    start: String,
    end: String,
) -> Result<Vec<(String, i64)>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    match repo.local_path {
        None => Ok(vec![]),
        Some(lp) => {
            let path = std::path::PathBuf::from(lp).join("docs").join("done.md");
            crate::export::parse_done_entries_in_period(&path, &start, &end)
                .map_err(|e| e.to_string())
        }
    }
}

#[tauri::command]
pub fn read_repo_files(
    db: State<AppDb>,
    repo_id: i64,
    rel_paths: Vec<String>,
) -> Result<Vec<Option<String>>, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(local_path) = repo.local_path else {
        return Ok(rel_paths.iter().map(|_| None).collect());
    };
    let root = std::path::Path::new(&local_path);
    let mut result: Vec<Option<String>> = Vec::with_capacity(rel_paths.len());
    for rel in &rel_paths {
        // Reject `..`-escapes and absolute paths — symmetric with the
        // write_deploy_files guard so the read side respects the same
        // repo-root boundary the write side enforces.
        if !sync::is_safe_subpath(rel) {
            result.push(None);
            continue;
        }
        let p = root.join(rel);
        if p.exists() {
            match std::fs::read_to_string(&p) {
                Ok(s) => result.push(Some(s)),
                Err(_) => result.push(None),
            }
        } else {
            result.push(None);
        }
    }
    Ok(result)
}

#[tauri::command]
pub fn write_deploy_files(
    db: State<AppDb>,
    deploy_env_id: i64,
    repo_id: i64,
    local_path: String,
    files: Vec<RenderedFile>,
) -> Result<WriteResult, String> {
    let root = std::path::Path::new(&local_path);
    // Guard: never silently recreate a deleted/moved repo folder (B-001 invariant).
    sync::ensure_root_exists(root)?;

    let mut written: Vec<String> = Vec::new();
    let mut errors: Vec<WriteError> = Vec::new();
    for f in &files {
        // Path traversal guard: reject `..`, absolute paths, drive letters.
        // `f.path` ultimately comes from meta.json `file_targets`, which a user
        // can edit via TemplatesScreen; without this check a malicious template
        // could write outside the repo root.
        if !sync::is_safe_subpath(&f.path) {
            errors.push(WriteError {
                path: f.path.clone(),
                error: format!("unsafe path rejected: {}", f.path),
            });
            continue;
        }
        let target = root.join(&f.path);
        if let Some(parent) = target.parent() {
            if parent != root {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(WriteError {
                        path: f.path.clone(),
                        error: e.to_string(),
                    });
                    continue;
                }
            }
        }
        match std::fs::write(&target, &f.content) {
            Ok(_) => written.push(f.path.clone()),
            Err(e) => errors.push(WriteError {
                path: f.path.clone(),
                error: e.to_string(),
            }),
        }
    }

    // v0.20.0: record deploy event after successful file write.
    // H8 review-fix: report `written.len()` (actually written) rather than
    // `files.len()` (input total). Path-rejects + fs::write failures used
    // to be silently counted into the metric.
    let _ = db.insert_deploy_event(
        Some(deploy_env_id),
        repo_id,
        "render",
        &chrono::Utc::now().to_rfc3339(),
        Some(&serde_json::json!({ "file_count": written.len() }).to_string()),
    );

    Ok(WriteResult { written, errors })
}
