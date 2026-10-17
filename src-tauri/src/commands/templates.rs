use crate::db::AppDb;
use crate::models::*;
use crate::sync;
use crate::template_seeder;
use std::path::PathBuf;
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
    save_template_file_in(&db, dirs::home_dir(), &language_key, &file_name, &content)
}

/// The body of `save_template_file`, with the home directory injected so a test
/// can run it against a sandbox instead of the real `~/.claude/`.
fn save_template_file_in(
    db: &AppDb,
    home: Option<PathBuf>,
    language_key: &str,
    file_name: &str,
    content: &str,
) -> Result<TemplateSaveOutcome, String> {
    // The DB write goes first and is the only fatal failure here: a rule the
    // owner has just agreed on must never be lost because a render target
    // happened to be unwritable.
    db.upsert_template_file(language_key, file_name, content, true)
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
    //
    // T-000207: through `refresh_rules`, so hand-backs pending anywhere in the
    // portfolio are collected before this render overwrites them. A pending
    // hand-back of THIS skill is superseded: the owner's explicit edit wins.
    let skill_name = &file_name[SKILL_PREFIX.len()..file_name.len() - SKILL_SUFFIX.len()];
    let claude_dir = home.map(|h| h.join(".claude"));
    let rules = sync::refresh_rules(
        db,
        claude_dir.as_deref(),
        sync::RenderScope::Everywhere,
        Some(skill_name),
    );
    let mut errors = rules.errors;
    if claude_dir.is_none() {
        errors.push("Cannot determine home directory".to_string());
    }
    Ok(TemplateSaveOutcome {
        rendered: true,
        skills: rules.skills,
        repos: rules.repos,
        errors,
        taken: rules.taken,
        lost: rules.lost,
        superseded: rules.superseded,
    })
}

/// T-000207: what the startup rule delivery found, held until the frontend has
/// loaded and can show it. `None` once taken, or when the launch rendered nothing.
#[derive(Default)]
pub struct StartupRulesReport(pub std::sync::Mutex<Option<RulesRefresh>>);

/// Returns the startup rule report once, then clears it, so a reload of the
/// window does not repeat the same toast.
#[tauri::command]
pub fn take_startup_rules_report(state: State<StartupRulesReport>) -> Option<RulesRefresh> {
    state.0.lock().ok().and_then(|mut slot| slot.take())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn hand_back_edit(repo: &Path, name: &str) -> String {
        let skills = repo.join("docs").join("sdh_skills");
        let copy = skills.join(format!("{}.md", name));
        let edited = format!("{}\n\nHANDED BACK\n", fs::read_to_string(&copy).unwrap());
        fs::write(&copy, &edited).unwrap();
        fs::write(skills.join(format!("{}.md.approved", name)), "").unwrap();
        edited
    }

    /// T-000207 (R3): a Settings save re-renders the portfolio. Hand-backs of
    /// other skills are taken up first; a hand-back of the very skill the owner
    /// just edited loses to that explicit edit, and says so.
    #[test]
    fn test_settings_save_takes_other_handbacks_and_supersedes_the_same_skill() {
        let db_dir = TempDir::new().unwrap();
        let db = AppDb::new(db_dir.path().join("test.db")).unwrap();
        template_seeder::seed_bundled_templates(&db).unwrap();
        let home = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();
        db.insert_local_repository(&repo_dir.path().to_string_lossy(), "agentrepo", None, None)
            .unwrap();
        sync::render_skills_to_repo(&db, repo_dir.path()).unwrap();
        let skills = repo_dir.path().join("docs").join("sdh_skills");
        let other = hand_back_edit(repo_dir.path(), "sdh-retro");
        hand_back_edit(repo_dir.path(), "sdh-debugging");

        let outcome = save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-debugging.md.tmpl",
            "OWNER TEXT FROM SETTINGS",
        )
        .unwrap();

        let row = |name: &str| {
            db.get_template_file("_global", &format!("skill.{}.md.tmpl", name))
                .unwrap()
                .unwrap()
                .content
        };
        // The other skill's hand-back was taken up, not wiped.
        assert_eq!(row("sdh-retro"), other);
        assert_eq!(
            fs::read_to_string(skills.join("sdh-retro.md")).unwrap(),
            other
        );
        assert!(!skills.join("sdh-retro.md.approved").exists());
        // The same skill: the owner's Settings edit wins everywhere.
        assert_eq!(row("sdh-debugging"), "OWNER TEXT FROM SETTINGS");
        assert_eq!(
            fs::read_to_string(skills.join("sdh-debugging.md")).unwrap(),
            "OWNER TEXT FROM SETTINGS"
        );
        assert!(!skills.join("sdh-debugging.md.approved").exists());
        assert!(outcome.rendered);
        assert_eq!(outcome.taken, vec!["sdh-retro".to_string()]);
        assert_eq!(
            outcome.superseded,
            vec!["sdh-debugging (agentrepo)".to_string()],
            "the set-aside hand-back is reported, not dropped silently"
        );
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    }
}
