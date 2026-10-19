use crate::db::{rule_event, AppDb};
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
    Ok(after_rule_write(
        db,
        home,
        language_key,
        file_name,
        rule_event::SAVED,
    ))
}

/// The rule a template file stands for, or `None` for an ordinary template.
/// Rules live in `_global` only — see `sync::rule_name_of`.
fn rule_of<'a>(language_key: &str, file_name: &'a str) -> Option<&'a str> {
    match sync::rule_name_of(file_name) {
        Some(rule) if language_key == "_global" => Some(rule),
        _ => None,
    }
}

/// What follows a successful DB write of a template by the owner (a save or a
/// reset), shared so the two can never drift apart:
/// - an ordinary template: nothing;
/// - a rule (T-000174): `kind` goes to its history;
/// - a skill: additionally delivered everywhere at once (below).
///
/// The global CLAUDE.md section is recorded but not rendered — it goes out
/// through its own sync button.
fn after_rule_write(
    db: &AppDb,
    home: Option<PathBuf>,
    language_key: &str,
    file_name: &str,
    kind: &str,
) -> TemplateSaveOutcome {
    let Some(rule) = rule_of(language_key, file_name) else {
        return TemplateSaveOutcome::default();
    };
    // Written before the render, so a hand-back the render then supersedes
    // reads after the edit that superseded it. A failed history write must not
    // turn a stored edit into a failed one: it is reported, nothing more.
    let mut errors = Vec::new();
    if let Err(e) = db.insert_rule_event(rule, kind, None, None) {
        errors.push(
            SyncIssue::new(SyncIssueCode::RuleFailed, rule)
                .target("rule_events")
                .detail(e),
        );
    }
    let Some(skill_name) = sync::skill_name_of(file_name) else {
        return TemplateSaveOutcome {
            errors,
            ..TemplateSaveOutcome::default()
        };
    };

    // T-000203: a rule is only agreed once it reaches the agents. Render both
    // targets now rather than waiting for the separate global-rules button and
    // for a project sync that auto-runs only where auto-sync is opted into.
    //
    // T-000207: through `refresh_rules`, so hand-backs pending anywhere in the
    // portfolio are collected before this render overwrites them. A pending
    // hand-back of THIS skill is superseded: the owner's explicit edit wins.
    let claude_dir = home.map(|h| h.join(".claude"));
    let rules = sync::refresh_rules(
        db,
        claude_dir.as_deref(),
        sync::RenderScope::Everywhere,
        Some(skill_name),
    );
    errors.extend(rules.errors);
    if claude_dir.is_none() {
        errors.push(SyncIssue::lookup(lookup::HOME_DIR, ""));
    }
    TemplateSaveOutcome {
        rendered: true,
        skills: rules.skills,
        repos: rules.repos,
        errors,
        taken: rules.taken,
        lost: rules.lost,
        superseded: rules.superseded,
    }
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

/// Async for the same reason as a save: resetting a skill re-renders the whole
/// portfolio.
#[tauri::command(async)]
pub fn reset_template_file(
    db: State<AppDb>,
    language_key: String,
    file_name: String,
) -> Result<TemplateSaveOutcome, String> {
    reset_template_file_in(&db, dirs::home_dir(), &language_key, &file_name)
}

/// The body of `reset_template_file`, home injected like `save_template_file_in`.
///
/// T-000212: a reset used to write the database only, so every repo copy and
/// `~/.claude/skills/` kept the text the owner had just discarded until some
/// later sync. A skill reset now goes through the same delivery as a save,
/// including superseding a pending hand-back of that skill — the owner decided
/// (2026-10-19) that a reset is an explicit edit like any other; without that
/// the collection in this very pass would take the agent's text up and the
/// reset would silently not happen.
fn reset_template_file_in(
    db: &AppDb,
    home: Option<PathBuf>,
    language_key: &str,
    file_name: &str,
) -> Result<TemplateSaveOutcome, String> {
    let bundled = template_seeder::bundled_file_content(language_key, file_name)
        .ok_or_else(|| format!("No bundled default for {}/{}", language_key, file_name))?;
    db.upsert_template_file(language_key, file_name, &bundled, false)
        .map_err(|e| e.to_string())?;
    Ok(after_rule_write(
        db,
        home,
        language_key,
        file_name,
        rule_event::RESET,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::skills::test_support::hand_back_edit;
    use std::fs;
    use tempfile::TempDir;

    fn seeded() -> (AppDb, TempDir) {
        let db_dir = TempDir::new().unwrap();
        let db = AppDb::new(db_dir.path().join("test.db")).unwrap();
        template_seeder::seed_bundled_templates(&db).unwrap();
        (db, db_dir)
    }

    /// T-000207 review: a conflict (or a failed store) leaves the marker and the
    /// edited copy in place on purpose — so the render in that same pass must
    /// leave the copy alone too, or the next pass reports it "lost" and the
    /// agent's text is gone. Proven across the two renders that hit it hardest:
    /// a Settings save (everywhere) and a project sync (`write_repo_skeletons`).
    #[test]
    fn test_unresolved_handback_copies_survive_a_settings_save_and_a_project_sync() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let one = TempDir::new().unwrap();
        let two = TempDir::new().unwrap();
        let p = db.create_project("P", None, "standard").unwrap();
        for (dir, label) in [(&one, "one"), (&two, "two")] {
            db.insert_local_repository(
                &dir.path().to_string_lossy(),
                label,
                Some(p.id),
                Some("tool"),
            )
            .unwrap();
            sync::render_skills_to_repo(&db, dir.path()).unwrap();
        }
        let text_one = hand_back_edit(one.path(), "sdh-debugging");
        let text_two = hand_back_edit(two.path(), "sdh-debugging");
        let copy = |dir: &TempDir| {
            fs::read_to_string(dir.path().join("docs/sdh_skills/sdh-debugging.md")).unwrap()
        };
        let marker = |dir: &TempDir| dir.path().join("docs/sdh_skills/sdh-debugging.md.approved");

        let outcome = save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
            "OWNER TEXT FOR ANOTHER SKILL",
        )
        .unwrap();

        assert_eq!(
            copy(&one),
            text_one,
            "the Settings render left the copy alone"
        );
        assert_eq!(copy(&two), text_two);
        assert!(marker(&one).exists() && marker(&two).exists());
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e.code == SyncIssueCode::RuleHeld),
            "the skipped copy is reported: {:?}",
            outcome.errors
        );

        let result = crate::sync::project_sync::run_project_sync_in(
            &db,
            p.id,
            Some(home.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(
            copy(&one),
            text_one,
            "the project-sync render left it alone too"
        );
        assert_eq!(copy(&two), text_two);
        assert!(marker(&one).exists() && marker(&two).exists());
        assert!(result.rules_lost.is_empty(), "{:?}", result.rules_lost);
    }

    /// `(kind, repo)` of a rule's history, oldest first.
    fn history(db: &AppDb, rule: &str) -> Vec<(String, Option<String>)> {
        let mut events: Vec<_> = db
            .list_rule_events(rule)
            .unwrap()
            .into_iter()
            .map(|e| (e.kind, e.repo))
            .collect();
        events.reverse();
        events
    }

    /// T-000212: a reset used to touch the database only, so every repo copy
    /// and `~/.claude/skills/` kept the text the owner had just thrown away
    /// until some later sync. It now delivers the bundled text everywhere at
    /// once — and, like a save, sets aside a pending hand-back of the same
    /// skill (owner's decision 2026-10-19): otherwise the collection in that
    /// very pass would take the agent's text up and the reset would silently
    /// not happen.
    #[test]
    fn test_resetting_a_skill_rerenders_every_copy_and_supersedes_its_handback() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();
        db.insert_local_repository(&repo_dir.path().to_string_lossy(), "agentrepo", None, None)
            .unwrap();
        save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
            "OWNER TEXT",
        )
        .unwrap();
        let agent_text = hand_back_edit(repo_dir.path(), "sdh-retro");
        let skills = repo_dir.path().join("docs").join("sdh_skills");
        let bundle =
            template_seeder::bundled_file_content("_global", "skill.sdh-retro.md.tmpl").unwrap();

        let outcome = reset_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
        )
        .unwrap();

        let row = db
            .get_template_file("_global", "skill.sdh-retro.md.tmpl")
            .unwrap()
            .unwrap();
        assert_eq!(row.content, bundle, "the agent's text was not taken up");
        assert_ne!(row.content, agent_text);
        assert!(!row.is_custom);
        assert_eq!(
            fs::read_to_string(skills.join("sdh-retro.md")).unwrap(),
            bundle,
            "the repo copy is the bundled text right away, without a sync"
        );
        assert_eq!(
            fs::read_to_string(home.path().join(".claude/skills/sdh-retro/SKILL.md")).unwrap(),
            bundle
        );
        assert!(!skills.join("sdh-retro.md.approved").exists());
        assert!(outcome.rendered);
        assert!(outcome.taken.is_empty(), "{:?}", outcome.taken);
        let label = repo_dir.path().to_string_lossy().to_string();
        assert_eq!(outcome.superseded, vec![format!("sdh-retro ({})", label)]);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!(
            history(&db, "sdh-retro"),
            vec![
                ("saved".to_string(), None),
                ("reset".to_string(), None),
                ("superseded".to_string(), Some(label)),
            ]
        );
    }

    /// The global CLAUDE.md section is a rule too: its reset is recorded, but
    /// it is delivered by the separate sync button, exactly like its save.
    #[test]
    fn test_resetting_the_global_section_is_recorded_but_not_rendered() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "claude.md.global.tmpl",
            "OWNER SECTION",
        )
        .unwrap();

        let outcome = reset_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "claude.md.global.tmpl",
        )
        .unwrap();

        assert!(!outcome.rendered);
        assert_eq!(
            db.get_template_file("_global", "claude.md.global.tmpl")
                .unwrap()
                .unwrap()
                .content,
            template_seeder::bundled_file_content("_global", "claude.md.global.tmpl").unwrap()
        );
        assert_eq!(
            history(&db, "claude.md.global"),
            vec![("saved".to_string(), None), ("reset".to_string(), None)]
        );
        assert!(!home.path().join(".claude").exists(), "nothing rendered");
    }

    /// An ordinary template is not a rule: no render, no history.
    #[test]
    fn test_saving_or_resetting_an_ordinary_template_renders_nothing_and_records_nothing() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();

        let saved = save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            ".gitignore.tmpl",
            "custom",
        )
        .unwrap();
        let reset = reset_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            ".gitignore.tmpl",
        )
        .unwrap();

        assert!(!saved.rendered && !reset.rendered);
        let conn = db.conn.lock().unwrap();
        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM rule_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 0);
    }

    /// `skill.md.tmpl` has the prefix and the suffix overlapping: it names no
    /// skill. Saving it is an ordinary template save, never a panic.
    #[test]
    fn test_saving_a_file_named_like_a_bare_skill_affix_does_not_panic() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();

        let outcome = save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.md.tmpl",
            "x",
        )
        .unwrap();

        assert!(!outcome.rendered);
    }

    /// T-000207 (R3): a Settings save re-renders the portfolio. Hand-backs of
    /// other skills are taken up first; a hand-back of the very skill the owner
    /// just edited loses to that explicit edit, and says so.
    #[test]
    fn test_settings_save_takes_other_handbacks_and_supersedes_the_same_skill() {
        let (db, _db_dir) = seeded();
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
            vec![format!(
                "sdh-debugging ({})",
                repo_dir.path().to_string_lossy()
            )],
            "the set-aside hand-back is reported, not dropped silently"
        );
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    }
}
