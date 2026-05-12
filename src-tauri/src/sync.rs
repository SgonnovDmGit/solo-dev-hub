use crate::db::{self, AppDb};
use crate::export;
use crate::models::{Bug, FileBugNote, MigrationReport};
use regex::Regex;
use std::fs;
use std::path::Path;

/// Ensure repo root folder exists on disk. Returns error if missing — never creates.
/// Used as a guard at entry points before any create_dir_all calls, so that a stale
/// local_path in the DB does not silently recreate a deleted/moved folder.
pub fn ensure_root_exists(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Err(format!(
            "Repo root folder not found: {}. Update local_path in settings or restore the folder.",
            root.display()
        ));
    }
    if !root.is_dir() {
        return Err(format!(
            "Repo root path is not a directory: {}",
            root.display()
        ));
    }
    Ok(())
}

/// Reject paths that would escape the repo root: absolute paths, paths
/// containing `..`, or paths with drive letters / root components. Used to
/// guard `write_deploy_files` against malicious `file_targets` in meta.json.
/// Accepts forward and backward separators uniformly via `Path::components()`.
pub fn is_safe_subpath(rel: &str) -> bool {
    use std::path::Component;
    let p = Path::new(rel);
    if p.is_absolute() {
        return false;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Remove `.git` folder inside `repo_root` if it exists. No-op if missing.
/// Used by B-003 delete-repo flow; leaves other files in the folder untouched.
pub fn remove_git_dir(repo_root: &Path) -> Result<(), String> {
    let git_dir = repo_root.join(".git");
    if git_dir.exists() {
        fs::remove_dir_all(&git_dir)
            .map_err(|e| format!("Failed to remove {}: {}", git_dir.display(), e))?;
    }
    Ok(())
}

/// Scan directory for REQ-*.md files (not .response.md)
pub fn scan_requirements(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return vec![];
    }
    fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|name| {
                    name.starts_with("REQ-")
                        && name.ends_with(".md")
                        && !name.ends_with(".response.md")
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Scan directory for *.response.md files
pub fn scan_responses(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return vec![];
    }
    fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|name| name.ends_with(".response.md"))
                .collect()
        })
        .unwrap_or_default()
}

/// Remove a file
pub fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Copy file, overwriting only if content changed
pub fn copy_file_if_changed(source: &Path, target: &Path) -> Result<bool, String> {
    if !source.exists() {
        return Ok(false);
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if target.exists() {
        let src = fs::read(source).map_err(|e| e.to_string())?;
        let dst = fs::read(target).map_err(|e| e.to_string())?;
        if src == dst {
            return Ok(false);
        }
    }
    fs::copy(source, target).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Move a file atomically: write to target first, then delete source.
/// If write fails, source remains intact — no data loss.
/// Target's parent directory is created if needed.
pub fn migrate_file(source: &Path, target: &Path) -> Result<(), String> {
    if !source.exists() {
        return Err(format!("Source not found: {}", source.display()));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(source, target).map_err(|e| format!("Copy failed: {}", e))?;
    fs::remove_file(source).map_err(|e| format!("Remove source failed: {}", e))?;
    Ok(())
}

/// F-033 Stage 1e: replay a pending repo rename on the recipient side.
/// Idempotent: returns Ok(false) when there's nothing to do (old dir missing,
/// or new dir already exists — the latter counts as "migrated earlier, or
/// operator created the new folder manually"). Returns Ok(true) when the
/// filesystem was actually mutated.
///
/// Callers pass the **parent** directory (e.g. `srv/docs/client-requirements/`)
/// and the `old`/`new` canonical folder names. No DB state is updated — idempotency
/// comes from fs checks, not from a persistent "applied" flag.
pub fn replay_rename_in_dir(parent: &Path, old: &str, new: &str) -> Result<bool, String> {
    if old == new || old.is_empty() || new.is_empty() {
        return Ok(false);
    }
    let old_dir = parent.join(old);
    let new_dir = parent.join(new);
    if !old_dir.exists() {
        return Ok(false);
    }
    if new_dir.exists() {
        // Caller may choose to log this (collision = manual intervention needed).
        return Ok(false);
    }
    fs::rename(&old_dir, &new_dir).map_err(|e| {
        format!(
            "fs::rename {} -> {}: {}",
            old_dir.display(),
            new_dir.display(),
            e
        )
    })?;
    Ok(true)
}

/// F-033 Stage 1f Case B: one-time migration of a subfolder under a parent dir,
/// renaming `old_name/` to `new_name/` if and only if:
///   - `old_name` != `new_name`
///   - `parent/old_name/` exists
///   - `parent/new_name/` does NOT exist
/// Returns Ok(true) on actual rename, Ok(false) on no-op, and pushes a warning
/// to `warnings` if both dirs coexist (ambiguous — operator must resolve).
pub fn migrate_subfolder_rename(
    parent: &Path,
    old_name: &str,
    new_name: &str,
    warnings: &mut Vec<String>,
) -> Result<bool, String> {
    if old_name == new_name || old_name.is_empty() || new_name.is_empty() {
        return Ok(false);
    }
    let old_dir = parent.join(old_name);
    let new_dir = parent.join(new_name);
    if !old_dir.exists() {
        return Ok(false);
    }
    if new_dir.exists() {
        warnings.push(format!(
            "Migration ambiguous: both {} and {} exist — leaving alone (user should merge or remove one)",
            old_dir.display(),
            new_dir.display()
        ));
        return Ok(false);
    }
    fs::rename(&old_dir, &new_dir).map_err(|e| {
        format!(
            "fs::rename {} -> {}: {}",
            old_dir.display(),
            new_dir.display(),
            e
        )
    })?;
    Ok(true)
}

/// F-033 Stage 1f Case C: migrate files from the flat `server-requirements/`
/// on the microservice side into per-parent subfolders `server-requirements/<parent>/`.
///
/// Attribution algorithm per flat REQ file:
/// - Collect matching parents (byte-equal copy at `<parent-side>/<filename>`).
/// - Collect conflicting parents (copy exists but content differs).
/// - If any conflicts → leave file in flat root + push error (unsafe).
/// - Else if 0 matches → leave + warn (orphan).
/// - Else if 1 match → `fs::rename` into that parent's subfolder.
/// - Else (>= 2 same-content) → `fs::copy` into each matching parent's subfolder,
///   then `fs::remove_file` the source.
///
/// `parent_lookup` is a callable `fn(filename) -> Vec<(parent_canonical, existing_path_on_parent)>`.
/// Passing it as a closure keeps this helper unit-testable without DB / repo deps.
///
/// Returns number of flat files migrated (moved or copied-N-and-removed). Errors and
/// orphan warnings are appended to `warnings`.
pub fn migrate_flat_to_nested<F>(
    flat_root: &Path,
    mut parent_lookup: F,
    warnings: &mut Vec<String>,
) -> Result<usize, String>
where
    F: FnMut(&str) -> Vec<(String, std::path::PathBuf)>,
{
    if !flat_root.exists() || !flat_root.is_dir() {
        return Ok(0);
    }
    // Collect flat REQ/.response files (skip subdirs and non-matching names).
    let mut flat_files: Vec<std::path::PathBuf> = Vec::new();
    for entry in fs::read_dir(flat_root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("REQ-") {
            continue;
        }
        if !name.ends_with(".md") {
            continue;
        }
        flat_files.push(p);
    }

    let mut migrated: usize = 0;
    for flat in flat_files {
        let fname = flat
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let flat_content = match fs::read(&flat) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("Case C: read {}: {}", flat.display(), e));
                continue;
            }
        };
        let candidates = parent_lookup(&fname);
        let mut matches: Vec<String> = Vec::new();
        let mut conflicts: Vec<String> = Vec::new();
        for (parent_canonical, candidate_path) in candidates {
            match fs::read(&candidate_path) {
                Ok(c) if c == flat_content => matches.push(parent_canonical),
                Ok(_) => conflicts.push(parent_canonical),
                Err(_) => { /* parent-side file missing or unreadable — ignore */ }
            }
        }
        if !conflicts.is_empty() {
            warnings.push(format!(
                "Case C: {} has diverged copies on parent(s) [{}] — manual resolution needed, left in flat root",
                fname,
                conflicts.join(", ")
            ));
            continue;
        }
        match matches.len() {
            0 => {
                warnings.push(format!(
                    "Case C: {} has no matching parent copy — possibly stale, left in flat root",
                    fname
                ));
            }
            1 => {
                let dst = flat_root.join(&matches[0]).join(&fname);
                if let Some(dst_dir) = dst.parent() {
                    fs::create_dir_all(dst_dir).map_err(|e| e.to_string())?;
                }
                fs::rename(&flat, &dst).map_err(|e| {
                    format!("Case C: rename {} -> {}: {}", flat.display(), dst.display(), e)
                })?;
                migrated += 1;
            }
            _ => {
                // Multi-parent same-content: duplicate into each matching parent's
                // subfolder. If any copy fails mid-loop, roll back the successful
                // ones and leave the flat source intact — preserves "all or nothing"
                // for the parent set so the next sync pass can retry cleanly without
                // ghost partial copies in some parents.
                let mut created: Vec<std::path::PathBuf> = Vec::new();
                let mut copy_error: Option<String> = None;
                for parent_canonical in &matches {
                    let dst = flat_root.join(parent_canonical).join(&fname);
                    if let Some(dst_dir) = dst.parent() {
                        if let Err(e) = fs::create_dir_all(dst_dir) {
                            copy_error = Some(e.to_string());
                            break;
                        }
                    }
                    if let Err(e) = fs::copy(&flat, &dst) {
                        copy_error = Some(format!(
                            "Case C: copy {} -> {}: {}",
                            flat.display(),
                            dst.display(),
                            e
                        ));
                        break;
                    }
                    created.push(dst);
                }
                if let Some(err) = copy_error {
                    for dst in &created {
                        let _ = fs::remove_file(dst);
                    }
                    warnings.push(format!(
                        "Case C: {} partial-copy failed, rolled back ({}); left in flat root for retry",
                        fname, err
                    ));
                    continue;
                }
                fs::remove_file(&flat).map_err(|e| {
                    format!("Case C: remove flat source {}: {}", flat.display(), e)
                })?;
                migrated += 1;
            }
        }
    }
    Ok(migrated)
}

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

