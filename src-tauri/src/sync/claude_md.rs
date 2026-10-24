// CLAUDE.md section rendering. Generates the `<!-- manager:begin -->` /
// `<!-- manager:end -->` block from DB state + bundled templates, both for
// per-repo project context and global AI-rules. Also provides the small
// `copy_doc_skeleton_if_missing` helper for first-time doc creation (todo.md,
// bug-reports.md).

use crate::db::AppDb;
use crate::models::Repository;
use crate::sync::shared_structure::{self as ss, ProjectStructure};
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

    let content = crate::export::read_to_string_lossy(claude_md_path).map_err(|e| e.to_string())?;
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
///
/// `shared_block` fills `{{SHARED_BLOCK}}` (`shared_block_for` of the repo; empty
/// for an ordinary one).
pub fn update_claude_md_section(
    db: &AppDb,
    project_id: Option<i64>,
    repo_role: Option<&str>,
    shared_block: &str,
    claude_md_path: &Path,
) -> Result<(), String> {
    let template_opt = db
        .get_template_file("_global", "claude.md.section.tmpl")
        .map_err(|e| e.to_string())?;
    let template = match template_opt {
        Some(t) => t.content,
        None => return Err("claude.md.section.tmpl not found in DB".to_string()),
    };
    let rendered = render_claude_section(&template, db, project_id, repo_role, shared_block)?;
    write_claude_section(claude_md_path, &rendered)
}

/// F-000044 / T-000222: the shared-repo block of the CLAUDE.md section of `repo`.
///
/// * the owner (shared mode on) or a contributor -> `claude.md.shared.tmpl` with
///   this install's number band (6 digits, no separators);
/// * a pending helper -> `claude.md.shared-pending.tmpl`;
/// * an ordinary repo, or one whose structure cannot be told -> empty.
///
/// `structure` is the trusted structure phase A settled in this sync, if any;
/// otherwise the work-tree file is read. The owner of a repo whose structure
/// file is not written yet is described from this install's login and the
/// repo's own branch. Templates come from the database, like the section's.
pub fn shared_block_for(
    db: &AppDb,
    repo: &Repository,
    structure: Option<&ProjectStructure>,
) -> String {
    let Ok(state) = db.get_shared_state(repo.id) else {
        return String::new();
    };
    #[derive(PartialEq)]
    enum Kind {
        Owner,
        Contributor,
        Pending,
    }
    let kind = if state.shared_mode {
        Kind::Owner
    } else {
        match state.shared_role.as_deref() {
            Some("contributor") => Kind::Contributor,
            Some("pending") => Kind::Pending,
            _ => return String::new(),
        }
    };
    let from_file = || match (repo.local_path.as_deref(), repo.github_name.as_deref()) {
        (Some(path), Some(gh)) => {
            ss::read_structure_file(Path::new(path), gh, &ss::AdminVerified::load(db))
                .ok()
                .flatten()
        }
        _ => None,
    };
    let structure = structure.cloned().or_else(from_file);
    let login = db.github_login();

    // (owner's login, dev branch, band)
    let (main_login, dev_branch, band) = match (&structure, &kind) {
        (Some(s), Kind::Pending) => (s.main.login.clone(), s.main.dev_branch.clone(), None),
        (Some(s), _) => {
            let Some(band) = login.as_deref().and_then(|l| ss::band_for(s, l)) else {
                return String::new();
            };
            (s.main.login.clone(), s.main.dev_branch.clone(), Some(band))
        }
        (None, Kind::Owner) => {
            let (Some(login), Some(branch)) = (
                login.clone(),
                db.get_autocommit_branch(repo.id)
                    .ok()
                    .flatten()
                    .map(|b| b.trim().to_string())
                    .filter(|b| !b.is_empty()),
            ) else {
                return String::new();
            };
            // Nobody has a band yet: the owner has everything below the first one.
            (login, branch, Some((1, ss::FIRST_PARTICIPANT_BAND - 1)))
        }
        (None, _) => return String::new(),
    };

    let file = if kind == Kind::Pending {
        "claude.md.shared-pending.tmpl"
    } else {
        "claude.md.shared.tmpl"
    };
    let Some(template) = db.get_template_file("_global", file).ok().flatten() else {
        return String::new();
    };
    let mut out = template
        .content
        .replace("{{MAIN_LOGIN}}", &main_login)
        .replace("{{DEV_BRANCH}}", &dev_branch);
    if let Some((lo, hi)) = band {
        out = out
            .replace("{{BAND_LO}}", &format!("{:06}", lo))
            .replace("{{BAND_HI}}", &format!("{:06}", hi));
    }
    out.trim().to_string()
}

