// CLAUDE.md section rendering. Generates the `<!-- manager:begin -->` /
// `<!-- manager:end -->` block from DB state + bundled templates, both for
// per-repo project context and global AI-rules. Also provides the small
// `copy_doc_skeleton_if_missing` helper for first-time doc creation (todo.md,
// bug-reports.md).

use crate::db::AppDb;
use regex::Regex;
use std::fs;
use std::path::Path;

/// Copy a skeleton template to `docs/<relative_name>` if file does not exist.
/// Empty template = no-op. Returns true if file was created.
/// Used for F-016: todo.md and bug-reports.md skeletons.
pub fn copy_doc_skeleton_if_missing(
    template: &str,
    repo_root: &Path,
    relative_name: &str,
) -> Result<bool, String> {
    if template.trim().is_empty() {
        return Ok(false);
    }
    let docs_dir = repo_root.join("docs");
    let target = docs_dir.join(relative_name);
    if target.exists() {
        return Ok(false);
    }
    fs::create_dir_all(&docs_dir).map_err(|e| e.to_string())?;
    fs::write(&target, template).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Write a pre-rendered block between `<!-- manager:begin -->` / `<!-- manager:end -->` markers.
/// If the file doesn't exist — created with just the block. If markers present — replaced in place.
/// If no markers — appended to the end. Orphan begin/end → error (user must fix manually).
fn write_claude_section(claude_md_path: &Path, rendered: &str) -> Result<(), String> {
    let section = format!(
        "<!-- manager:begin -->\n{}\n<!-- manager:end -->",
        rendered.trim()
    );

    if !claude_md_path.exists() {
        if let Some(parent) = claude_md_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(claude_md_path, &section).map_err(|e| e.to_string())?;
        return Ok(());
    }

    let content = fs::read_to_string(claude_md_path).map_err(|e| e.to_string())?;
    let has_begin = content.contains("manager:begin");
    let has_end = content.contains("manager:end");

    match (has_begin, has_end) {
        (true, true) => {
            let re =
                Regex::new(r"(?s)<!--\s*manager:begin\s*-->.*?<!--\s*manager:end\s*-->").unwrap();
            let updated = re.replace(&content, section.as_str()).to_string();
            fs::write(claude_md_path, updated).map_err(|e| e.to_string())?;
        }
        (false, false) => {
            let updated = format!("{}\n\n{}\n", content.trim_end(), section);
            fs::write(claude_md_path, updated).map_err(|e| e.to_string())?;
        }
        (true, false) => {
            return Err("Orphan <!-- manager:begin --> marker without matching end — fix CLAUDE.md manually".to_string());
        }
        (false, true) => {
            return Err("Orphan <!-- manager:end --> marker without matching begin — fix CLAUDE.md manually".to_string());
        }
    }
    Ok(())
}

/// Per-repo CLAUDE.md: project context (name, repos, microservices, parents).
/// Uses `claude.md.section.tmpl`. Rendered with project-specific placeholders.
pub fn update_claude_md_section(
    db: &AppDb,
    project_id: Option<i64>,
    repo_role: Option<&str>,
    claude_md_path: &Path,
) -> Result<(), String> {
    let template_opt = db
        .get_template_file("_global", "claude.md.section.tmpl")
        .map_err(|e| e.to_string())?;
    let template = match template_opt {
        Some(t) => t.content,
        None => return Err("claude.md.section.tmpl not found in DB".to_string()),
    };
    let rendered = render_claude_section(&template, db, project_id, repo_role)?;
    write_claude_section(claude_md_path, &rendered)
}

/// Global `~/.claude/CLAUDE.md`: format rules (todo/done/bug-reports), no project context.
/// Uses `claude.md.global.tmpl`. No placeholders — content is inserted verbatim between markers.
pub fn update_claude_md_global(db: &AppDb, claude_md_path: &Path) -> Result<(), String> {
    let template_opt = db
        .get_template_file("_global", "claude.md.global.tmpl")
        .map_err(|e| e.to_string())?;
    let template = match template_opt {
        Some(t) => t.content,
        None => return Err("claude.md.global.tmpl not found in DB".to_string()),
    };
    write_claude_section(claude_md_path, &template)
}

fn render_claude_section(
    template: &str,
    db: &AppDb,
    project_id: Option<i64>,
    repo_role: Option<&str>,
) -> Result<String, String> {
    let (name, type_display, description, repos_table, ms_block, parents_block) = if let Some(pid) =
        project_id
    {
        let project = db.get_project(pid).map_err(|e| e.to_string())?;
        let repos = db
            .list_repos_by_project(Some(pid))
            .map_err(|e| e.to_string())?;
        let ms_ids = db
            .list_project_microservices(pid)
            .map_err(|e| e.to_string())?;
        let parents = db
            .list_parents_of_microservice(pid)
            .map_err(|e| e.to_string())?;

        let td = match project.project_type.as_str() {
            "microservice" => "⚙ Microservice",
            _ => "📁 Standard",
        };

        let mut rt = String::new();
        if repos.is_empty() {
            rt.push_str("_No repositories._");
        } else {
            rt.push_str("| Repository | Role |\n|------------|------|\n");
            for r in &repos {
                rt.push_str(&format!(
                    "| {} | {} |\n",
                    r.display_name(),
                    r.role.as_deref().unwrap_or("—")
                ));
            }
        }

        let mut mb = String::new();
        if ms_ids.is_empty() {
            mb.push_str("_No connected microservices._");
        } else {
            for ms_id in &ms_ids {
                if let Ok(ms_proj) = db.get_project(*ms_id) {
                    mb.push_str(&format!("- {}\n", ms_proj.name));
                }
            }
        }

        let mut pb = String::new();
        if parents.is_empty() {
            pb.push_str("_No parent projects._");
        } else {
            for p in &parents {
                pb.push_str(&format!("- {}\n", p.name));
            }
        }

        (
            project.name,
            td.to_string(),
            project.description.unwrap_or_default(),
            rt,
            mb,
            pb,
        )
    } else {
        (
                "—".to_string(),
                "—".to_string(),
                "Global AI instructions (configured by the user in Solo Dev Hub). For per-project context see CLAUDE.md of the specific repo.".to_string(),
                "_not applicable_".to_string(),
                "_not applicable_".to_string(),
                "_not applicable_".to_string(),
            )
    };

    let role_display = repo_role.unwrap_or("—");

    let rendered = template
        .replace("{{PROJECT_NAME}}", &name)
        .replace("{{PROJECT_TYPE_DISPLAY}}", &type_display)
        .replace("{{REPO_ROLE_OR_UNSET}}", role_display)
        .replace("{{PROJECT_DESCRIPTION}}", &description)
        .replace("{{REPOS_TABLE}}", &repos_table)
        .replace("{{MICROSERVICES_BLOCK}}", &ms_block)
        .replace("{{PARENTS_BLOCK}}", &parents_block);

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::gitignore::sync_gitignore_section;
    use std::fs;
    use tempfile::TempDir;

    // ── update_claude_md_section tests (no DB needed — use create_file tests) ─

    fn make_test_db() -> AppDb {
        let dir = TempDir::new().unwrap();
        let db = AppDb::new(dir.path().join("test.db")).unwrap();
        use crate::template_seeder::seed_bundled_templates;
        seed_bundled_templates(&db).unwrap();
        db
    }

    #[test]
    fn test_update_claude_section_creates_file_if_missing() {
        let db = make_test_db();
        let proj = db
            .create_project("TestP", Some("Desc"), "standard")
            .unwrap();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");

        update_claude_md_section(&db, Some(proj.id), Some("server"), &claude).unwrap();

        let content = fs::read_to_string(&claude).unwrap();
        assert!(content.contains("manager:begin"));
        assert!(content.contains("manager:end"));
        assert!(content.contains("TestP"));
    }

    #[test]
    fn test_update_claude_section_appends_when_no_markers() {
        let db = make_test_db();
        let proj = db.create_project("App", None, "standard").unwrap();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        fs::write(&claude, "# My custom rules\n\nDo not touch.\n").unwrap();

        update_claude_md_section(&db, Some(proj.id), None, &claude).unwrap();

        let content = fs::read_to_string(&claude).unwrap();
        assert!(
            content.starts_with("# My custom rules"),
            "user content preserved at top"
        );
        assert!(content.contains("manager:begin"));
        assert!(content.contains("App"));
    }

    #[test]
    fn test_update_claude_section_replaces_between_markers() {
        let db = make_test_db();
        let proj = db.create_project("V1", None, "standard").unwrap();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        fs::write(
            &claude,
            "top\n<!-- manager:begin -->\nold stuff\n<!-- manager:end -->\nbottom\n",
        )
        .unwrap();

        update_claude_md_section(&db, Some(proj.id), None, &claude).unwrap();

        let content = fs::read_to_string(&claude).unwrap();
        assert!(content.contains("V1"), "new project name present");
        assert!(!content.contains("old stuff"), "old content replaced");
        assert!(content.contains("top\n"), "text before markers preserved");
        assert!(content.contains("bottom"), "text after markers preserved");
    }

    #[test]
    fn test_update_claude_section_preserves_user_content() {
        let db = make_test_db();
        let proj = db.create_project("Proj", None, "standard").unwrap();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        let original = "# Rules\n\nImportant rule.\n\n<!-- manager:begin -->\nplaceholder\n<!-- manager:end -->\n\n# More rules\n\nAnother important thing.\n";
        fs::write(&claude, original).unwrap();

        update_claude_md_section(&db, Some(proj.id), None, &claude).unwrap();

        let content = fs::read_to_string(&claude).unwrap();
        assert!(content.contains("# Rules\n\nImportant rule."));
        assert!(content.contains("# More rules\n\nAnother important thing."));
        assert!(content.contains("Proj"));
    }

    #[test]
    fn test_update_claude_section_multiple_markers_replaces_first() {
        let db = make_test_db();
        let proj = db.create_project("First", None, "standard").unwrap();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        let content = "<!-- manager:begin -->\nold1\n<!-- manager:end -->\nmiddle\n<!-- manager:begin -->\nold2\n<!-- manager:end -->\n";
        fs::write(&claude, content).unwrap();

        update_claude_md_section(&db, Some(proj.id), None, &claude).unwrap();

        let result = fs::read_to_string(&claude).unwrap();
        assert!(result.contains("First"), "first pair replaced");
        assert!(result.contains("old2"), "second pair untouched");
    }

    #[test]
    fn test_update_claude_section_orphan_begin_marker() {
        let db = make_test_db();
        let proj = db.create_project("X", None, "standard").unwrap();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        fs::write(&claude, "stuff\n<!-- manager:begin -->\nno end marker\n").unwrap();

        let result = update_claude_md_section(&db, Some(proj.id), None, &claude);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Orphan"));
    }

    #[test]
    fn test_update_claude_section_orphan_end_marker() {
        let db = make_test_db();
        let proj = db.create_project("Y", None, "standard").unwrap();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        fs::write(&claude, "stuff\n<!-- manager:end -->\nno begin\n").unwrap();

        let result = update_claude_md_section(&db, Some(proj.id), None, &claude);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Orphan"));
    }

    #[test]
    fn test_update_claude_section_global_rendering() {
        let db = make_test_db();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");

        update_claude_md_section(&db, None, None, &claude).unwrap();

        let content = fs::read_to_string(&claude).unwrap();
        assert!(content.contains("manager:begin"));
        assert!(
            content.contains("—"),
            "global placeholders should contain dashes"
        );
        assert!(content.contains("Global AI instructions"));
        assert!(content.contains("_not applicable_"));
    }

    // ── copy_doc_skeleton_if_missing tests (F-016) ───────────────────────────

    #[test]
    fn test_copy_doc_skeleton_when_missing() {
        let tmp = TempDir::new().unwrap();
        let created = copy_doc_skeleton_if_missing("# Todo\n", tmp.path(), "todo.md").unwrap();
        assert!(created);
        let target = tmp.path().join("docs/todo.md");
        assert!(target.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "# Todo\n");
    }

    #[test]
    fn test_copy_doc_skeleton_skips_existing() {
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        fs::create_dir_all(&docs).unwrap();
        fs::write(docs.join("todo.md"), "existing user content").unwrap();

        let created =
            copy_doc_skeleton_if_missing("# New skeleton", tmp.path(), "todo.md").unwrap();
        assert!(!created);
        assert_eq!(
            fs::read_to_string(docs.join("todo.md")).unwrap(),
            "existing user content",
            "existing file must not be overwritten"
        );
    }

    #[test]
    fn test_copy_doc_skeleton_empty_template_noop() {
        let tmp = TempDir::new().unwrap();
        let created = copy_doc_skeleton_if_missing("  \n  ", tmp.path(), "bug-reports.md").unwrap();
        assert!(!created);
        assert!(!tmp.path().join("docs/bug-reports.md").exists());
        assert!(
            !tmp.path().join("docs").exists(),
            "docs dir not created for empty template"
        );
    }

    /// Simulates init_docs_for_repo on a bare folder (local-only repo, no .git).
    /// Verifies all 3 skeletons get created: docs/todo.md, docs/bug-reports.md, .gitignore.
    #[test]
    fn test_init_docs_flow_on_bare_folder() {
        let db = make_test_db();
        let todo_t = db
            .get_template_file("_global", "todo.md.tmpl")
            .unwrap()
            .unwrap()
            .content;
        let bugs_t = db
            .get_template_file("_global", "bug-reports.md.tmpl")
            .unwrap()
            .unwrap()
            .content;
        let gi_t = db
            .get_template_file("_global", ".gitignore.tmpl")
            .unwrap()
            .unwrap()
            .content;
        assert!(!todo_t.trim().is_empty());
        assert!(!bugs_t.trim().is_empty());
        assert!(!gi_t.trim().is_empty());

        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let c1 = copy_doc_skeleton_if_missing(&todo_t, base, "todo.md").unwrap();
        let c2 = copy_doc_skeleton_if_missing(&bugs_t, base, "bug-reports.md").unwrap();
        let c3 = sync_gitignore_section(&gi_t, base).unwrap();

        assert!(
            c1 && c2 && c3,
            "all 3 skeletons should be created on bare folder"
        );
        assert!(base.join("docs/todo.md").exists());
        assert!(base.join("docs/bug-reports.md").exists());
        let gi_content = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert!(gi_content.contains("solo-dev-hub:begin"));
        assert!(gi_content.contains("solo-dev-hub:end"));
    }

    // ── sync_global_claude_md timestamp persistence (F-000036 Task 5) ────────

    #[test]
    fn test_sync_global_claude_md_sets_last_sync_at() {
        let db = make_test_db();
        let tmp = TempDir::new().unwrap();
        let claude_path = tmp.path().join("CLAUDE.md");

        // Pre: setting absent
        assert!(db.get_setting("ai_rules_last_sync_at").unwrap().is_none());

        // Act: simulate the command path (call update_claude_md_global + set_setting)
        update_claude_md_global(&db, &claude_path).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        db.set_setting("ai_rules_last_sync_at", &now).unwrap();

        // Assert: setting populated with valid RFC3339 timestamp within last 5s
        let stored = db.get_setting("ai_rules_last_sync_at").unwrap();
        assert!(stored.is_some());
        let parsed = chrono::DateTime::parse_from_rfc3339(&stored.unwrap()).unwrap();
        let delta = chrono::Utc::now() - parsed.with_timezone(&chrono::Utc);
        assert!(delta.num_seconds() < 5);
    }

    #[test]
    fn test_sync_global_claude_md_does_not_set_on_failure() {
        let db = make_test_db();
        let tmp = TempDir::new().unwrap();
        // Parent path is a regular file, not a directory — create_dir_all + write must fail.
        let parent_as_file = tmp.path().join("not_a_dir");
        fs::write(&parent_as_file, "i am a file").unwrap();
        let bad_path = parent_as_file.join("CLAUDE.md");

        // Pre: setting absent
        assert!(db.get_setting("ai_rules_last_sync_at").unwrap().is_none());

        // Act — expect failure
        let result = update_claude_md_global(&db, &bad_path);
        assert!(result.is_err(), "writing under a file-as-parent must fail");

        // Assert: setting still absent (we never reached the set_setting call)
        assert!(db.get_setting("ai_rules_last_sync_at").unwrap().is_none());
    }
}
