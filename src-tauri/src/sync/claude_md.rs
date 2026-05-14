// CLAUDE.md / project.md / .gitignore section rendering. Generates docs files
// from DB state + bundled templates; manages the `<!-- manager:begin -->` /
// `<!-- manager:end -->` block and the `solo-dev-hub:begin` / `solo-dev-hub:end`
// gitignore block.

use crate::db::AppDb;
use regex::Regex;
use std::fs;
use std::path::Path;

/// Sync a managed section in .gitignore between markers — with **dedup logic**.
/// Приципы:
/// - Из template берём только rule-строки (не комментарии и не пустые).
/// - Отфильтровываем те, которые **уже есть** в user-контенте (exact-match trimmed).
/// - Оставшиеся правила (если есть) → блок между маркерами.
/// - Если новых правил нет — блок удаляется (или не создаётся).
///
/// Returns true if file was modified.
pub fn sync_gitignore_section(template: &str, repo_root: &Path) -> Result<bool, String> {
    if template.trim().is_empty() {
        return Ok(false);
    }
    let target = repo_root.join(".gitignore");

    let existing = if target.exists() {
        fs::read_to_string(&target).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    let has_begin = existing.contains("solo-dev-hub:begin");
    let has_end = existing.contains("solo-dev-hub:end");

    // User content = everything outside any existing managed block.
    // Self-heal orphan markers: if only one marker present, just strip that line
    // and treat rest as user content — block will be rebuilt below.
    let block_re =
        Regex::new(r"(?s)\n*# --- solo-dev-hub:begin.*?# --- solo-dev-hub:end ---\n*").unwrap();
    let user_content = if has_begin && has_end {
        block_re.replace(&existing, "\n").to_string()
    } else if has_begin || has_end {
        // Orphan: strip any line containing either marker string, keep rest as user content.
        existing
            .lines()
            .filter(|line| !line.contains("solo-dev-hub:begin") && !line.contains("solo-dev-hub:end"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        existing.clone()
    };

    // Collect user's actual rules (trimmed, non-comment, non-empty).
    use std::collections::HashSet;
    let user_rules: HashSet<&str> = user_content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    // Extract rules from template only (drop comments) and keep those NOT already in user_rules.
    let new_rules: Vec<&str> = template
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| !user_rules.contains(l))
        .collect();

    // Decide desired final content.
    let user_trimmed = user_content.trim_end();
    let desired = if new_rules.is_empty() {
        if user_trimmed.is_empty() {
            String::new()
        } else {
            format!("{}\n", user_trimmed)
        }
    } else {
        let block = format!(
            "# --- solo-dev-hub:begin ---\n{}\n# --- solo-dev-hub:end ---",
            new_rules.join("\n")
        );
        if user_trimmed.is_empty() {
            format!("{}\n", block)
        } else {
            format!("{}\n\n{}\n", user_trimmed, block)
        }
    };

    if desired == existing {
        return Ok(false);
    }
    fs::write(&target, desired).map_err(|e| e.to_string())?;
    Ok(true)
}

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

/// Generate docs/project.md describing the project's structure.
/// Overwrites existing file on each call (app-owned).
pub fn generate_project_md(db: &AppDb, project_id: i64, repo_root: &Path) -> Result<(), String> {
    let project = db.get_project(project_id).map_err(|e| e.to_string())?;
    let repos = db
        .list_repos_by_project(Some(project_id))
        .map_err(|e| e.to_string())?;
    let ms_ids = db
        .list_project_microservices(project_id)
        .map_err(|e| e.to_string())?;
    let parents = db
        .list_parents_of_microservice(project_id)
        .map_err(|e| e.to_string())?;

    let type_display = match project.project_type.as_str() {
        "microservice" => "⚙ Microservice",
        _ => "📁 Standard",
    };

    let mut md = format!(
        "# {}\n\n{}\n\n**Project type:** {}\n\n",
        project.name,
        project.description.as_deref().unwrap_or(""),
        type_display,
    );

    md.push_str("## Repositories\n\n");
    if repos.is_empty() {
        md.push_str("_No repositories._\n\n");
    } else {
        md.push_str("| Repository | Role | Path | GitHub |\n");
        md.push_str("|------------|------|------|--------|\n");
        for r in &repos {
            let name = r.display_name();
            let role = r.role.as_deref().unwrap_or("—");
            let path = r.local_path.as_deref().unwrap_or("—");
            let gh = if r.github_name.is_some() { "✓" } else { "📁 local" };
            md.push_str(&format!("| {} | {} | {} | {} |\n", name, role, path, gh));
        }
        md.push('\n');
    }

    md.push_str("## Connected microservices\n\n");
    if ms_ids.is_empty() {
        md.push_str("_No connected microservices._\n\n");
    } else {
        for ms_id in &ms_ids {
            let ms_proj = db.get_project(*ms_id).map_err(|e| e.to_string())?;
            let srv_label = match db.server_repo_of_microservice(*ms_id) {
                Ok(srv) => srv.display_name(),
                Err(_) => "⚠ no server repo".to_string(),
            };
            md.push_str(&format!("- **{}** — server repo: {}\n", ms_proj.name, srv_label));
        }
        md.push('\n');
    }

    md.push_str("## Parent projects\n\n");
    if parents.is_empty() {
        md.push_str("_No parent projects._\n\n");
    } else {
        // F-000040: include each parent server-repo's local path so MS-LLM
        // can write proactive announcements directly into that filesystem.
        // server_repo_of_microservice() is generic over project_id and
        // returns the server-role repo of any project (despite its name).
        for p in &parents {
            match db.server_repo_of_microservice(p.id) {
                Ok(srv) => {
                    let srv_label = srv.display_name();
                    let path_label = match srv.local_path.as_deref() {
                        Some(path) => format!("path: {}", path),
                        None => "no local path configured".to_string(),
                    };
                    md.push_str(&format!(
                        "- **{}** — server repo: {} ({})\n",
                        p.name, srv_label, path_label
                    ));
                }
                Err(_) => {
                    md.push_str(&format!(
                        "- **{}** — ⚠ server repo not resolvable\n",
                        p.name
                    ));
                }
            }
        }
        md.push('\n');
    }

    md.push_str("---\n_Auto-generated by Solo Dev Hub. Do not edit manually — changes will be overwritten on next sync._\n");

    let docs_dir = repo_root.join("docs");
    fs::create_dir_all(&docs_dir).map_err(|e| e.to_string())?;
    fs::write(docs_dir.join("project.md"), md).map_err(|e| e.to_string())?;
    Ok(())
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
            let re = Regex::new(r"(?s)<!--\s*manager:begin\s*-->.*?<!--\s*manager:end\s*-->")
                .unwrap();
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
    let (name, type_display, description, repos_table, ms_block, parents_block) =
        if let Some(pid) = project_id {
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
        let proj = db.create_project("TestP", Some("Desc"), "standard").unwrap();
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
        assert!(content.starts_with("# My custom rules"), "user content preserved at top");
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
        assert!(content.contains("—"), "global placeholders should contain dashes");
        assert!(content.contains("Global AI instructions"));
        assert!(content.contains("_not applicable_"));
    }

    // ── generate_project_md tests ────────────────────────────────────────────

    #[test]
    fn test_generate_project_md_standard_with_repos() {
        let db = make_test_db();
        let proj = db.create_project("WebApp", Some("My web application"), "standard").unwrap();
        let repo = db.upsert_repository("owner/web-backend", None, None, None, None, None).unwrap();
        db.assign_repository(repo.id, Some(proj.id), Some("server")).unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, proj.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("# WebApp"));
        assert!(content.contains("My web application"));
        assert!(content.contains("📁 Standard"));
        // B-000001: display_name() returns last segment, not full owner/repo.
        assert!(content.contains("web-backend"));
        assert!(!content.contains("owner/web-backend"));
        assert!(content.contains("server"));
    }

    #[test]
    fn test_generate_project_md_microservice_with_parents() {
        let db = make_test_db();
        let parent = db.create_project("Parent", None, "standard").unwrap();
        let ms = db.create_project("AuthMS", None, "microservice").unwrap();
        let ms_repo = db.upsert_repository("owner/auth-backend", None, None, None, None, None).unwrap();
        db.assign_repository(ms_repo.id, Some(ms.id), Some("server")).unwrap();
        db.connect_microservice(parent.id, ms.id).unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, ms.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("# AuthMS"));
        assert!(content.contains("⚙ Microservice"));
        assert!(content.contains("Parent"), "parent project listed");
    }

    /// F-000040: Parent projects section must include parent server-repo's
    /// local path so MS-LLM can write announcements directly into that filesystem.
    #[test]
    fn test_generate_project_md_microservice_parent_includes_server_path() {
        let db = make_test_db();
        let parent = db.create_project("WebApp", None, "standard").unwrap();
        let parent_srv = db
            .upsert_repository("owner/web-app-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(parent_srv.id, Some(parent.id), Some("server"))
            .unwrap();
        db.set_repo_local_path(parent_srv.id, Some("/home/dev/web-app-backend"))
            .unwrap();

        let ms = db.create_project("Storage", None, "microservice").unwrap();
        let ms_repo = db
            .upsert_repository("owner/storage-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(ms_repo.id, Some(ms.id), Some("server"))
            .unwrap();
        db.connect_microservice(parent.id, ms.id).unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, ms.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        // MS-LLM consumes path from this line to write announcements to parent
        assert!(
            content.contains("- **WebApp** — server repo: web-app-backend (path: /home/dev/web-app-backend)"),
            "parent rendering should include canonical server-repo name + local path; got:\n{}",
            content
        );
    }

    /// F-000040: Parent server repo without local_path renders graceful placeholder
    /// (announcement-write is impossible in that state, but project.md still renders).
    #[test]
    fn test_generate_project_md_microservice_parent_without_server_path() {
        let db = make_test_db();
        let parent = db.create_project("WebApp", None, "standard").unwrap();
        let parent_srv = db
            .upsert_repository("owner/web-app-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(parent_srv.id, Some(parent.id), Some("server"))
            .unwrap();
        // intentionally no set_repo_local_path

        let ms = db.create_project("Storage", None, "microservice").unwrap();
        let ms_repo = db
            .upsert_repository("owner/storage-backend", None, None, None, None, None)
            .unwrap();
        db.assign_repository(ms_repo.id, Some(ms.id), Some("server"))
            .unwrap();
        db.connect_microservice(parent.id, ms.id).unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, ms.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(
            content.contains("- **WebApp** — server repo: web-app-backend (no local path configured)"),
            "should signal missing path explicitly; got:\n{}",
            content
        );
    }

    #[test]
    fn test_generate_project_md_fallback_placeholders_when_empty() {
        let db = make_test_db();
        let proj = db.create_project("Empty", None, "standard").unwrap();

        let tmp = TempDir::new().unwrap();
        generate_project_md(&db, proj.id, tmp.path()).unwrap();

        let content = fs::read_to_string(tmp.path().join("docs/project.md")).unwrap();
        assert!(content.contains("_No repositories._"));
        assert!(content.contains("_No connected microservices._"));
        assert!(content.contains("_No parent projects._"));
        assert!(!content.contains("{{"));
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

        let created = copy_doc_skeleton_if_missing("# New skeleton", tmp.path(), "todo.md").unwrap();
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
        assert!(!tmp.path().join("docs").exists(), "docs dir not created for empty template");
    }

    /// Simulates init_docs_for_repo on a bare folder (local-only repo, no .git).
    /// Verifies all 3 skeletons get created: docs/todo.md, docs/bug-reports.md, .gitignore.
    #[test]
    fn test_init_docs_flow_on_bare_folder() {
        let db = make_test_db();
        let todo_t = db.get_template_file("_global", "todo.md.tmpl").unwrap().unwrap().content;
        let bugs_t = db.get_template_file("_global", "bug-reports.md.tmpl").unwrap().unwrap().content;
        let gi_t = db.get_template_file("_global", ".gitignore.tmpl").unwrap().unwrap().content;
        assert!(!todo_t.trim().is_empty());
        assert!(!bugs_t.trim().is_empty());
        assert!(!gi_t.trim().is_empty());

        let tmp = TempDir::new().unwrap();
        let base = tmp.path();

        let c1 = copy_doc_skeleton_if_missing(&todo_t, base, "todo.md").unwrap();
        let c2 = copy_doc_skeleton_if_missing(&bugs_t, base, "bug-reports.md").unwrap();
        let c3 = sync_gitignore_section(&gi_t, base).unwrap();

        assert!(c1 && c2 && c3, "all 3 skeletons should be created on bare folder");
        assert!(base.join("docs/todo.md").exists());
        assert!(base.join("docs/bug-reports.md").exists());
        let gi_content = fs::read_to_string(base.join(".gitignore")).unwrap();
        assert!(gi_content.contains("solo-dev-hub:begin"));
        assert!(gi_content.contains("solo-dev-hub:end"));
    }

    // ── sync_gitignore_section tests (F-016 dedup-merge) ──────────────────────

    #[test]
    fn test_sync_gitignore_creates_new_file_with_block() {
        let tmp = TempDir::new().unwrap();
        let changed = sync_gitignore_section("# Section\n*.log\n.env", tmp.path()).unwrap();
        assert!(changed);
        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains("solo-dev-hub:begin"));
        assert!(content.contains("solo-dev-hub:end"));
        assert!(content.contains("*.log"));
        assert!(content.contains(".env"));
        assert!(!content.contains("# Section"), "comments from template NOT carried into block");
    }

    #[test]
    fn test_sync_gitignore_dedup_filters_duplicates() {
        let tmp = TempDir::new().unwrap();
        let user = "# Секреты\n.env\n\n# AI\nCLAUDE.*\n.claude/\n*.exe\n";
        fs::write(tmp.path().join(".gitignore"), user).unwrap();

        // Template has .env (dup), CLAUDE.* (dup), .claude/ (dup), new ones below
        let template = "# Секреты\n.env\n\n# AI\nCLAUDE.*\n.claude/\n\n# Docs\ndocs/todo.md\ndocs/done.md";
        let changed = sync_gitignore_section(template, tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.starts_with("# Секреты"), "user content preserved at top");
        assert!(content.contains("*.exe"), "user rules preserved");
        // .env appears ONCE (user's), not duplicated in block
        assert_eq!(content.matches("\n.env\n").count() + content.matches("\n.env$").count(), 1);
        // Block contains only NEW rules (docs/todo.md, docs/done.md)
        let block_start = content.find("solo-dev-hub:begin").unwrap();
        let block_end = content.find("solo-dev-hub:end").unwrap();
        let block = &content[block_start..block_end];
        assert!(block.contains("docs/todo.md"), "new rules in block");
        assert!(block.contains("docs/done.md"), "new rules in block");
        assert!(!block.contains(".env"), "duplicate NOT in block");
        assert!(!block.contains("CLAUDE.*"), "duplicate NOT in block");
        assert!(!block.contains(".claude/"), "duplicate NOT in block");
    }

    #[test]
    fn test_sync_gitignore_removes_block_when_all_duplicates() {
        let tmp = TempDir::new().unwrap();
        let user = ".env\n*.log\ndocs/todo.md\n";
        fs::write(tmp.path().join(".gitignore"), user).unwrap();

        // All template rules already in user — block shouldn't be created
        let changed = sync_gitignore_section(".env\n*.log\ndocs/todo.md", tmp.path()).unwrap();
        assert!(!changed, "no changes needed — all duplicates");

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(!content.contains("solo-dev-hub"), "no block created when all dup");
    }

    #[test]
    fn test_sync_gitignore_rebuilds_block_on_update() {
        let tmp = TempDir::new().unwrap();
        // Initial: user has .env; block has docs/todo.md (now stale)
        let initial = ".env\n\n# --- solo-dev-hub:begin ---\nold-rule\n# --- solo-dev-hub:end ---\n";
        fs::write(tmp.path().join(".gitignore"), initial).unwrap();

        // New template: .env (dup with user), new-rule (new), *.bak (new)
        let changed = sync_gitignore_section(".env\nnew-rule\n*.bak", tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains("new-rule"));
        assert!(content.contains("*.bak"));
        assert!(!content.contains("old-rule"), "old block content replaced");
        let block_start = content.find("solo-dev-hub:begin").unwrap();
        let block_end = content.find("solo-dev-hub:end").unwrap();
        let block = &content[block_start..block_end];
        assert!(!block.contains(".env"), ".env not duplicated in new block");
    }

    #[test]
    fn test_sync_gitignore_idempotent() {
        let tmp = TempDir::new().unwrap();
        let template = ".env\n*.log";
        sync_gitignore_section(template, tmp.path()).unwrap();
        let before = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();

        let changed2 = sync_gitignore_section(template, tmp.path()).unwrap();
        assert!(!changed2, "second call = no change");
        let after = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn test_sync_gitignore_self_heals_orphan_begin() {
        let tmp = TempDir::new().unwrap();
        // File has only begin marker — orphan
        let broken = ".env\n# --- solo-dev-hub:begin ---\nsome-rule\n";
        fs::write(tmp.path().join(".gitignore"), broken).unwrap();

        let changed = sync_gitignore_section("new-rule", tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains(".env"), "user content preserved");
        assert!(content.contains("solo-dev-hub:begin"), "new valid block created");
        assert!(content.contains("solo-dev-hub:end"), "new valid block created");
        assert!(content.contains("new-rule"), "template rule added");
        // Old orphan line was stripped
        assert_eq!(content.matches("solo-dev-hub:begin").count(), 1, "only one begin marker");
    }

    #[test]
    fn test_sync_gitignore_self_heals_orphan_end() {
        let tmp = TempDir::new().unwrap();
        let broken = ".env\nstuff\n# --- solo-dev-hub:end ---\n";
        fs::write(tmp.path().join(".gitignore"), broken).unwrap();

        let changed = sync_gitignore_section("new-rule", tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains(".env"));
        assert!(content.contains("new-rule"));
        assert_eq!(content.matches("solo-dev-hub:end").count(), 1);
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