/// Global `~/.claude/CLAUDE.md`: format rules (todo/done/bug-reports), no project context.
/// Uses `claude.md.global.tmpl`. No placeholders — content is inserted verbatim between markers.
pub fn update_claude_md_global(db: &AppDb, claude_md_path: &Path) -> Result<(), String> {
    let template_opt = db
        .get_template_file("_global", super::GLOBAL_SECTION_FILE)
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
    shared_block: &str,
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
    // F-000044: the shared-repo block; an ordinary repo has none, and then the
    // blank line that separates it goes too.
    let rendered = if shared_block.trim().is_empty() {
        rendered
            .replace("{{SHARED_BLOCK}}\n\n", "")
            .replace("{{SHARED_BLOCK}}", "")
    } else {
        rendered.replace("{{SHARED_BLOCK}}", shared_block.trim())
    };

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

        update_claude_md_section(&db, Some(proj.id), Some("server"), "", &claude).unwrap();

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

        update_claude_md_section(&db, Some(proj.id), None, "", &claude).unwrap();

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

        update_claude_md_section(&db, Some(proj.id), None, "", &claude).unwrap();

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

        update_claude_md_section(&db, Some(proj.id), None, "", &claude).unwrap();

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

        update_claude_md_section(&db, Some(proj.id), None, "", &claude).unwrap();

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

        let result = update_claude_md_section(&db, Some(proj.id), None, "", &claude);
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

        let result = update_claude_md_section(&db, Some(proj.id), None, "", &claude);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Orphan"));
    }

    #[test]
    fn test_update_claude_section_global_rendering() {
        let db = make_test_db();
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");

        update_claude_md_section(&db, None, None, "", &claude).unwrap();

        let content = fs::read_to_string(&claude).unwrap();
        assert!(content.contains("manager:begin"));
        assert!(
            content.contains("—"),
            "global placeholders should contain dashes"
        );
        assert!(content.contains("Global AI instructions"));
        assert!(content.contains("_not applicable_"));
    }

    // ── shared block (F-000044 / T-000222) ───────────────────────────────────

    fn structure(participants: &[(&str, u32)]) -> ProjectStructure {
        use crate::sync::shared_structure::{Participant, StructMain, StructProject, StructRepo};
        ProjectStructure {
            format: 1,
            project: StructProject {
                name: "P".into(),
                project_type: "standard".into(),
            },
            main: StructMain {
                login: "owner".into(),
                dev_branch: "dev".into(),
            },
            repos: vec![StructRepo {
                github: "owner/app".into(),
                role: "tool".into(),
            }],
            microservices: vec![],
            participants: participants
                .iter()
                .map(|(l, b)| Participant {
                    login: l.to_string(),
                    branch: format!("{}-dev", l),
                    band_start: *b,
                })
                .collect(),
        }
    }

    /// A repo of the test db: `mode` is the owner's switch, `role` the cached role.
    fn shared_repo(db: &AppDb, mode: bool, role: Option<&str>) -> Repository {
        let repo = db
            .insert_local_repository("/nonexistent/app", "owner/app", None, None)
            .unwrap();
        db.set_shared_mode(repo.id, mode).unwrap();
        db.set_shared_role(repo.id, role).unwrap();
        db.get_repository(repo.id).unwrap()
    }

    fn section(db: &AppDb, repo: &Repository, s: Option<&ProjectStructure>) -> String {
        let pid = match db.list_projects().unwrap().first() {
            Some(p) => p.id,
            None => {
                db.create_project("Proj", Some("Desc"), "standard")
                    .unwrap()
                    .id
            }
        };
        let tmp = TempDir::new().unwrap();
        let claude = tmp.path().join("CLAUDE.md");
        let block = shared_block_for(db, repo, s);
        update_claude_md_section(db, Some(pid), Some("tool"), &block, &claude).unwrap();
        fs::read_to_string(&claude).unwrap()
    }

    #[test]
    fn test_section_of_a_non_shared_repo_has_no_block_and_no_stray_blank_line() {
        let db = make_test_db();
        let repo = shared_repo(&db, false, None);
        let text = section(&db, &repo, None);
        assert!(!text.contains("{{"), "no placeholder left:\n{}", text);
        assert!(!text.contains("Shared repo"));
        assert!(
            text.contains(
                "**Role of this repo:** tool

Desc"
            ),
            "the description follows after one blank line:
{}",
            text
        );
    }

    #[test]
    fn test_section_of_the_owner_names_the_login_branch_and_the_low_band() {
        let db = make_test_db();
        db.set_setting("github_login", "owner").unwrap();
        let repo = shared_repo(&db, true, Some("main"));
        let s = structure(&[("helper", 500_000)]);
        let text = section(&db, &repo, Some(&s));
        assert!(text.contains("## Shared repo"), "{}", text);
        assert!(
            text.contains("Owner: `owner`, dev branch `dev`"),
            "{}",
            text
        );
        assert!(text.contains("`000001`–`499999`"), "{}", text);
        assert!(!text.contains("{{"));
    }

    #[test]
    fn test_section_of_a_contributor_has_his_own_band() {
        let db = make_test_db();
        db.set_setting("github_login", "helper").unwrap();
        let repo = shared_repo(&db, false, Some("contributor"));
        let s = structure(&[("helper", 500_000), ("other", 600_000)]);
        let text = section(&db, &repo, Some(&s));
        assert!(
            text.contains("Owner: `owner`, dev branch `dev`"),
            "{}",
            text
        );
        assert!(text.contains("`500000`–`599999`"), "{}", text);
        let text = {
            db.set_setting("github_login", "other").unwrap();
            section(&db, &repo, Some(&s))
        };
        assert!(text.contains("`600000`–`699999`"), "{}", text);
    }

    #[test]
    fn test_section_of_a_pending_helper_uses_the_pending_wording() {
        let db = make_test_db();
        db.set_setting("github_login", "newbie").unwrap();
        let repo = shared_repo(&db, false, Some("pending"));
        let s = structure(&[("helper", 500_000)]);
        let text = section(&db, &repo, Some(&s));
        assert!(text.contains("## Shared repo"), "{}", text);
        assert!(
            text.contains("waiting for the owner's approval"),
            "{}",
            text
        );
        assert!(
            !text.contains("number band"),
            "no band for a pending helper"
        );
        assert!(!text.contains("{{"));
    }

    #[test]
    fn test_section_of_an_owner_without_a_structure_file_falls_back_to_his_branch() {
        let db = make_test_db();
        db.set_setting("github_login", "owner").unwrap();
        let repo = shared_repo(&db, true, None);
        db.set_autocommit_branch(repo.id, Some("dev")).unwrap();
        let text = section(&db, &repo, None);
        assert!(
            text.contains("Owner: `owner`, dev branch `dev`"),
            "{}",
            text
        );
        assert!(text.contains("`000001`–`499999`"), "{}", text);
        // Without a login there is nothing to say: no block, no leftovers.
        db.delete_setting("github_login").unwrap();
        let text = section(&db, &repo, None);
        assert!(!text.contains("Shared repo") && !text.contains("{{"));
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
