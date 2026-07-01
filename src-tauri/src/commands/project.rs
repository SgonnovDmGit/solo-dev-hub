use crate::db::AppDb;
use crate::models::*;
use tauri::State;

// ── Project commands ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn create_project(
    db: State<AppDb>,
    name: String,
    description: Option<String>,
    project_type: String,
) -> Result<Project, String> {
    if project_type != "standard" && project_type != "microservice" {
        return Err(format!("Invalid project_type: {}", project_type));
    }
    db.create_project(&name, description.as_deref(), &project_type)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_projects(db: State<AppDb>) -> Result<Vec<Project>, String> {
    db.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_project(
    db: State<AppDb>,
    id: i64,
    name: String,
    description: Option<String>,
) -> Result<Project, String> {
    db.update_project(id, &name, description.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_project(db: State<AppDb>, id: i64) -> Result<(), String> {
    db.delete_project(id).map_err(|e| e.to_string())
}

// T-000136: toggle the per-project auto-sync opt-in flag from the UI.
#[tauri::command]
pub fn set_project_auto_sync(
    db: State<AppDb>,
    project_id: i64,
    enabled: bool,
) -> Result<(), String> {
    db.set_project_auto_sync(project_id, enabled)
        .map_err(|e| e.to_string())
}

// ── Microservice connection commands (F-012) ─────────────────────────────────

#[tauri::command]
pub fn connect_microservice(
    db: State<AppDb>,
    project_id: i64,
    microservice_project_id: i64,
) -> Result<(), String> {
    db.connect_microservice(project_id, microservice_project_id)
}

#[tauri::command]
pub fn disconnect_microservice(
    db: State<AppDb>,
    project_id: i64,
    microservice_project_id: i64,
) -> Result<(), String> {
    db.disconnect_microservice(project_id, microservice_project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_project_microservices(db: State<AppDb>, project_id: i64) -> Result<Vec<i64>, String> {
    db.list_project_microservices(project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_microservice_projects(db: State<AppDb>) -> Result<Vec<Project>, String> {
    db.list_microservice_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_parents_of_microservice(
    db: State<AppDb>,
    ms_project_id: i64,
) -> Result<Vec<Project>, String> {
    db.list_parents_of_microservice(ms_project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_project_type(db: State<AppDb>, id: i64, new_type: String) -> Result<Project, String> {
    db.update_project_type(id, &new_type)
}

#[tauri::command]
pub fn server_repo_of_microservice(
    db: State<AppDb>,
    ms_project_id: i64,
) -> Result<Repository, String> {
    db.server_repo_of_microservice(ms_project_id)
}