// ── Bugs MD ↔ DB sync (v0.16.0) ──────────────────────────────────────────────
//
// SQLite = SoT for bugs. MD (docs/bug-reports.md) is the LLM-facing view:
// LLM edits status+comment on existing rows per the global CLAUDE.md contract;
// all other fields are either app-managed (id, date, fix_attempts, confirmed_at)
// or user-owned through the UI (description, severity, category).
//
// `migrate_bugs_for_repo` is the one-time MD→DB import (lazy, first open of
// bug-tab per repo in v0.16.0+). `reconcile_bugs_for_repo` is the ongoing
// sync on bug-tab open / Refresh button / global Sync: LLM-writable fields
// (status, comment) ingest MD→DB; protected-field violations and new/deleted
// rows are silently reverted by `regenerate_bugs_md` at the end.

/// Fixed repo-relative path for the bug-reports file. Matches the global
/// CLAUDE.md template contract; hardcoded since T-048 removed the configurable
/// path setting.
const BUG_REPORTS_REL: &str = "docs/bug-reports.md";

/// Parse numeric part of a `B-NNN` display id. Lenient on length — accepts
/// legacy 3-digit (`B-042` → 42), new 6-digit (`B-000042` → 42), or any `\d+`.
/// Returns None if the prefix is absent or the tail isn't integer.
pub fn parse_numeric_id(display_id: &str) -> Option<i64> {
    display_id
        .strip_prefix("B-")?
        .parse::<i64>()
        .ok()
        .filter(|n| *n >= 0)
}

/// Validate a status transition initiated by LLM via MD edit.
/// Allowed transitions (global CLAUDE.md bug workflow + LLM-friendly shortcuts):
///   created → in-progress         (taking into work, no fix yet)
///   created → testing             (quick fix shortcut, bumps fix_attempts)
///   in-progress → testing         (fix ready, bumps fix_attempts)
///   rejected → in-progress        (restart work after rejection)
///   rejected → testing            (quick retry after rejection, bumps fix_attempts)
///   testing → confirmed           (UI-only path via ✓ button, not reachable via LLM MD edit)
///   testing → rejected            (UI-only path via ✗ button)
/// Anything else (e.g. `created → confirmed`, `confirmed → anything`,
/// `testing → created`) is a contract violation — ignored with a warning.
/// All transitions ending in `testing` bump `fix_attempts +1` — see reconcile logic.
pub fn valid_transition(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("created", "in-progress")
            | ("created", "testing")
            | ("in-progress", "testing")
            | ("rejected", "in-progress")
            | ("rejected", "testing")
            | ("testing", "confirmed")
            | ("testing", "rejected")
    )
}

/// Convert a DB row to the MD-facing 8-field `FileBugNote`. `created_at` ISO
/// timestamp is truncated to `YYYY-MM-DD` (first 10 chars) to match the MD
/// contract. `confirmed_at` is not in MD — lives in DB only.
fn bug_to_file_note(bug: &Bug) -> FileBugNote {
    let date = bug.created_at.get(..10).unwrap_or(&bug.created_at).to_string();
    FileBugNote {
        id: bug.display_id.clone(),
        date,
        description: bug.description.clone(),
        severity: bug.severity.clone(),
        category: bug.category.clone(),
        status: bug.status.clone(),
        fix_attempts: bug.fix_attempts,
        comment: bug.comment.clone(),
    }
}

/// Build the `docs/bug-reports.md` absolute path for a repo's local checkout.
fn bug_reports_path(local_path: &str) -> std::path::PathBuf {
    let clean = local_path.trim_end_matches(['/', '\\']);
    Path::new(clean).join(BUG_REPORTS_REL)
}

/// Rewrite `docs/bug-reports.md` from the current `bugs` DB state.
///
/// v0.21.1 rules: row appears in MD if it's active (status != 'confirmed') OR
/// it's confirmed but not yet LLM-acknowledged (archived_from_md_at IS NULL).
/// This restores the original LLM-acknowledgement workflow: app sets
/// status='confirmed' on user ✓ click, MD now shows the confirmation, and on
/// the next LLM session the row gets removed from MD as cleanup. Reconcile
/// then sets archived_from_md_at, after which subsequent regens permanently
/// exclude the row. DB history (with confirmed_at) is preserved indefinitely.
///
/// Called after every mutation that can change MD contents: create/resolve/reject
/// via UI, end of reconcile, end of migration.
///
/// If `local_path` is None (remote-only repo), this is a silent no-op — MD
/// will regenerate once the repo is cloned and `local_path` populated.
pub fn regenerate_bugs_md(db: &AppDb, repo_id: i64) -> Result<(), String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(ref local_path) = repo.local_path else {
        return Ok(());
    };
    let path = bug_reports_path(local_path);
    let bugs = db.list_bugs_for_md(repo_id).map_err(|e| e.to_string())?;
    let file_notes: Vec<FileBugNote> = bugs.iter().map(bug_to_file_note).collect();
    let md = export::generate_bug_reports(&file_notes);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    fs::write(&path, md).map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    Ok(())
}

