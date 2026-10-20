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
    // T-000174: a rule save is read-old-text, write, record, deliver — one
    // step as far as every other pass is concerned. Held across all of it, so
    // an auto-sync cannot slip between the write and the event and leave the
    // origin history disagreeing with the stored text. An ordinary template
    // needs no lock. `after_rule_write` runs the unlocked delivery
    // (`refresh_rules_locked`); the lock is not re-entrant.
    let _guard = rule_of(language_key, file_name).map(|_| sync::rules_guard());
    // T-000224: a rule that follows an owner's canon is changed in his repo.
    refuse_followed(db, language_key, file_name)?;
    // T-000206: a save of the very text already stored changes nothing, so it
    // is no event of the rule's history. Compared before the write, which is
    // what replaces the old text. An unreadable old row counts as a change:
    // recording one save too many beats losing one.
    //
    // An unchanged save still renders through `after_rule_write` and so still
    // supersedes pending hand-backs of this skill. Intended: pressing Save is
    // the owner's explicit edit, and the explicit edit wins — the history
    // just has nothing new to say about the text.
    let unchanged = matches!(
        db.get_template_file(language_key, file_name),
        Ok(Some(old)) if old.content == content
    );
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
        (!unchanged).then_some(rule_event::SAVED),
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

/// T-000224: a rule this install takes from an owner (`sync::shared_canon`) is
/// read-only here; the error is `rule_follows_main: <login>`.
fn refuse_followed(db: &AppDb, language_key: &str, file_name: &str) -> Result<(), String> {
    match rule_of(language_key, file_name) {
        Some(rule) => sync::shared_canon::refuse_if_followed(db, rule),
        None => Ok(()),
    }
}

