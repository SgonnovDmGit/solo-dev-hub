// Renders SDH workflow-rule skills from bundled `_global/skill.<name>.md.tmpl`
// templates to two targets: user-level Claude Code skills
// (`~/.claude/skills/<name>/SKILL.md`) and per-repo reference copies
// (`<repo>/docs/sdh_skills/<name>.md`). Single source -> two render targets.

use crate::db::AppDb;
use std::fs;
use std::path::Path;

const SKILL_PREFIX: &str = "skill.";
const SKILL_SUFFIX: &str = ".md.tmpl";

/// Enumerate bundled skill templates as (skill_name, content).
/// A skill template is a `_global` file named `skill.<name>.md.tmpl`;
/// `skill_name` is the middle segment (e.g. "sdh-cross-repo-req").
/// Sorted by name for deterministic output.
pub fn list_skill_templates(db: &AppDb) -> Result<Vec<(String, String)>, String> {
    let files = db
        .list_template_files("_global")
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for f in files {
        if let Some(rest) = f.file_name.strip_prefix(SKILL_PREFIX) {
            if let Some(name) = rest.strip_suffix(SKILL_SUFFIX) {
                out.push((name.to_string(), f.content));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Render each skill to `<claude_dir>/skills/<name>/SKILL.md` (Claude Code
/// user-level layout: one folder per skill). Returns count written.
pub fn render_skills_global(db: &AppDb, claude_dir: &Path) -> Result<usize, String> {
    let skills = list_skill_templates(db)?;
    let mut n = 0;
    for (name, content) in skills {
        let dir = claude_dir.join("skills").join(&name);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join("SKILL.md"), content).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

/// Render each skill to `<repo_base>/docs/sdh_skills/<name>.md` (flat reference
/// copy for non-Claude-Code agents). Returns count written.
pub fn render_skills_to_repo(db: &AppDb, repo_base: &Path) -> Result<usize, String> {
    let skills = list_skill_templates(db)?;
    let dir = repo_base.join("docs").join("sdh_skills");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut n = 0;
    for (name, content) in skills {
        fs::write(dir.join(format!("{}.md", name)), content).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn db_with_skill() -> AppDb {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        std::mem::forget(tmp);
        let db = AppDb::new(path).unwrap();
        // Inject two synthetic skill templates (real bundle skills are authored
        // in later tasks). Also inject a non-skill `_global` file to prove the
        // prefix filter excludes it.
        db.upsert_template_file("_global", "skill.sdh-alpha.md.tmpl", "ALPHA BODY", false)
            .unwrap();
        db.upsert_template_file("_global", "skill.sdh-beta.md.tmpl", "BETA BODY", false)
            .unwrap();
        db.upsert_template_file("_global", ".gitignore.tmpl", "ignore", false)
            .unwrap();
        db
    }

    #[test]
    fn test_list_filters_and_sorts() {
        let db = db_with_skill();
        let skills = list_skill_templates(&db).unwrap();
        assert_eq!(skills.len(), 2, "only skill.* files, not .gitignore.tmpl");
        assert_eq!(skills[0].0, "sdh-alpha");
        assert_eq!(skills[1].0, "sdh-beta");
        assert_eq!(skills[0].1, "ALPHA BODY");
    }

    #[test]
    fn test_render_global_layout() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let n = render_skills_global(&db, tmp.path()).unwrap();
        assert_eq!(n, 2);
        let f = tmp.path().join("skills/sdh-alpha/SKILL.md");
        assert!(f.exists(), "one folder per skill with SKILL.md");
        assert_eq!(fs::read_to_string(&f).unwrap(), "ALPHA BODY");
    }

    #[test]
    fn test_render_repo_layout() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let n = render_skills_to_repo(&db, tmp.path()).unwrap();
        assert_eq!(n, 2);
        let f = tmp.path().join("docs/sdh_skills/sdh-beta.md");
        assert!(f.exists(), "flat <name>.md under docs/sdh_skills");
        assert_eq!(fs::read_to_string(&f).unwrap(), "BETA BODY");
    }

    #[test]
    fn test_render_idempotent_overwrite() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        render_skills_to_repo(&db, tmp.path()).unwrap();
        let n = render_skills_to_repo(&db, tmp.path()).unwrap();
        assert_eq!(n, 2);
    }
}