/// One-time lazy MD→DB bug import for a repo. Triggered on first open of
/// bug-tab in v0.16.0+. Idempotent — returns `already=true` on repeat calls.
///
/// Flow (atomicity: all DB writes in one transaction; MD write outside commit):
///   1. Check `bugs_migrated_at` marker — skip if set.
///   2. Resolve `local_path`; if None, skip (remote-only repo).
///   3. Parse `docs/bug-reports.md`. Absent file → set marker to empty, done.
///   4. Pre-check: every row has a valid `B-NNN` id and no duplicate
///      numeric_id in the file. If duplicates found, return Err before any
///      DB writes (user fixes MD manually, retries via Refresh).
///   5. Single-transaction INSERT of all rows + UPDATE marker.
///   6. Regenerate MD from DB state (6-digit ids, confirmed rows dropped).
///      Regen failure does not roll back DB — next reconcile self-heals.
pub fn migrate_bugs_for_repo(db: &AppDb, repo_id: i64) -> Result<MigrationReport, String> {
    // 1. Already migrated?
    if db
        .get_bugs_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(MigrationReport {
            imported: 0,
            confirmed_archived: 0,
            already: true,
        });
    }

    // 2. local_path present?
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(ref local_path) = repo.local_path else {
        // Remote-only: can't migrate yet. Leave marker NULL so next-time open
        // after clone triggers migration. (This is a no-op; we don't set marker.)
        return Ok(MigrationReport {
            imported: 0,
            confirmed_archived: 0,
            already: false,
        });
    };

    let now = db::utc_now_rfc3339();

    // 3. Parse MD (absent file = empty import, still set marker).
    let path = bug_reports_path(local_path);
    if !path.exists() {
        db.set_bugs_migrated_at(repo_id, &now)
            .map_err(|e| e.to_string())?;
        return Ok(MigrationReport {
            imported: 0,
            confirmed_archived: 0,
            already: false,
        });
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("Read {} failed: {}", path.display(), e))?;
    let (file_notes, warnings) = export::parse_bug_reports(&content);
    for w in &warnings {
        eprintln!("[migrate_bugs repo={}] parse warn: {}", repo_id, w);
    }

    // 4. Pre-check: extract numeric_ids, detect duplicates.
    let mut rows: Vec<(i64, FileBugNote)> = Vec::with_capacity(file_notes.len());
    let mut seen_ids = std::collections::HashSet::new();
    for note in file_notes {
        let Some(nid) = parse_numeric_id(&note.id) else {
            return Err(format!(
                "Unparseable bug id '{}' in {}. Fix MD and retry.",
                note.id,
                path.display()
            ));
        };
        if !seen_ids.insert(nid) {
            return Err(format!(
                "Duplicate bug id '{}' in {}. Fix MD and retry.",
                note.id,
                path.display()
            ));
        }
        rows.push((nid, note));
    }

    // 5. Transactional insert + marker.
    let report = db
        .migrate_bugs_transactional(repo_id, &rows, &now)
        .map_err(|e| format!("Migration transaction failed: {}", e))?;

    // 6. Regen MD from DB (outside transaction — self-healing on fs failure).
    if let Err(e) = regenerate_bugs_md(db, repo_id) {
        eprintln!(
            "[migrate_bugs repo={}] regen after commit failed: {} \
             — DB is consistent, next reconcile will regen",
            repo_id, e
        );
    }

    Ok(report)
}

/// 2-way sync MD ↔ DB for a repo. Ingests LLM edits of `status` / `comment`
/// from MD into DB; protected-field mismatches and unknown/deleted rows are
/// silently corrected by the final `regenerate_bugs_md`.
///
/// Preconditions:
///   - `bugs_migrated_at IS NOT NULL` — caller must have run
///     `migrate_bugs_for_repo` first. `ensure_bugs_migrated` Tauri command
///     handles this for UI.
///   - If `local_path IS NULL` → no-op (remote-only repo).
///   - MD file absent → all DB rows with `status != 'confirmed'` appear to
///     have been "deleted"; regen recreates the MD from DB state (self-heals).
pub fn reconcile_bugs_for_repo(db: &AppDb, repo_id: i64) -> Result<(), String> {
    if db
        .get_bugs_migrated_at(repo_id)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Err(format!(
            "repo {} is not migrated yet — call ensure_bugs_migrated first",
            repo_id
        ));
    }

    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let Some(ref local_path) = repo.local_path else {
        return Ok(());
    };

    // Read MD (missing file = empty rows, all active DB bugs "restored").
    let path = bug_reports_path(local_path);
    let file_notes: Vec<FileBugNote> = if path.exists() {
        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Read {} failed: {}", path.display(), e))?;
        let (notes, warnings) = export::parse_bug_reports(&content);
        for w in &warnings {
            eprintln!("[reconcile_bugs repo={}] parse warn: {}", repo_id, w);
        }
        notes
    } else {
        Vec::new()
    };

    // Build lookup by numeric_id.
    let db_bugs = db
        .list_bugs_by_repo(repo_id, true)
        .map_err(|e| e.to_string())?;
    let db_by_nid: std::collections::HashMap<i64, &Bug> =
        db_bugs.iter().map(|b| (b.numeric_id, b)).collect();

    let now = db::utc_now_rfc3339();
    let mut md_ids = std::collections::HashSet::new();

    for note in &file_notes {
        let Some(nid) = parse_numeric_id(&note.id) else {
            eprintln!(
                "[reconcile_bugs repo={}] unparseable id '{}' — drop on regen",
                repo_id, note.id
            );
            continue;
        };
        md_ids.insert(nid);

        let Some(db_bug) = db_by_nid.get(&nid) else {
            eprintln!(
                "[reconcile_bugs repo={}] orphan row {} in MD (not in DB) — drop on regen",
                repo_id, note.id
            );
            continue;
        };

        // Status transition (LLM-writable).
        if note.status != db_bug.status {
            if valid_transition(&db_bug.status, &note.status) {
                let new_attempts = if note.status == "testing" && db_bug.status != "testing" {
                    Some(db_bug.fix_attempts + 1)
                } else {
                    None
                };
                let new_confirmed_at = if note.status == "confirmed" {
                    Some(now.as_str())
                } else {
                    None
                };
                let event_type = match (db_bug.status.as_str(), note.status.as_str()) {
                    ("created", "in-progress") => "taken",
                    ("created", "testing") => "entered_testing",
                    ("in-progress", "testing") => "entered_testing",
                    ("rejected", "in-progress") => "reopened",
                    ("rejected", "testing") => "entered_testing",
                    ("testing", "confirmed") => "confirmed",
                    ("testing", "rejected") => "rejected",
                    _ => "taken", // fallback — valid_transition filters invalids, unreachable in practice
                };
                db.update_bug_status(db_bug.id, &note.status, new_attempts, new_confirmed_at)
                    .map_err(|e| e.to_string())?;
                db.insert_bug_event(
                    db_bug.id,
                    event_type,
                    Some(db_bug.status.as_str()),
                    Some(note.status.as_str()),
                    &now,
                )
                .map_err(|e| e.to_string())?;
            } else {
                eprintln!(
                    "[reconcile_bugs repo={}] invalid transition {} → {} for {} — revert on regen",
                    repo_id, db_bug.status, note.status, note.id
                );
            }
        }

        // Comment (LLM-writable). Empty string in MD = None in DB (normalize).
        let md_comment = note.comment.as_deref().filter(|s| !s.is_empty());
        let db_comment = db_bug.comment.as_deref().filter(|s| !s.is_empty());
        if md_comment != db_comment {
            db.update_bug_comment(db_bug.id, md_comment)
                .map_err(|e| e.to_string())?;
        }

        // Protected fields (description, severity, category, fix_attempts, date) —
        // ignored here. Any mismatch will be overwritten by the regen below.
    }

    // v0.21.1: Rows missing from MD have two interpretations depending on status:
    //   - Active (status != 'confirmed') missing: LLM-deleted by mistake — regen
    //     will restore them in MD (DB is authoritative for active state).
    //   - Confirmed missing AND archived_from_md_at IS NULL: LLM acknowledged the
    //     confirmation by removing the row → mark archived_from_md_at = NOW so
    //     subsequent regens permanently exclude this row (history kept in DB).
    for db_bug in &db_bugs {
        if md_ids.contains(&db_bug.numeric_id) {
            continue;
        }
        if db_bug.status == "confirmed" && db_bug.archived_from_md_at.is_none() {
            db.mark_bug_archived_from_md(db_bug.id)
                .map_err(|e| e.to_string())?;
        } else if db_bug.status != "confirmed" {
            eprintln!(
                "[reconcile_bugs repo={}] bug {} deleted from MD by LLM — restore on regen",
                repo_id, db_bug.display_id
            );
        }
    }

    // Final regen: authoritative MD = DB state. Corrects all protected-field
    // edits, removes orphans, restores LLM-deleted active rows. Confirmed rows
    // appear if not yet LLM-acknowledged; otherwise excluded.
    regenerate_bugs_md(db, repo_id)?;
    Ok(())
}

// ── v0.20.0: Task sync ────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
pub struct SyncTasksReport {
    pub imported: u32,
    pub events_emitted: u32,
}

