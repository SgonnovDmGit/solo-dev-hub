use crate::db::AppDb;
use crate::models::*;
use crate::sync;
use crate::template_seeder;
use tauri::State;

// ── Templates (0.6.0) ─────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_template_languages(db: State<AppDb>) -> Result<Vec<TemplateLanguage>, String> {
    let keys = db.list_template_languages().map_err(|e| e.to_string())?;
    let mut result: Vec<TemplateLanguage> = Vec::with_capacity(keys.len());
    for key in keys {
        let files = db.list_template_files(&key).map_err(|e| e.to_string())?;
        // Parse display_name from meta.json if present, fallback to language_key.
        let display_name = files
            .iter()
            .find(|f| f.file_name == "meta.json")
            .and_then(|f| serde_json::from_str::<serde_json::Value>(&f.content).ok())
            .and_then(|v| {
                v.get("display_name")
                    .and_then(|s| s.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| key.clone());
        result.push(TemplateLanguage {
            language_key: key,
            display_name,
            file_count: files.len() as i64,
        });
    }
    Ok(result)
}

#[tauri::command]
pub fn list_template_files(
    db: State<AppDb>,
    language_key: String,
) -> Result<Vec<TemplateFile>, String> {
    db.list_template_files(&language_key)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_template_file(
    db: State<AppDb>,
    language_key: String,
    file_name: String,
) -> Result<Option<TemplateFile>, String> {
    db.get_template_file(&language_key, &file_name)
        .map_err(|e| e.to_string())
}

/// A `_global` file named `skill.<name>.md.tmpl` is a workflow rule, not an
/// ordinary template — see `sync::skills`.
const SKILL_PREFIX: &str = "skill.";
const SKILL_SUFFIX: &str = ".md.tmpl";

/// Async because a rule save now walks the whole portfolio: a synchronous
/// command would do that filesystem work on the UI thread and freeze the window.
#[tauri::command(async)]
pub fn save_template_file(
    db: State<AppDb>,
    language_key: String,
    file_name: String,
    content: String,
) -> Result<TemplateSaveOutcome, String> {
    // The DB write goes first and is the only fatal failure here: a rule the
    // owner has just agreed on must never be lost because a render target
    // happened to be unwritable.
    db.upsert_template_file(&language_key, &file_name, &content, true)
        .map_err(|e| e.to_string())?;

    let is_skill = language_key == "_global"
        && file_name.starts_with(SKILL_PREFIX)
        && file_name.ends_with(SKILL_SUFFIX);
    if !is_skill {
        return Ok(TemplateSaveOutcome::default());
    }

    // T-000203: a rule is only agreed once it reaches the agents. Render both
    // targets now rather than waiting for the separate global-rules button and
    // for a project sync that auto-runs only where auto-sync is opted into.
    let Some(home) = dirs::home_dir() else {
        return Ok(TemplateSaveOutcome {
            rendered: false,
            errors: vec!["Cannot determine home directory".to_string()],
            ..Default::default()
        });
    };
    let (skills, repos, errors) = sync::render_skills_everywhere(&db, &home.join(".claude"));
    Ok(TemplateSaveOutcome {
        rendered: true,
        skills,
        repos,
        errors,
    })
}

#[tauri::command]
pub fn reset_template_file(
    db: State<AppDb>,
    language_key: String,
    file_name: String,
) -> Result<(), String> {
    let bundled = template_seeder::bundled_file_content(&language_key, &file_name)
        .ok_or_else(|| format!("No bundled default for {}/{}", language_key, file_name))?;
    db.upsert_template_file(&language_key, &file_name, &bundled, false)
        .map_err(|e| e.to_string())
}