/// What follows a successful DB write of a template by the owner (a save or a
/// reset), shared so the two can never drift apart:
/// - an ordinary template: nothing;
/// - a rule (T-000174): `kind` goes to its history — `None` for a save that
///   changed nothing (T-000206); a reset always records;
/// - a skill: additionally delivered everywhere at once (below).
///
/// The global CLAUDE.md section is recorded but not rendered — it goes out
/// through its own sync button.
///
/// **The caller holds `sync::rules_guard()`** for a rule (T-000174): the write,
/// the event and the delivery are one hold, and the delivery therefore calls
/// `refresh_rules_locked`.
fn after_rule_write(
    db: &AppDb,
    home: Option<PathBuf>,
    language_key: &str,
    file_name: &str,
    kind: Option<&str>,
) -> TemplateSaveOutcome {
    let Some(rule) = rule_of(language_key, file_name) else {
        return TemplateSaveOutcome::default();
    };
    // Written before the render, so a hand-back the render then supersedes
    // reads after the edit that superseded it. A failed history write must not
    // turn a stored edit into a failed one: it is reported, nothing more.
    let mut errors = Vec::new();
    if let Err(e) = kind.map_or(Ok(()), |kind| db.insert_rule_event(rule, kind, None, None)) {
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
    // T-000207: through `refresh_rules_locked`, so hand-backs pending anywhere in the
    // portfolio are collected before this render overwrites them. A pending
    // hand-back of THIS skill is superseded: the owner's explicit edit wins.
    let claude_dir = home.map(|h| h.join(".claude"));
    let rules = sync::refresh_rules_locked(
        db,
        claude_dir.as_deref(),
        sync::RenderScope::Everywhere,
        Some(skill_name),
    );
    rendered_outcome(rules, errors, claude_dir.is_none())
}

/// The outcome of a delivery to the whole portfolio: `errors` first, then the
/// pass's own, plus the missing home directory when `~/.claude` was not
/// reachable. Shared by a rule write and a conflict resolution.
fn rendered_outcome(
    rules: RulesRefresh,
    mut errors: Vec<SyncIssue>,
    home_missing: bool,
) -> TemplateSaveOutcome {
    errors.extend(rules.errors);
    if home_missing {
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

// ── Rules screen (T-000174, T-000206) ───────────────────────────────────────

/// Every workflow rule with its status. Async: reads the filesystem to count
/// the repos a rule reaches.
#[tauri::command(async)]
pub fn list_rules(db: State<AppDb>) -> Result<RulesOverview, String> {
    list_rules_in(&db)
}

/// The global CLAUDE.md section first, then the skills alphabetically.
fn list_rules_in(db: &AppDb) -> Result<RulesOverview, String> {
    let files = db
        .list_template_files("_global")
        .map_err(|e| e.to_string())?;
    let conflicts = db.open_rule_conflicts().map_err(|e| e.to_string())?;
    let followed = sync::shared_canon::followed_rules(db);
    let mut rules = Vec::new();
    for file in files {
        let Some(rule) = sync::rule_name_of(&file.file_name).map(str::to_string) else {
            continue;
        };
        let canon = template_seeder::bundled_file_content("_global", &file.file_name);
        rules.push(RuleInfo {
            // Text against the bundle, not `is_custom`: a save of text equal to
            // the bundle leaves the flag set but changes nothing.
            changed: canon.as_deref().is_some_and(|canon| canon != file.content),
            conflict: conflicts.contains(&rule),
            follows_main: followed.get(&rule).map(|f| f.login.clone()),
            origin: db.latest_origin_event(&rule).map_err(|e| e.to_string())?,
            canon,
            file_name: file.file_name,
            content: file.content,
            rule,
        });
    }
    rules.sort_by(|a, b| {
        (a.rule != sync::GLOBAL_SECTION_RULE, &a.rule)
            .cmp(&(b.rule != sync::GLOBAL_SECTION_RULE, &b.rule))
    });
    Ok(RulesOverview {
        rules,
        repos: sync::count_render_targets(db)?,
    })
}

/// A rule's history, newest first.
#[tauri::command]
pub fn list_rule_events(db: State<AppDb>, rule: String) -> Result<Vec<RuleEvent>, String> {
    db.list_rule_events(&rule).map_err(|e| e.to_string())
}

/// The sides of a rule's hand-back conflict, read live from the repos.
#[tauri::command(async)]
pub fn get_rule_conflict(db: State<AppDb>, rule: String) -> Result<Vec<RuleConflictSide>, String> {
    sync::conflict_sides(&db, &rule)
}

/// The owner decides a conflict: `accept` = the label of the side to take,
/// `None` = reject both. Errors `rule_conflict_gone` / `rule_conflict_side_gone`
/// are matched by the frontend. Async: delivers to the whole portfolio.
#[tauri::command(async)]
pub fn resolve_rule_conflict(
    db: State<AppDb>,
    rule: String,
    accept: Option<String>,
) -> Result<TemplateSaveOutcome, String> {
    resolve_rule_conflict_in(&db, dirs::home_dir(), &rule, accept.as_deref())
}

/// The body of `resolve_rule_conflict`, home injected like the save's.
fn resolve_rule_conflict_in(
    db: &AppDb,
    home: Option<PathBuf>,
    rule: &str,
    accept: Option<&str>,
) -> Result<TemplateSaveOutcome, String> {
    let claude_dir = home.map(|h| h.join(".claude"));
    let rules = sync::resolve_conflict(db, claude_dir.as_deref(), rule, accept)?;
    Ok(rendered_outcome(rules, Vec::new(), claude_dir.is_none()))
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
    // T-000174: same hold as a save — see `save_template_file_in`.
    let _guard = rule_of(language_key, file_name).map(|_| sync::rules_guard());
    refuse_followed(db, language_key, file_name)?;
    db.upsert_template_file(language_key, file_name, &bundled, false)
        .map_err(|e| e.to_string())?;
    Ok(after_rule_write(
        db,
        home,
        language_key,
        file_name,
        Some(rule_event::RESET),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::conflict_note;
    use crate::db::rules::rule_history;
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
        rule_history(db, rule)
            .into_iter()
            .map(|e| (e.kind, e.repo))
            .collect()
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

    /// T-000206: the owner saving a skill whose hand-backs are in conflict is a
    /// decision too — both markers are set aside, so the conflict is closed and
    /// says why, and the agents' reason files go with it.
    #[test]
    fn test_saving_a_skill_in_conflict_closes_the_conflict() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let dirs = [TempDir::new().unwrap(), TempDir::new().unwrap()];
        for (i, dir) in dirs.iter().enumerate() {
            db.insert_local_repository(
                &dir.path().to_string_lossy(),
                &format!("r{}", i),
                None,
                None,
            )
            .unwrap();
            sync::render_skills_to_repo(&db, dir.path()).unwrap();
            hand_back_edit(dir.path(), "sdh-debugging");
        }
        let skills = |dir: &TempDir| dir.path().join("docs/sdh_skills");
        // Any pass opens the conflict — here the save of another skill.
        save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
            "OTHER",
        )
        .unwrap();
        assert!(db.open_rule_conflicts().unwrap().contains("sdh-debugging"));
        assert!(skills(&dirs[0]).join("sdh-debugging.md.conflict").exists());

        let outcome = save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-debugging.md.tmpl",
            "OWNER DECIDES",
        )
        .unwrap();

        assert!(db.open_rule_conflicts().unwrap().is_empty());
        for dir in &dirs {
            assert!(!skills(dir).join("sdh-debugging.md.approved").exists());
            assert!(!skills(dir).join("sdh-debugging.md.conflict").exists());
            assert_eq!(
                fs::read_to_string(skills(dir).join("sdh-debugging.md")).unwrap(),
                "OWNER DECIDES"
            );
        }
        let resolved: Vec<_> = rule_history(&db, "sdh-debugging")
            .into_iter()
            .filter(|e| e.kind == rule_event::CONFLICT_RESOLVED)
            .collect();
        assert_eq!(resolved.len(), 1, "{:?}", resolved);
        assert_eq!(resolved[0].repo, None);
        assert_eq!(
            resolved[0].note.as_deref(),
            Some(conflict_note::SUPERSEDED_BY_OWNER)
        );
        assert_eq!(outcome.superseded.len(), 2, "{:?}", outcome.superseded);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    }

    /// T-000206: a reset is an explicit edit like a save — it sets both
    /// markers of a conflict aside and closes the conflict, saying why.
    #[test]
    fn test_resetting_a_skill_in_conflict_closes_the_conflict() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let dirs = [TempDir::new().unwrap(), TempDir::new().unwrap()];
        for (i, dir) in dirs.iter().enumerate() {
            db.insert_local_repository(
                &dir.path().to_string_lossy(),
                &format!("r{}", i),
                None,
                None,
            )
            .unwrap();
            sync::render_skills_to_repo(&db, dir.path()).unwrap();
            hand_back_edit(dir.path(), "sdh-debugging");
        }
        sync::refresh_rules(&db, None, sync::RenderScope::UptakeOnly, None);
        assert!(db.open_rule_conflicts().unwrap().contains("sdh-debugging"));

        reset_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-debugging.md.tmpl",
        )
        .unwrap();

        assert!(db.open_rule_conflicts().unwrap().is_empty());
        let resolved: Vec<_> = rule_history(&db, "sdh-debugging")
            .into_iter()
            .filter(|e| e.kind == rule_event::CONFLICT_RESOLVED)
            .collect();
        assert_eq!(resolved.len(), 1, "{:?}", resolved);
        assert_eq!(
            resolved[0].note.as_deref(),
            Some(conflict_note::SUPERSEDED_BY_OWNER)
        );
        for dir in &dirs {
            let skills = dir.path().join("docs/sdh_skills");
            assert!(!skills.join("sdh-debugging.md.approved").exists());
            assert!(!skills.join("sdh-debugging.md.conflict").exists());
        }
    }

    /// T-000174: the DB write and the history event of a rule save/reset run
    /// under the rules lock, like the collection they precede. Without it an
    /// auto-sync could interleave between the write and the event, and the
    /// recorded origin would disagree with the stored text. Held from outside
    /// here: the operation must not touch the rule until the lock is free.
    #[test]
    fn test_saving_or_resetting_a_rule_waits_for_the_rules_lock() {
        use std::time::Duration;
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let stored = || {
            db.get_template_file("_global", "skill.sdh-retro.md.tmpl")
                .unwrap()
                .unwrap()
                .content
        };
        let bundle = stored();
        let events = || db.list_rule_events("sdh-retro").unwrap().len();

        // Verdicts are taken while the lock is held and asserted only after it
        // is released and the thread joined: an assert panicking inside the
        // scope would leave the worker blocked on the lock and the scope
        // joining it forever — a hang instead of a failure.
        let guard = sync::rules_guard();
        let (during_stored, during_events) = std::thread::scope(|s| {
            let save = s.spawn(|| {
                save_template_file_in(
                    &db,
                    Some(home.path().to_path_buf()),
                    "_global",
                    "skill.sdh-retro.md.tmpl",
                    "OWNER TEXT",
                )
                .unwrap()
            });
            std::thread::sleep(Duration::from_millis(300));
            let seen = (stored(), events());
            drop(guard);
            save.join().unwrap();
            seen
        });
        assert!(
            during_stored == bundle,
            "the save wrote before it had the lock"
        );
        assert_eq!(during_events, 0);
        assert_eq!(stored(), "OWNER TEXT");
        assert_eq!(events(), 1);

        let guard = sync::rules_guard();
        let (during_stored, during_events) = std::thread::scope(|s| {
            let reset = s.spawn(|| {
                reset_template_file_in(
                    &db,
                    Some(home.path().to_path_buf()),
                    "_global",
                    "skill.sdh-retro.md.tmpl",
                )
                .unwrap()
            });
            std::thread::sleep(Duration::from_millis(300));
            let seen = (stored(), events());
            drop(guard);
            reset.join().unwrap();
            seen
        });
        assert_eq!(
            during_stored, "OWNER TEXT",
            "the reset wrote before the lock"
        );
        assert_eq!(during_events, 1);
        assert!(stored() == bundle);
        assert_eq!(events(), 2);
    }

    /// An ordinary template is no rule: it never waits for the rules lock.
    /// The worker signals when the save returns; the verdict is read with a
    /// timeout and asserted only after the lock is released, so a regression
    /// fails instead of hanging.
    #[test]
    fn test_saving_an_ordinary_template_does_not_wait_for_the_rules_lock() {
        use std::time::Duration;
        let (db, _db_dir) = seeded();
        let (done, saved) = std::sync::mpsc::channel();
        let guard = sync::rules_guard();
        let returned_in_time = std::thread::scope(|s| {
            let worker = s.spawn(|| {
                let outcome =
                    save_template_file_in(&db, None, "_global", ".gitignore.tmpl", "custom");
                let _ = done.send(());
                outcome.unwrap()
            });
            let returned = saved.recv_timeout(Duration::from_secs(2)).is_ok();
            drop(guard);
            worker.join().unwrap();
            returned
        });
        assert!(
            returned_in_time,
            "an ordinary save blocked on the rules lock"
        );
    }

    /// Saving the text that is already stored changes nothing, so it is not an
    /// event of the rule's history either.
    #[test]
    fn test_a_save_that_changes_nothing_records_no_event() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let save = |text: &str| {
            save_template_file_in(
                &db,
                Some(home.path().to_path_buf()),
                "_global",
                "skill.sdh-retro.md.tmpl",
                text,
            )
            .unwrap()
        };

        save("OWNER TEXT");
        save("OWNER TEXT");

        assert_eq!(history(&db, "sdh-retro"), vec![("saved".to_string(), None)]);
    }

    /// The history is a record, never a precondition: with `rule_events` broken
    /// a save and a reset still store the text, and say what failed.
    #[test]
    fn test_a_failed_history_write_does_not_block_a_save_or_a_reset() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE rule_events")
            .unwrap();
        let stored = || {
            db.get_template_file("_global", "skill.sdh-retro.md.tmpl")
                .unwrap()
                .unwrap()
                .content
        };
        let failed = |o: &TemplateSaveOutcome| {
            o.errors
                .iter()
                .any(|e| e.code == SyncIssueCode::RuleFailed && e.subject == "sdh-retro")
        };

        let saved = save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
            "OWNER TEXT",
        )
        .unwrap();
        assert_eq!(stored(), "OWNER TEXT");
        assert!(failed(&saved), "{:?}", saved.errors);

        let reset = reset_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
        )
        .unwrap();
        assert_eq!(
            stored(),
            template_seeder::bundled_file_content("_global", "skill.sdh-retro.md.tmpl").unwrap()
        );
        assert!(failed(&reset), "{:?}", reset.errors);
    }

    // ── Rules screen: listing and deciding a conflict ────────────────────────

    const SKILL: &str = "sdh-debugging";

    fn skills_dir(dir: &TempDir) -> PathBuf {
        dir.path().join("docs/sdh_skills")
    }

    fn rule_row(db: &AppDb) -> TemplateFile {
        db.get_template_file("_global", "skill.sdh-debugging.md.tmpl")
            .unwrap()
            .unwrap()
    }

    /// Two local repos, each handing back its own text of `SKILL`, and a pass
    /// that opens the conflict. Returns the checkouts, their labels (a
    /// local-only repo is labelled by its path) and the texts they handed back.
    fn open_conflict(db: &AppDb) -> (Vec<TempDir>, Vec<String>, Vec<String>) {
        let dirs = vec![TempDir::new().unwrap(), TempDir::new().unwrap()];
        let mut labels = Vec::new();
        let mut texts = Vec::new();
        for (i, dir) in dirs.iter().enumerate() {
            let path = dir.path().to_string_lossy().to_string();
            db.insert_local_repository(&path, &format!("r{}", i), None, None)
                .unwrap();
            sync::render_skills_to_repo(db, dir.path()).unwrap();
            texts.push(hand_back_edit(dir.path(), SKILL));
            labels.push(path);
        }
        sync::refresh_rules(db, None, sync::RenderScope::UptakeOnly, None);
        assert!(db.open_rule_conflicts().unwrap().contains(SKILL));
        (dirs, labels, texts)
    }

    fn resolutions(db: &AppDb) -> Vec<RuleEvent> {
        rule_history(db, SKILL)
            .into_iter()
            .filter(|e| e.kind == rule_event::CONFLICT_RESOLVED)
            .collect()
    }

    /// Accepting a side makes its text the rule, everywhere, and the history
    /// names the side as the text's origin.
    #[test]
    fn test_accepting_a_conflict_side_makes_its_text_the_rule_everywhere() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let (dirs, labels, texts) = open_conflict(&db);

        let outcome = resolve_rule_conflict_in(
            &db,
            Some(home.path().to_path_buf()),
            SKILL,
            Some(&labels[1]),
        )
        .unwrap();

        let row = rule_row(&db);
        assert_eq!(row.content, texts[1]);
        assert!(row.is_custom);
        for dir in &dirs {
            assert!(!skills_dir(dir).join("sdh-debugging.md.approved").exists());
            assert!(!skills_dir(dir).join("sdh-debugging.md.conflict").exists());
            assert_eq!(
                fs::read_to_string(skills_dir(dir).join("sdh-debugging.md")).unwrap(),
                texts[1],
                "every copy carries the accepted text"
            );
        }
        assert_eq!(
            fs::read_to_string(home.path().join(".claude/skills/sdh-debugging/SKILL.md")).unwrap(),
            texts[1]
        );
        let resolved = resolutions(&db);
        assert_eq!(resolved.len(), 1, "{:?}", resolved);
        assert_eq!(resolved[0].repo.as_deref(), Some(labels[1].as_str()));
        assert_eq!(resolved[0].note.as_deref(), Some(conflict_note::ACCEPTED));
        assert_eq!(db.latest_origin_event(SKILL).unwrap().unwrap(), resolved[0]);
        assert!(db.open_rule_conflicts().unwrap().is_empty());
        assert!(outcome.rendered);
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(outcome.taken.is_empty() && outcome.lost.is_empty());
    }

    /// Rejecting every side leaves the rule alone and puts it back in every copy.
    #[test]
    fn test_rejecting_a_conflict_keeps_the_rule_and_restores_the_copies() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let (dirs, _labels, _texts) = open_conflict(&db);
        let before = rule_row(&db);

        let outcome =
            resolve_rule_conflict_in(&db, Some(home.path().to_path_buf()), SKILL, None).unwrap();

        let row = rule_row(&db);
        assert_eq!(row.content, before.content);
        assert_eq!(row.is_custom, before.is_custom);
        for dir in &dirs {
            assert!(!skills_dir(dir).join("sdh-debugging.md.approved").exists());
            assert!(!skills_dir(dir).join("sdh-debugging.md.conflict").exists());
            assert_eq!(
                fs::read_to_string(skills_dir(dir).join("sdh-debugging.md")).unwrap(),
                before.content,
                "both hand-backs are overwritten by the rule"
            );
        }
        let resolved = resolutions(&db);
        assert_eq!(resolved.len(), 1, "{:?}", resolved);
        assert_eq!(resolved[0].repo, None);
        assert_eq!(
            resolved[0].note.as_deref(),
            Some(conflict_note::REJECTED_BY_OWNER)
        );
        assert!(
            db.latest_origin_event(SKILL).unwrap().is_none(),
            "a rejection writes no text, so it is no origin"
        );
        assert!(db.open_rule_conflicts().unwrap().is_empty());
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    }

    /// Nothing open to decide, or a side that is not there any more: refused
    /// by an exact code, with nothing touched.
    #[test]
    fn test_deciding_a_conflict_that_is_gone_or_a_side_that_is_gone_touches_nothing() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let (dirs, labels, texts) = open_conflict(&db);
        let resolve = |rule: &str, accept: Option<&str>| {
            resolve_rule_conflict_in(&db, Some(home.path().to_path_buf()), rule, accept)
        };
        let untouched = || {
            assert!(db.open_rule_conflicts().unwrap().contains(SKILL));
            assert!(resolutions(&db).is_empty());
            for (dir, text) in dirs.iter().zip(&texts) {
                assert!(skills_dir(dir).join("sdh-debugging.md.approved").exists());
                assert!(skills_dir(dir).join("sdh-debugging.md.conflict").exists());
                assert_eq!(
                    &fs::read_to_string(skills_dir(dir).join("sdh-debugging.md")).unwrap(),
                    text
                );
            }
            assert!(!rule_row(&db).is_custom);
            assert!(!home.path().join(".claude").exists(), "nothing rendered");
        };

        // A rule with no open conflict, accepting or rejecting.
        assert_eq!(
            resolve("sdh-retro", None).unwrap_err(),
            "rule_conflict_gone"
        );
        assert_eq!(
            resolve("sdh-retro", Some(&labels[0])).unwrap_err(),
            "rule_conflict_gone"
        );
        untouched();

        // A label no marker carries.
        assert_eq!(
            resolve(SKILL, Some("nobody/nothing")).unwrap_err(),
            "rule_conflict_side_gone"
        );
        untouched();

        // A side whose marker went away between the screen and the click.
        fs::remove_file(skills_dir(&dirs[0]).join("sdh-debugging.md.approved")).unwrap();
        assert_eq!(
            resolve(SKILL, Some(&labels[0])).unwrap_err(),
            "rule_conflict_side_gone"
        );
        assert!(db.open_rule_conflicts().unwrap().contains(SKILL));
        assert!(!rule_row(&db).is_custom);
        assert!(resolutions(&db).is_empty());
    }

    /// A failed repo listing refuses the decision before anything is touched:
    /// nothing was read, so neither side can be found and no marker cleared.
    #[test]
    fn test_deciding_a_conflict_with_an_unreadable_repo_list_touches_nothing() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let (dirs, labels, texts) = open_conflict(&db);
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE repositories")
            .unwrap();

        for accept in [None, Some(labels[0].as_str())] {
            let err = resolve_rule_conflict_in(&db, Some(home.path().to_path_buf()), SKILL, accept)
                .unwrap_err();
            assert!(
                err != "rule_conflict_gone" && err != "rule_conflict_side_gone",
                "the listing failure is reported as itself: {}",
                err
            );
        }

        assert!(db.open_rule_conflicts().unwrap().contains(SKILL));
        assert!(resolutions(&db).is_empty());
        assert!(!rule_row(&db).is_custom);
        for (dir, text) in dirs.iter().zip(&texts) {
            assert!(skills_dir(dir).join("sdh-debugging.md.approved").exists());
            assert!(skills_dir(dir).join("sdh-debugging.md.conflict").exists());
            assert_eq!(
                &fs::read_to_string(skills_dir(dir).join("sdh-debugging.md")).unwrap(),
                text
            );
        }
    }

    /// An open row with no marker left anywhere (removed by hand, checkouts
    /// gone) must still be closable, or the screen is stuck on it for good.
    #[test]
    fn test_a_conflict_with_no_live_side_can_still_be_rejected() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        db.open_rule_conflict(SKILL, &std::collections::BTreeSet::from([41, 42]))
            .unwrap();

        resolve_rule_conflict_in(&db, Some(home.path().to_path_buf()), SKILL, None).unwrap();

        assert!(db.open_rule_conflicts().unwrap().is_empty());
        let resolved = resolutions(&db);
        assert_eq!(resolved.len(), 1, "{:?}", resolved);
        assert_eq!(
            resolved[0].note.as_deref(),
            Some(conflict_note::REJECTED_BY_OWNER)
        );
    }

    /// The Rules screen reads the sides from the repos, not from the stored row:
    /// label, when, what the agent wrote, and the text itself.
    #[test]
    fn test_the_conflict_view_lists_both_sides_with_their_notes_and_texts() {
        let (db, _db_dir) = seeded();
        let dirs = [TempDir::new().unwrap(), TempDir::new().unwrap()];
        let mut labels = Vec::new();
        for (i, dir) in dirs.iter().enumerate() {
            let path = dir.path().to_string_lossy().to_string();
            db.insert_local_repository(&path, &format!("r{}", i), None, None)
                .unwrap();
            sync::render_skills_to_repo(&db, dir.path()).unwrap();
            labels.push(path);
        }
        crate::sync::skills::test_support::hand_back(
            &skills_dir(&dirs[0]),
            SKILL,
            "TEXT ZERO",
            "  approved in chat \n",
        );
        crate::sync::skills::test_support::hand_back(&skills_dir(&dirs[1]), SKILL, "TEXT ONE", "");

        let sides = sync::conflict_sides(&db, SKILL).unwrap();

        let mut expected = vec![
            (labels[0].clone(), Some("approved in chat"), "TEXT ZERO"),
            (labels[1].clone(), None, "TEXT ONE"),
        ];
        expected.sort();
        let got: Vec<_> = sides
            .iter()
            .map(|s| (s.repo.clone(), s.note.as_deref(), s.content.as_str()))
            .collect();
        assert_eq!(got, expected, "sorted by label");
        for side in &sides {
            let at = side.marked_at.as_deref().expect("a marker time");
            chrono::DateTime::parse_from_rfc3339(at).expect("an RFC 3339 time");
        }
        assert!(
            sync::conflict_sides(&db, "sdh-retro").unwrap().is_empty(),
            "markers of other skills are not sides of this one"
        );
    }

    fn add_checkout(db: &AppDb, path: &str, name: &str) {
        db.insert_local_repository(path, name, None, None).unwrap();
    }

    /// The listing: the global section first, then the skills alphabetically;
    /// `changed` is the text against the bundle, never the `is_custom` flag.
    #[test]
    fn test_listing_rules_orders_them_and_compares_the_text_with_the_bundle() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();

        let fresh = list_rules_in(&db).unwrap();

        assert_eq!(fresh.rules[0].rule, "claude.md.global");
        assert_eq!(fresh.rules[0].file_name, "claude.md.global.tmpl");
        let skills: Vec<&str> = fresh.rules[1..].iter().map(|r| r.rule.as_str()).collect();
        assert!(skills.len() > 1);
        assert!(skills.iter().all(|s| s.starts_with("sdh-")));
        let mut sorted = skills.clone();
        sorted.sort();
        assert_eq!(skills, sorted);
        for rule in &fresh.rules {
            assert!(!rule.changed && !rule.conflict, "{}", rule.rule);
            assert_eq!(rule.canon.as_deref(), Some(rule.content.as_str()));
            assert_eq!(rule.origin, None);
        }

        // A row marked custom that holds exactly the bundled text is unchanged.
        let canon =
            template_seeder::bundled_file_content("_global", "skill.sdh-retro.md.tmpl").unwrap();
        save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
            &canon,
        )
        .unwrap();
        save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-debugging.md.tmpl",
            "OWNER TEXT",
        )
        .unwrap();

        let listed = list_rules_in(&db).unwrap();
        let get = |name: &str| listed.rules.iter().find(|r| r.rule == name).unwrap();
        assert!(
            db.get_template_file("_global", "skill.sdh-retro.md.tmpl")
                .unwrap()
                .unwrap()
                .is_custom
        );
        assert!(!get("sdh-retro").changed, "custom flag is not a change");
        assert!(get("sdh-debugging").changed);
        assert_eq!(get("sdh-debugging").content, "OWNER TEXT");
        assert_eq!(
            get("sdh-debugging").canon.as_deref(),
            Some(
                template_seeder::bundled_file_content("_global", "skill.sdh-debugging.md.tmpl")
                    .unwrap()
                    .as_str()
            )
        );
        let origin = get("sdh-debugging").origin.as_ref().unwrap();
        assert_eq!(origin.kind, "saved");
        assert_eq!(get("claude.md.global").origin, None);
    }

    /// The conflict flag, and the repo count: only checkouts that exist on this
    /// machine are delivered to, so only those are counted.
    #[test]
    fn test_listing_rules_flags_conflicts_and_counts_existing_checkouts_only() {
        let (db, _db_dir) = seeded();
        let (_dirs, _labels, _texts) = open_conflict(&db);
        let elsewhere = TempDir::new().unwrap().path().join("not-on-this-machine");
        add_checkout(&db, &elsewhere.to_string_lossy(), "elsewhere");

        let listed = list_rules_in(&db).unwrap();

        let flagged: Vec<&str> = listed
            .rules
            .iter()
            .filter(|r| r.conflict)
            .map(|r| r.rule.as_str())
            .collect();
        assert_eq!(flagged, vec![SKILL]);
        assert_eq!(listed.repos, 2, "the missing checkout is not counted");
    }

    // ── T-000224: a rule that follows an owner's canon is read-only ──────────────

    /// A rule taken from `owner` through a contributor repo of this install.
    fn follow(db: &AppDb, rule: &str, owner: &str) -> i64 {
        let repo = db
            .insert_local_repository("/nonexistent/shared", "owner/app", None, None)
            .unwrap();
        db.set_shared_role(repo.id, Some("contributor")).unwrap();
        db.set_setting(
            sync::shared_canon::RULES_FROM_MAIN_KEY,
            &format!(
                r#"{{"{}":{{"login":"{}","repo_id":{},"committed_at":1}}}}"#,
                rule, owner, repo.id
            ),
        )
        .unwrap();
        repo.id
    }

    #[test]
    fn test_a_followed_rule_is_flagged_and_refuses_save_and_reset() {
        let (db, _db_dir) = seeded();
        let home = TempDir::new().unwrap();
        let repo_id = follow(&db, "sdh-debugging", "owner");

        let listed = list_rules_in(&db).unwrap();
        for rule in &listed.rules {
            let want = (rule.rule == "sdh-debugging").then(|| "owner".to_string());
            assert_eq!(rule.follows_main, want, "{}", rule.rule);
        }

        let file = "skill.sdh-debugging.md.tmpl";
        let before = db
            .get_template_file("_global", file)
            .unwrap()
            .unwrap()
            .content;
        let save = save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            file,
            "MY EDIT",
        );
        assert_eq!(
            save.unwrap_err(),
            format!("{}: owner", sync::shared_canon::ERR_FOLLOWS_MAIN)
        );
        let reset = reset_template_file_in(&db, Some(home.path().to_path_buf()), "_global", file);
        assert!(reset
            .unwrap_err()
            .starts_with(sync::shared_canon::ERR_FOLLOWS_MAIN));
        assert_eq!(
            db.get_template_file("_global", file)
                .unwrap()
                .unwrap()
                .content,
            before
        );
        assert!(
            !home.path().join(".claude/skills").exists(),
            "nothing rendered"
        );
        // Another rule is not affected.
        save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            "skill.sdh-retro.md.tmpl",
            "MINE",
        )
        .unwrap();

        // The repo stops being shared: the rule is editable again, text kept.
        db.set_shared_role(repo_id, None).unwrap();
        assert!(list_rules_in(&db)
            .unwrap()
            .rules
            .iter()
            .all(|r| r.follows_main.is_none()));
        save_template_file_in(
            &db,
            Some(home.path().to_path_buf()),
            "_global",
            file,
            "MY EDIT",
        )
        .unwrap();
    }
}