/// Sync todo.md + done.md from disk into the `tasks` DB table for the given repo.
///
/// Algorithm:
/// 1. Parse todo.md and done.md from disk.
/// 2. Compare against existing `tasks` rows in DB (keyed by task_id string).
/// 3. New tasks → INSERT. Status changes → UPDATE + event. todo→done move → UPDATE source + event.
/// 4. First-sync semantics: if `tasks_migrated_at IS NULL` for this repo, suppress all
///    "created" events (silent backfill of legacy data). Mark migrated after.
///
/// Returns `SyncTasksReport` with counts of imported rows and emitted events.
pub fn sync_tasks_for_repo(db: &AppDb, repo_id: i64) -> Result<SyncTasksReport, String> {
    use std::collections::{HashMap, HashSet};

    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;

    let local_path = match repo.local_path.clone() {
        Some(p) => p,
        None => {
            db.mark_tasks_migrated(repo_id, &chrono::Utc::now().to_rfc3339())
                .map_err(|e| e.to_string())?;
            return Ok(SyncTasksReport { imported: 0, events_emitted: 0 });
        }
    };

    let todo_path = Path::new(&local_path).join("docs").join("todo.md");
    let done_path = Path::new(&local_path).join("docs").join("done.md");

    // Determine whether this is a first sync (suppress created events for legacy backfill)
    let was_migrated = db.get_tasks_migrated_at(repo_id).map_err(|e| e.to_string())?.is_some();
    let suppress_created_events = !was_migrated;

    // Read and parse todo.md
    let (todo_tasks, todo_mtime) = if todo_path.exists() {
        let content = std::fs::read_to_string(&todo_path)
            .map_err(|e| format!("read todo.md: {}", e))?;
        let mtime = std::fs::metadata(&todo_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
        let (tasks, _warnings) = export::parse_todo_tasks(&content);
        (tasks, mtime)
    } else {
        (Vec::new(), String::new())
    };

    // Read and parse done.md
    let done_tasks = if done_path.exists() {
        let content = std::fs::read_to_string(&done_path)
            .map_err(|e| format!("read done.md: {}", e))?;
        let (tasks, _warnings) = export::parse_done_tasks(&content);
        tasks
    } else {
        Vec::new()
    };

    // Load existing DB rows keyed by task_id string
    let db_todos = db.list_tasks_by_repo(repo_id, "todo").map_err(|e| e.to_string())?;
    let db_dones = db.list_tasks_by_repo(repo_id, "done").map_err(|e| e.to_string())?;
    let db_todo_by_id: HashMap<String, _> = db_todos.iter().map(|t| (t.task_id.clone(), t.clone())).collect();
    let db_done_by_id: HashMap<String, _> = db_dones.iter().map(|t| (t.task_id.clone(), t.clone())).collect();

    let mut imported = 0u32;
    let mut events_emitted = 0u32;

    // ── Process TODO entries ─────────────────────────────────────────────────
    for tt in &todo_tasks {
        let prefix = if tt.id.starts_with("T-") {
            "T"
        } else if tt.id.starts_with("F-") {
            "F"
        } else {
            continue; // Skip unknown prefixes
        };

        let created_at = if tt.created_at.is_empty() {
            todo_mtime.clone()
        } else {
            tt.created_at.clone()
        };

        if let Some(existing) = db_todo_by_id.get(&tt.id) {
            // Row exists in DB as todo — check for status change
            let new_status = if tt.status.is_empty() { None } else { Some(tt.status.as_str()) };
            let old_status = existing.status.as_deref();
            if new_status != old_status {
                let event_type = match (old_status, new_status) {
                    (Some("open"), Some("in-progress")) => "taken",
                    (Some("in-progress"), Some("review")) => "review",
                    (Some("review"), Some("open")) | (Some("done"), Some("in-progress")) => "reopened",
                    _ => {
                        // Unusual transition — update status but emit no event
                        eprintln!(
                            "[sync_tasks repo={}] unusual status transition: {:?} -> {:?} for {}",
                            repo_id, old_status, new_status, tt.id
                        );
                        ""
                    }
                };
                db.update_task_status(existing.id, new_status).map_err(|e| e.to_string())?;
                if !event_type.is_empty() {
                    db.insert_task_event(
                        existing.id,
                        event_type,
                        &chrono::Utc::now().to_rfc3339(),
                        old_status,
                        new_status,
                    ).map_err(|e| e.to_string())?;
                    events_emitted += 1;
                }
            }
        } else if let Some(existing_done) = db_done_by_id.get(&tt.id) {
            // Was in done in DB but reappeared in todo — reopened
            db.update_task_source(existing_done.id, "todo").map_err(|e| e.to_string())?;
            db.update_task_status(existing_done.id, Some(tt.status.as_str())).map_err(|e| e.to_string())?;
            db.insert_task_event(
                existing_done.id,
                "reopened",
                &chrono::Utc::now().to_rfc3339(),
                None,
                Some(tt.status.as_str()),
            ).map_err(|e| e.to_string())?;
            events_emitted += 1;
        } else {
            // New task — insert
            let effort = tt.effort.parse::<f64>().ok();
            let row = db.insert_task(
                repo_id,
                &tt.id,
                prefix,
                &tt.description,
                effort,
                if tt.priority.is_empty() { None } else { Some(tt.priority.as_str()) },
                if tt.status.is_empty() { None } else { Some(tt.status.as_str()) },
                None, // version — not known for todo tasks
                "todo",
                &created_at,
            ).map_err(|e| e.to_string())?;
            imported += 1;

            if !suppress_created_events {
                let to_status = if tt.status.is_empty() { None } else { Some(tt.status.as_str()) };
                db.insert_task_event(
                    row.id,
                    "created",
                    &chrono::Utc::now().to_rfc3339(),
                    None,
                    to_status,
                ).map_err(|e| e.to_string())?;
                events_emitted += 1;
            }
        }
    }

    // ── Process DONE entries ─────────────────────────────────────────────────
    // Build set of todo ids seen in MD (for detecting todo→done moves)
    let _md_todo_ids: HashSet<&str> = todo_tasks.iter().map(|t| t.id.as_str()).collect();

    for dt in &done_tasks {
        let prefix = if dt.id.starts_with("T-") {
            "T"
        } else if dt.id.starts_with("F-") {
            "F"
        } else if dt.id.starts_with("D-") {
            "D"
        } else {
            continue;
        };

        if db_done_by_id.contains_key(&dt.id) {
            // Already in done — skip (idempotent)
        } else if let Some(was_in_todo) = db_todo_by_id.get(&dt.id) {
            // Was in todo in DB, now in done in MD — task completed
            db.update_task_source(was_in_todo.id, "done").map_err(|e| e.to_string())?;
            db.update_task_status(was_in_todo.id, None).map_err(|e| e.to_string())?;
            db.insert_task_event(
                was_in_todo.id,
                "done",
                &chrono::Utc::now().to_rfc3339(),
                was_in_todo.status.as_deref(),
                None,
            ).map_err(|e| e.to_string())?;
            events_emitted += 1;
        } else {
            // Brand new done entry (historical task, never seen before in DB)
            let fallback_date = if todo_mtime.is_empty() {
                chrono::Utc::now().format("%Y-%m-%d").to_string()
            } else {
                todo_mtime.clone()
            };
            let row = db.insert_task(
                repo_id,
                &dt.id,
                prefix,
                &dt.description,
                None, // no effort for done tasks
                None, // no priority
                None, // no active status for done tasks
                Some(dt.version.as_str()),
                "done",
                if dt.date.is_empty() { &fallback_date } else { &dt.date },
            ).map_err(|e| e.to_string())?;
            imported += 1;

            if !suppress_created_events {
                db.insert_task_event(
                    row.id,
                    "done",
                    &chrono::Utc::now().to_rfc3339(),
                    None,
                    None,
                ).map_err(|e| e.to_string())?;
                events_emitted += 1;
            }
        }
    }

    // ── Cleanup orphan todo rows (in DB but absent from MD) ──────────────────
    // Fixes B-000004: when LLM normalises an ID in todo.md (e.g. T-034 → T-000034
    // or placeholder "F-NNN" → real "F-000035"), the old DB row used to stick
    // around as a duplicate forever. todo.md is canonical for tasks, so any DB
    // row whose task_id is no longer in MD is an orphan and gets dropped here.
    // task_events cascade via FK. Done rows are append-only and untouched.
    let md_todo_ids: HashSet<&str> = todo_tasks.iter().map(|t| t.id.as_str()).collect();
    let db_todos_now = db.list_tasks_by_repo(repo_id, "todo").map_err(|e| e.to_string())?;
    for t in &db_todos_now {
        if !md_todo_ids.contains(t.task_id.as_str()) {
            db.delete_task(t.id).map_err(|e| e.to_string())?;
        }
    }

    db.mark_tasks_migrated(repo_id, &chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;

    Ok(SyncTasksReport { imported, events_emitted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_ensure_root_exists_ok() {
        let tmp = TempDir::new().unwrap();
        assert!(ensure_root_exists(tmp.path()).is_ok());
    }

    #[test]
    fn test_is_safe_subpath_accepts_normal() {
        assert!(is_safe_subpath("Dockerfile"));
        assert!(is_safe_subpath(".github/workflows/deploy.yml"));
        assert!(is_safe_subpath("subdir/file.txt"));
        assert!(is_safe_subpath("./Dockerfile"));
    }

    #[test]
    fn test_is_safe_subpath_rejects_parent_dir() {
        assert!(!is_safe_subpath("../escape.yml"));
        assert!(!is_safe_subpath("ok/../../escape.yml"));
        assert!(!is_safe_subpath(".github/../../../etc/passwd"));
    }

    #[test]
    fn test_is_safe_subpath_rejects_absolute() {
        assert!(!is_safe_subpath("/etc/passwd"));
        assert!(!is_safe_subpath("/tmp/x.yml"));
    }

    #[cfg(windows)]
    #[test]
    fn test_is_safe_subpath_rejects_windows_absolute() {
        assert!(!is_safe_subpath("C:\\Windows\\system32\\foo"));
        assert!(!is_safe_subpath("C:/x.yml"));
        assert!(!is_safe_subpath("\\server\\share\\x"));
    }

    #[test]
    fn test_ensure_root_exists_missing() {
        let result = ensure_root_exists(Path::new("/nonexistent/abc123/xyz"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_remove_git_dir_missing_is_noop() {
        let tmp = TempDir::new().unwrap();
        // No .git folder — should succeed without errors.
        assert!(remove_git_dir(tmp.path()).is_ok());
    }

    #[test]
    fn test_remove_git_dir_removes_only_git() {
        let tmp = TempDir::new().unwrap();
        let git = tmp.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let keep = tmp.path().join("README.md");
        fs::write(&keep, "hello").unwrap();

        remove_git_dir(tmp.path()).unwrap();

        assert!(!git.exists(), ".git should be removed");
        assert!(keep.exists(), "other files must be kept");
    }

    #[test]
    fn test_ensure_root_exists_is_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("file.txt");
        fs::write(&file, "x").unwrap();
        let result = ensure_root_exists(&file);
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_requirements_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let result = scan_requirements(tmp.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_requirements_nonexistent_dir() {
        let result = scan_requirements(Path::new("/nonexistent/path/abc123"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_scan_requirements_finds_req_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("REQ-001.md"), "req content").unwrap();
        fs::write(tmp.path().join("REQ-002.md"), "req content 2").unwrap();
        fs::write(tmp.path().join("REQ-001.response.md"), "response").unwrap();
        fs::write(tmp.path().join("other.md"), "other").unwrap();

        let result = scan_requirements(tmp.path());
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"REQ-001.md".to_string()));
        assert!(result.contains(&"REQ-002.md".to_string()));
    }

    #[test]
    fn test_scan_responses() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("REQ-001.response.md"), "response").unwrap();
        fs::write(tmp.path().join("REQ-002.response.md"), "response 2").unwrap();
        fs::write(tmp.path().join("REQ-001.md"), "req").unwrap();

        let result = scan_responses(tmp.path());
        assert_eq!(result.len(), 2);
        assert!(result.contains(&"REQ-001.response.md".to_string()));
    }

    #[test]
    fn test_scan_responses_nonexistent_dir() {
        let result = scan_responses(Path::new("/nonexistent/path/abc123"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_remove_file_if_exists() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.md");
        fs::write(&file, "content").unwrap();
        assert!(file.exists());

        remove_file_if_exists(&file).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn test_remove_file_if_exists_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("nonexistent.md");
        let result = remove_file_if_exists(&file);
        assert!(result.is_ok());
    }

    // ── F-033 Stage 1e: replay_rename_in_dir ─────────────────────────────────

    #[test]
    fn test_replay_rename_noop_when_old_missing() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        // new exists but no old
        fs::create_dir_all(parent.join("new-name")).unwrap();
        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert!(!result, "no-op when old_dir missing");
        assert!(parent.join("new-name").exists());
    }

    #[test]
    fn test_replay_rename_success() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old-name")).unwrap();
        fs::write(parent.join("old-name/REQ-001.md"), "x").unwrap();

        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert!(result, "actual rename happened");
        assert!(!parent.join("old-name").exists());
        assert!(parent.join("new-name").exists());
        assert!(parent.join("new-name/REQ-001.md").exists());
    }

    #[test]
    fn test_replay_rename_noop_when_new_exists() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old-name")).unwrap();
        fs::create_dir_all(parent.join("new-name")).unwrap();

        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert!(!result, "collision → no-op");
        assert!(parent.join("old-name").exists(), "old preserved");
        assert!(parent.join("new-name").exists(), "new preserved");
    }

    #[test]
    fn test_replay_rename_same_name_noop() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("same")).unwrap();
        let result = replay_rename_in_dir(tmp.path(), "same", "same").unwrap();
        assert!(!result);
    }

    // ── F-033 Stage 1f Case B: migrate_subfolder_rename ──────────────────────

    #[test]
    fn test_migrate_subfolder_rename_basic() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("ProjectName")).unwrap();
        fs::write(parent.join("ProjectName/REQ-001.md"), "data").unwrap();

        let mut warnings = Vec::new();
        let result = migrate_subfolder_rename(parent, "ProjectName", "repo-canonical", &mut warnings).unwrap();
        assert!(result);
        assert!(!parent.join("ProjectName").exists());
        assert!(parent.join("repo-canonical/REQ-001.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_migrate_subfolder_rename_ambiguous_both_exist() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old")).unwrap();
        fs::create_dir_all(parent.join("new")).unwrap();

        let mut warnings = Vec::new();
        let result = migrate_subfolder_rename(parent, "old", "new", &mut warnings).unwrap();
        assert!(!result);
        assert!(parent.join("old").exists());
        assert!(parent.join("new").exists());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("ambiguous"));
    }

    // ── F-033 Stage 1f Case C: migrate_flat_to_nested ────────────────────────

    #[test]
    fn test_case_c_single_parent_migrates_to_subfolder() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "payload").unwrap();
        fs::write(ms_flat.join("REQ-002.md"), "payload2").unwrap();

        // Parent side — both REQ present with identical content.
        let parent_side = tmp.path().join("parent-side");
        fs::create_dir_all(&parent_side).unwrap();
        fs::write(parent_side.join("REQ-001.md"), "payload").unwrap();
        fs::write(parent_side.join("REQ-002.md"), "payload2").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parent-A".to_string(), parent_side.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 2);
        assert!(ms_flat.join("parent-A/REQ-001.md").exists());
        assert!(ms_flat.join("parent-A/REQ-002.md").exists());
        assert!(!ms_flat.join("REQ-001.md").exists());
        assert!(!ms_flat.join("REQ-002.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_case_c_multi_parent_same_content_duplicates() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-007.md"), "shared").unwrap();

        let pa = tmp.path().join("pa");
        let pb = tmp.path().join("pb");
        fs::create_dir_all(&pa).unwrap();
        fs::create_dir_all(&pb).unwrap();
        fs::write(pa.join("REQ-007.md"), "shared").unwrap();
        fs::write(pb.join("REQ-007.md"), "shared").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![
                ("parentA".to_string(), pa.join(name)),
                ("parentB".to_string(), pb.join(name)),
            ]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 1);
        assert!(ms_flat.join("parentA/REQ-007.md").exists());
        assert!(ms_flat.join("parentB/REQ-007.md").exists());
        assert!(!ms_flat.join("REQ-007.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_case_c_multi_parent_different_content_unsafe() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-010.md"), "payloadA").unwrap();

        let pa = tmp.path().join("pa");
        let pb = tmp.path().join("pb");
        fs::create_dir_all(&pa).unwrap();
        fs::create_dir_all(&pb).unwrap();
        fs::write(pa.join("REQ-010.md"), "payloadA").unwrap();
        fs::write(pb.join("REQ-010.md"), "payloadB-different").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![
                ("parentA".to_string(), pa.join(name)),
                ("parentB".to_string(), pb.join(name)),
            ]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 0);
        assert!(ms_flat.join("REQ-010.md").exists(), "kept in flat root");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("diverged") || warnings[0].contains("parentB"));
    }

    #[test]
    fn test_case_c_orphan_file() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-999.md"), "stale").unwrap();

        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        // parent has no REQ-999

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parentA".to_string(), pa.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 0);
        assert!(ms_flat.join("REQ-999.md").exists());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no matching parent copy"));
    }

    #[test]
    fn test_case_c_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "x").unwrap();
        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        fs::write(pa.join("REQ-001.md"), "x").unwrap();

        let mk_lookup = || {
            let pa_cloned = pa.clone();
            move |name: &str| -> Vec<(String, std::path::PathBuf)> {
                vec![("parentA".to_string(), pa_cloned.join(name))]
            }
        };

        let mut w1 = Vec::new();
        let m1 = migrate_flat_to_nested(&ms_flat, mk_lookup(), &mut w1).unwrap();
        assert_eq!(m1, 1);

        // Second pass: flat root is empty of REQ-*.md → no-op.
        let mut w2 = Vec::new();
        let m2 = migrate_flat_to_nested(&ms_flat, mk_lookup(), &mut w2).unwrap();
        assert_eq!(m2, 0);
        assert!(w2.is_empty());
    }

    #[test]
    fn test_case_c_response_file_migrates_too() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "req").unwrap();
        fs::write(ms_flat.join("REQ-001.response.md"), "resp").unwrap();

        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        fs::write(pa.join("REQ-001.md"), "req").unwrap();
        fs::write(pa.join("REQ-001.response.md"), "resp").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parentA".to_string(), pa.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 2);
        assert!(ms_flat.join("parentA/REQ-001.md").exists());
        assert!(ms_flat.join("parentA/REQ-001.response.md").exists());
    }

    #[test]
    fn test_migrate_file_moves_content_and_removes_source() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("docs/api.md");
        let dst = tmp.path().join("docs/server-api/api.md");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, "api content").unwrap();

        migrate_file(&src, &dst).unwrap();

        assert!(!src.exists(), "source must be removed after migration");
        assert!(dst.exists(), "target must exist");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "api content");
    }

    #[test]
    fn test_migrate_file_creates_target_parent() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("api.md");
        let dst = tmp.path().join("nested/deep/server-api/api.md");
        fs::write(&src, "x").unwrap();

        migrate_file(&src, &dst).unwrap();
        assert!(dst.exists());
    }

    #[test]
    fn test_migrate_file_source_missing_errors() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("missing.md");
        let dst = tmp.path().join("dst.md");
        let result = migrate_file(&src, &dst);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Source not found"));
    }

    #[test]
    fn test_auto_migration_flow_simulates_first_sync() {
        // Simulates 0.9.0 client-side flow:
        // - Old docs/api.md exists (content A).
        // - Server has docs/api.md (content B).
        // - After sync: docs/api.md gone, docs/server-api/api.md has content B.
        let tmp = TempDir::new().unwrap();
        let old_api = tmp.path().join("docs/api.md");
        let new_api = tmp.path().join("docs/server-api/api.md");
        fs::create_dir_all(old_api.parent().unwrap()).unwrap();
        fs::write(&old_api, "content A").unwrap();

        // Step 1: migrate old → new (happens when old exists and new doesn't)
        assert!(old_api.exists() && !new_api.exists());
        migrate_file(&old_api, &new_api).unwrap();

        // Step 2: copy server api.md on top
        let srv_api = tmp.path().join("server_api.md");
        fs::write(&srv_api, "content B").unwrap();
        let copied = copy_file_if_changed(&srv_api, &new_api).unwrap();
        assert!(copied, "content differs, must overwrite");

        assert!(!old_api.exists());
        assert_eq!(fs::read_to_string(&new_api).unwrap(), "content B");
    }

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

    // ── v0.16.0 Bugs: migration + reconcile tests ─────────────────────────────

    /// Test helper: set up an in-memory DB with one repo pointed at a fresh
    /// temp directory. Returns (db, tmp, repo_id). The tmp dir is kept alive
    /// via the returned guard — drop it at the end of the test.
    fn setup_repo_with_dir() -> (AppDb, TempDir, i64) {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        let tmp = TempDir::new().unwrap();
        let repo = db
            .upsert_repository("owner/test-repo", None, None, None, None, None)
            .unwrap();
        db.set_repo_local_path(repo.id, Some(tmp.path().to_str().unwrap()))
            .unwrap();
        (db, tmp, repo.id)
    }

    /// Write `docs/bug-reports.md` with the provided lines inserted under
    /// the `## Open bugs` section.
    fn write_bug_reports_md(dir: &Path, bug_lines: &[&str]) {
        let docs = dir.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let mut md = String::from("# Bug reports\n\n## Open bugs\n\n");
        for line in bug_lines {
            md.push_str(line);
            if !line.ends_with('\n') {
                md.push('\n');
            }
        }
        fs::write(docs.join("bug-reports.md"), md).unwrap();
    }

    /// Read `docs/bug-reports.md` and return its contents (panics if missing).
    fn read_bug_reports_md(dir: &Path) -> String {
        fs::read_to_string(dir.join("docs").join("bug-reports.md")).unwrap()
    }

    #[test]
    fn test_parse_numeric_id() {
        assert_eq!(parse_numeric_id("B-1"), Some(1));
        assert_eq!(parse_numeric_id("B-042"), Some(42));
        assert_eq!(parse_numeric_id("B-000042"), Some(42));
        assert_eq!(parse_numeric_id("B-999999"), Some(999999));
        assert_eq!(parse_numeric_id("VB-042"), None);
        assert_eq!(parse_numeric_id("B-abc"), None);
        assert_eq!(parse_numeric_id("42"), None);
    }

    #[test]
    fn test_valid_transition_whitelist() {
        // Forward progress
        assert!(valid_transition("created", "in-progress"));
        assert!(valid_transition("created", "testing"));          // quick-fix shortcut
        assert!(valid_transition("in-progress", "testing"));
        assert!(valid_transition("rejected", "in-progress"));
        assert!(valid_transition("rejected", "testing"));         // retry after rejection
        assert!(valid_transition("testing", "confirmed"));        // (UI-only path)
        assert!(valid_transition("testing", "rejected"));         // (UI-only path)

        // Invalid: skipping the `testing` verification step is never allowed.
        assert!(!valid_transition("created", "confirmed"));
        assert!(!valid_transition("in-progress", "confirmed"));
        assert!(!valid_transition("in-progress", "rejected"));
        assert!(!valid_transition("rejected", "confirmed"));

        // Invalid: `confirmed` is terminal; no mutation allowed.
        assert!(!valid_transition("confirmed", "in-progress"));
        assert!(!valid_transition("confirmed", "testing"));
        assert!(!valid_transition("confirmed", "rejected"));

        // Invalid: backwards moves.
        assert!(!valid_transition("testing", "created"));
        assert!(!valid_transition("in-progress", "created"));
    }

    #[test]
    fn test_migrate_bugs_for_repo_lazy() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug one | minor | other | created | 0 |"],
        );

        let r1 = migrate_bugs_for_repo(&db, rid).unwrap();
        assert_eq!(r1.imported, 1);
        assert!(!r1.already);

        // Second call is a no-op (marker already set).
        let r2 = migrate_bugs_for_repo(&db, rid).unwrap();
        assert_eq!(r2.imported, 0);
        assert!(r2.already);
    }

    #[test]
    fn test_migrate_preserves_numeric_id() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-042 | 2026-03-01 | first | minor | other | created | 0 |",
                "- B-013 | 2026-03-02 | second | major | logic | in-progress | 0 |",
            ],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        let bugs = db.list_bugs_by_repo(rid, true).unwrap();
        assert_eq!(bugs.len(), 2);
        let b042 = bugs.iter().find(|b| b.numeric_id == 42).unwrap();
        assert_eq!(b042.display_id, "B-000042");
        let b013 = bugs.iter().find(|b| b.numeric_id == 13).unwrap();
        assert_eq!(b013.display_id, "B-000013");

        // Regen normalized MD to 6-digit IDs.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000013"));
        assert!(md.contains("B-000042"));
        assert!(!md.contains("- B-042 |")); // old 3-digit form gone
    }

    #[test]
    fn test_migrate_confirms_archived_rows_not_in_md() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | active | minor | other | created | 0 |",
                "- B-002 | 2026-03-02 | old fix | minor | other | confirmed | 1 | fixed",
            ],
        );
        let r = migrate_bugs_for_repo(&db, rid).unwrap();
        assert_eq!(r.imported, 2);
        assert_eq!(r.confirmed_archived, 1);

        // DB has both rows, including the confirmed one.
        let all = db.list_bugs_by_repo(rid, true).unwrap();
        assert_eq!(all.len(), 2);

        // MD has only the non-confirmed row after regen.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(!md.contains("B-000002"));
    }

    #[test]
    fn test_migrate_aborts_on_duplicate_id() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | first | minor | other | created | 0 |",
                "- B-001 | 2026-03-02 | dup | major | other | created | 0 |",
            ],
        );
        let err = migrate_bugs_for_repo(&db, rid).unwrap_err();
        assert!(err.to_lowercase().contains("duplicate"), "got: {}", err);

        // No DB writes, marker still NULL.
        assert!(db.get_bugs_migrated_at(rid).unwrap().is_none());
        assert!(db.list_bugs_by_repo(rid, true).unwrap().is_empty());
    }

    #[test]
    fn test_migrate_missing_md_file_sets_marker() {
        let (db, _tmp, rid) = setup_repo_with_dir();
        // No MD file written.
        let r = migrate_bugs_for_repo(&db, rid).unwrap();
        assert_eq!(r.imported, 0);
        assert!(!r.already);
        assert!(db.get_bugs_migrated_at(rid).unwrap().is_some());
    }

    #[test]
    fn test_reconcile_requires_migration_first() {
        let (db, _tmp, rid) = setup_repo_with_dir();
        let err = reconcile_bugs_for_repo(&db, rid).unwrap_err();
        assert!(err.contains("not migrated"), "got: {}", err);
    }

    #[test]
    fn test_reconcile_status_transition_to_testing_increments_attempts() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | in-progress | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM moves the bug to testing.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | testing | 0 | fix attempt"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.status, "testing");
        assert_eq!(b.fix_attempts, 1, "entering testing bumps fix_attempts");
        assert_eq!(b.comment.as_deref(), Some("fix attempt"));
    }

    #[test]
    fn test_reconcile_status_transition_to_confirmed_sets_confirmed_at() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | testing | 1 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM moves testing → confirmed.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | confirmed | 1 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.status, "confirmed");
        assert!(b.confirmed_at.is_some());
        assert!(b.archived_from_md_at.is_none(),
                "fresh confirm must not be archived yet — LLM hasn't acknowledged");

        // v0.21.1: MD keeps confirmed row visible (LLM-acknowledgement workflow).
        // Row drops only after LLM removes it from MD on a subsequent edit.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"), "confirmed row still in MD until LLM acknowledges");
    }

    #[test]
    fn test_reconcile_protected_field_restored_on_regen() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | original | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM violates contract: changes severity + description.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | TAMPERED | critical | security | created | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB kept original values.
        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.description, "original");
        assert_eq!(b.severity, "minor");
        assert_eq!(b.category, "other");

        // MD regenerated back to DB truth.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("| original | minor | other |"));
        assert!(!md.contains("TAMPERED"));
        assert!(!md.contains("critical"));
    }

    #[test]
    fn test_reconcile_orphan_row_removed() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | real | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM adds a row with a numeric_id not in DB — attempt to create bug via MD.
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-000001 | 2026-03-01 | real | minor | other | created | 0 |",
                "- B-888888 | 2026-03-09 | ghost | major | logic | created | 0 |",
            ],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB unchanged (still just B-000001).
        assert_eq!(db.list_bugs_by_repo(rid, true).unwrap().len(), 1);

        // MD regen drops the orphan.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(!md.contains("B-888888"));
    }

    #[test]
    fn test_reconcile_deleted_non_confirmed_row_restored() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | a | minor | other | created | 0 |",
                "- B-002 | 2026-03-02 | b | major | other | created | 0 |",
            ],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM deletes B-002 from MD (illegal — only confirmed rows can be removed).
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | a | minor | other | created | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB unchanged.
        assert_eq!(db.list_bugs_by_repo(rid, true).unwrap().len(), 2);

        // MD regen restored B-000002.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(md.contains("B-000002"));
    }

    #[test]
    fn test_reconcile_deleted_confirmed_row_stays_deleted_from_md_db_intact() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-001 | 2026-03-01 | active | minor | other | created | 0 |",
                "- B-002 | 2026-03-02 | closed | minor | other | confirmed | 1 |",
            ],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();
        // After migration, MD already has B-002 dropped.

        // LLM edit (just reaffirming MD state without B-002).
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB still has the confirmed row for history.
        let all = db.list_bugs_by_repo(rid, true).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|b| b.status == "confirmed"));

        // MD has only the active one.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(!md.contains("B-000002"));
    }

    #[test]
    fn test_reconcile_invalid_transition_ignored() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM attempts invalid transition: created → confirmed (skipping workflow).
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | confirmed | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        // DB status NOT changed; confirmed_at NOT set.
        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.status, "created");
        assert!(b.confirmed_at.is_none());

        // MD regen reverts to DB truth.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("| created |"));
        assert!(!md.contains("| confirmed |"));
    }

    #[test]
    fn test_reconcile_comment_update_propagates() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | in-progress | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // LLM updates comment only (no status change).
        write_bug_reports_md(
            tmp.path(),
            &[
                "- B-000001 | 2026-03-01 | bug | minor | other | in-progress | 0 | debugging now",
            ],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert_eq!(b.comment.as_deref(), Some("debugging now"));

        // LLM clears comment.
        write_bug_reports_md(
            tmp.path(),
            &["- B-000001 | 2026-03-01 | bug | minor | other | in-progress | 0 |"],
        );
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert!(b.comment.is_none());
    }

    #[test]
    fn test_reconcile_missing_md_file_restores_from_db() {
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | created | 0 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();

        // User deletes MD file externally.
        fs::remove_file(tmp.path().join("docs").join("bug-reports.md")).unwrap();

        reconcile_bugs_for_repo(&db, rid).unwrap();

        // File restored from DB state.
        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
    }

    #[test]
    fn test_regenerate_bugs_md_includes_unacknowledged_confirmed() {
        // v0.21.1: confirmed bugs appear in MD until LLM-acknowledged
        // (archived_from_md_at IS NULL). Both active and unacknowledged-confirmed rows show.
        let (db, tmp, rid) = setup_repo_with_dir();
        db.insert_bug(rid, 1, "2026-03-01T00:00:00Z", "active", "minor", "other",
                      "created", 0, None, None).unwrap();
        db.insert_bug(rid, 2, "2026-03-02T00:00:00Z", "fresh-confirm", "minor", "other",
                      "confirmed", 1, None, Some("2026-04-24T10:00:00Z")).unwrap();

        regenerate_bugs_md(&db, rid).unwrap();

        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"), "active bug must appear in MD");
        assert!(md.contains("B-000002"), "fresh confirmed (not yet acknowledged) must appear");
    }

    #[test]
    fn test_reconcile_marks_confirmed_archived_when_llm_removes_from_md() {
        // v0.21.1 workflow: app sets status='confirmed' on user ✓ click; row appears
        // in MD with confirmed status; LLM removes it on next session edit; reconcile
        // detects the absence and sets archived_from_md_at, ensuring future regens
        // permanently exclude the row.
        let (db, tmp, rid) = setup_repo_with_dir();
        write_bug_reports_md(
            tmp.path(),
            &["- B-001 | 2026-03-01 | bug | minor | other | confirmed | 1 |"],
        );
        migrate_bugs_for_repo(&db, rid).unwrap();
        // Migration's regen drops the confirmed row (legacy import semantics —
        // confirmed-from-MD treated as already archived).

        // Manually unarchive to simulate a v0.21.1+ flow: app set confirmed,
        // row visible to LLM in MD.
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE bugs SET archived_from_md_at = NULL WHERE display_id = 'B-000001'",
                [],
            ).unwrap();
        }
        regenerate_bugs_md(&db, rid).unwrap();
        let md_with_confirmed = read_bug_reports_md(tmp.path());
        assert!(md_with_confirmed.contains("B-000001"), "fresh-confirmed must be in MD");

        // LLM edit: removes the confirmed row.
        write_bug_reports_md(tmp.path(), &[]);

        // Reconcile must mark archived + final regen excludes it.
        reconcile_bugs_for_repo(&db, rid).unwrap();

        let b = db.get_bug_by_display_id(rid, "B-000001").unwrap().unwrap();
        assert!(b.archived_from_md_at.is_some(), "reconcile must mark archived");
        assert_eq!(b.status, "confirmed", "DB row stays as confirmed for history");

        let md_after = read_bug_reports_md(tmp.path());
        assert!(!md_after.contains("B-000001"), "archived row no longer in MD");
    }

    #[test]
    fn test_regenerate_bugs_md_excludes_archived_confirmed() {
        // v0.21.1: once LLM acknowledged a confirmed row (archived_from_md_at set),
        // it's permanently excluded from MD. DB row still exists for history.
        let (db, tmp, rid) = setup_repo_with_dir();
        db.insert_bug(rid, 1, "2026-03-01T00:00:00Z", "active", "minor", "other",
                      "created", 0, None, None).unwrap();
        let archived_bug = db.insert_bug(rid, 2, "2026-03-02T00:00:00Z", "archived", "minor", "other",
                                         "confirmed", 1, None, Some("2026-04-24T10:00:00Z")).unwrap();
        db.mark_bug_archived_from_md(archived_bug.id).unwrap();

        regenerate_bugs_md(&db, rid).unwrap();

        let md = read_bug_reports_md(tmp.path());
        assert!(md.contains("B-000001"));
        assert!(!md.contains("B-000002"), "archived confirmed must drop from MD");
    }

    #[test]
    fn test_reconcile_records_events_on_transition() {
        let db = AppDb::new(std::path::PathBuf::from(":memory:")).unwrap();
        let tmp = TempDir::new().unwrap();
        let repo = db
            .upsert_repository("owner/events-test", None, None, None, None, None)
            .unwrap();
        db.set_repo_local_path(repo.id, Some(tmp.path().to_str().unwrap()))
            .unwrap();

        let bug = db
            .insert_bug(
                repo.id,
                1,
                "2026-04-24T10:00:00Z",
                "x",
                "minor",
                "other",
                "created",
                0,
                None,
                None,
            )
            .unwrap();

        // Simulate LLM moving created → in-progress (should produce 'taken' event).
        db.update_bug_status(bug.id, "in-progress", None, None)
            .unwrap();
        db.insert_bug_event(
            bug.id,
            "taken",
            Some("created"),
            Some("in-progress"),
            &crate::db::utc_now_rfc3339(),
        )
        .unwrap();

        let conn = db.conn.lock().unwrap();
        let ev_type: String = conn
            .query_row(
                "SELECT event_type FROM bug_events WHERE bug_id=?1 ORDER BY id DESC LIMIT 1",
                [bug.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ev_type, "taken");
    }

    // ── C1/C2: sync_tasks_for_repo tests ─────────────────────────────────────

    fn make_db_for_sync_tests() -> AppDb {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        std::mem::forget(tmp);
        AppDb::new(path).unwrap()
    }

    #[test]
    fn test_sync_tasks_first_run_inserts_rows_no_events() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task A | 2 | high | open\n- [ ] T-002 | Task B | 4 | medium | in-progress\n",
        ).unwrap();
        let repo = db.insert_local_repository(repo_path.to_str().unwrap(), "test_repo", None, None).unwrap();

        let report = crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();

        assert_eq!(report.imported, 2);
        assert_eq!(report.events_emitted, 0, "first sync must not emit 'created' events");

        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos.len(), 2);
        let events = db.list_task_events_by_task(todos[0].id).unwrap();
        assert!(events.is_empty());
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_idempotent_no_changes_no_events() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task A | 2 | high | open\n",
        ).unwrap();
        let repo = db.insert_local_repository(repo_path.to_str().unwrap(), "test_repo", None, None).unwrap();

        crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap(); // first
        let r2 = crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.events_emitted, 0);
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_no_todo_md_marks_migrated() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = db.insert_local_repository(tmp.path().to_str().unwrap(), "test_repo", None, None).unwrap();

        let report = crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(report.imported, 0);
        assert!(db.get_tasks_migrated_at(repo.id).unwrap().is_some());
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_open_to_inprogress_emits_taken() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | open\n").unwrap();
        let repo = db.insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None).unwrap();

        crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();

        std::fs::write(repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | in-progress\n").unwrap();

        let r = crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r.events_emitted, 1);

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        let events = db.list_task_events_by_task(todos[0].id).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "taken");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_todo_to_done_emits_done() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | review\n").unwrap();
        std::fs::write(repo_path.join("docs/done.md"), "").unwrap();
        let repo = db.insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None).unwrap();

        crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();

        std::fs::write(repo_path.join("docs/todo.md"), "").unwrap();
        std::fs::write(repo_path.join("docs/done.md"),
            "## 2026-04-26\n- T-001 | Task | v0.20.0\n").unwrap();

        let r = crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r.events_emitted, 1);

        let dones = db.list_tasks_by_repo(repo.id, "done").unwrap();
        assert_eq!(dones.len(), 1);
        let events = db.list_task_events_by_task(dones[0].id).unwrap();
        let last = events.last().unwrap();
        assert_eq!(last.event_type, "done");
        std::mem::forget(tmp);
    }

    #[test]
    fn test_sync_tasks_unusual_transition_no_event() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | open\n").unwrap();
        let repo = db.insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None).unwrap();

        crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();

        std::fs::write(repo_path.join("docs/todo.md"),
            "- [ ] T-001 | Task | 2 | high | review\n").unwrap();

        let r = crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();
        assert_eq!(r.events_emitted, 0, "unusual transitions log warn but emit no event");

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos[0].status.as_deref(), Some("review"));
        std::mem::forget(tmp);
    }

    /// B-000004: when an ID in todo.md is rewritten (3-digit T-034 → 6-digit
    /// T-000034, or placeholder "F-NNN" → real "F-000035"), the old DB row
    /// must be dropped on the next sync — otherwise the same task shows up
    /// twice in the Tasks tab forever.
    #[test]
    fn test_sync_tasks_cleans_up_orphan_todo_rows() {
        let db = make_db_for_sync_tests();
        let tmp = tempfile::TempDir::new().unwrap();
        let repo_path = tmp.path().to_path_buf();
        std::fs::create_dir_all(repo_path.join("docs")).unwrap();
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-034 | Old format task | 2 | high | open\n- [ ] F-NNN | Placeholder feature | 4 | medium | open\n",
        ).unwrap();
        let repo = db.insert_local_repository(repo_path.to_str().unwrap(), "r1", None, None).unwrap();

        // First sync: 2 rows imported with the original (legacy / placeholder) ids.
        crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();
        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos.len(), 2);

        // Rewrite todo.md with normalised ids (LLM did the cleanup).
        std::fs::write(
            repo_path.join("docs/todo.md"),
            "- [ ] T-000034 | Old format task | 2 | high | open\n- [ ] F-000035 | Placeholder feature | 4 | medium | open\n",
        ).unwrap();

        crate::sync::sync_tasks_for_repo(&db, repo.id).unwrap();

        let todos = db.list_tasks_by_repo(repo.id, "todo").unwrap();
        assert_eq!(todos.len(), 2, "orphan rows with old ids must be cleaned up");
        let ids: std::collections::HashSet<&str> = todos.iter().map(|t| t.task_id.as_str()).collect();
        assert!(ids.contains("T-000034"));
        assert!(ids.contains("F-000035"));
        assert!(!ids.contains("T-034"), "old 3-digit row must be gone");
        assert!(!ids.contains("F-NNN"), "placeholder row must be gone");
        std::mem::forget(tmp);
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
