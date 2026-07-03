// Renders SDH workflow-rule skills from bundled `_global/skill.<name>.md.tmpl`
// templates to two targets: user-level Claude Code skills
// (`~/.claude/skills/<name>/SKILL.md`) and per-repo reference copies
// (`<repo>/docs/sdh_skills/<name>.md`). Single source -> two render targets.

use crate::db::AppDb;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const SKILL_PREFIX: &str = "skill.";
const SKILL_SUFFIX: &str = ".md.tmpl";
/// Namespace prefix for skills this app owns. Only `sdh-*` entries are ever
/// pruned — user-authored skills sharing the directory are never touched.
const SKILL_NAMESPACE: &str = "sdh-";

/// Enumerate bundled skill templates as (skill_name, content).
/// A skill template is a `_global` file named `skill.<name>.md.tmpl`;
/// `skill_name` is the middle segment (e.g. "sdh-cross-repo-req-send").
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
    let keep: HashSet<String> = skills.iter().map(|(name, _)| name.clone()).collect();
    let mut n = 0;
    for (name, content) in &skills {
        let dir = claude_dir.join("skills").join(name);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join("SKILL.md"), content).map_err(|e| e.to_string())?;
        n += 1;
    }
    // T-000149: drop stale `sdh-*` skill folders no longer in the bundle (e.g.
    // the pre-split combined skills) so obsolete rules stop surfacing.
    prune_stale_global_skills(claude_dir, &keep)?;
    Ok(n)
}

/// Render each skill to `<repo_base>/docs/sdh_skills/<name>.md` (flat reference
/// copy for non-Claude-Code agents). Returns count written.
pub fn render_skills_to_repo(db: &AppDb, repo_base: &Path) -> Result<usize, String> {
    let skills = list_skill_templates(db)?;
    let keep: HashSet<String> = skills.iter().map(|(name, _)| name.clone()).collect();
    let dir = repo_base.join("docs").join("sdh_skills");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut n = 0;
    for (name, content) in &skills {
        fs::write(dir.join(format!("{}.md", name)), content).map_err(|e| e.to_string())?;
        n += 1;
    }
    // T-000149: drop stale `sdh-*.md` reference copies no longer in the bundle.
    prune_stale_repo_skills(&dir, &keep)?;
    Ok(n)
}

/// Remove `<claude_dir>/skills/sdh-*` folders that are not in `keep` (the current
/// bundle). Only the `sdh-` namespace is touched; user-authored skills survive.
/// A missing skills dir is a no-op. Returns the number of folders pruned.
fn prune_stale_global_skills(claude_dir: &Path, keep: &HashSet<String>) -> Result<usize, String> {
    let skills_dir = claude_dir.join("skills");
    let entries = match fs::read_dir(&skills_dir) {
        Ok(e) => e,
        Err(_) => return Ok(0), // no skills dir yet — nothing to prune
    };
    let mut pruned = 0;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(SKILL_NAMESPACE) && !keep.contains(&name) {
            fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
            pruned += 1;
        }
    }
    Ok(pruned)
}

/// Remove `<dir>/sdh-*.md` reference copies whose stem is not in `keep`. Only the
/// `sdh-` namespace is touched. A missing dir is a no-op. Returns count pruned.
fn prune_stale_repo_skills(dir: &Path, keep: &HashSet<String>) -> Result<usize, String> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(0),
    };
    let mut pruned = 0;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let fname = entry.file_name().to_string_lossy().to_string();
        if let Some(stem) = fname.strip_suffix(".md") {
            if stem.starts_with(SKILL_NAMESPACE) && !keep.contains(stem) {
                fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
                pruned += 1;
            }
        }
    }
    Ok(pruned)
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
    fn test_render_global_prunes_stale_sdh_skills() {
        let db = db_with_skill(); // current bundle: sdh-alpha, sdh-beta
        let tmp = TempDir::new().unwrap();
        // Pre-seed a stale sdh-* skill (a previous, larger set) and a
        // user-authored non-sdh skill that must survive the prune.
        let stale = tmp.path().join("skills/sdh-obsolete");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("SKILL.md"), "OLD").unwrap();
        let user = tmp.path().join("skills/my-own-skill");
        fs::create_dir_all(&user).unwrap();
        fs::write(user.join("SKILL.md"), "MINE").unwrap();

        render_skills_global(&db, tmp.path()).unwrap();

        assert!(!stale.exists(), "stale sdh-* skill folder pruned");
        assert!(user.exists(), "non-sdh user skill left untouched");
        assert!(tmp.path().join("skills/sdh-alpha/SKILL.md").exists());
        assert!(tmp.path().join("skills/sdh-beta/SKILL.md").exists());
    }

    #[test]
    fn test_render_repo_prunes_stale_sdh_skills() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("docs/sdh_skills");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sdh-obsolete.md"), "OLD").unwrap();
        fs::write(dir.join("README.md"), "keep-me").unwrap(); // non-sdh, survives

        render_skills_to_repo(&db, tmp.path()).unwrap();

        assert!(
            !dir.join("sdh-obsolete.md").exists(),
            "stale sdh-*.md reference copy pruned"
        );
        assert!(dir.join("README.md").exists(), "non-sdh file left untouched");
        assert!(dir.join("sdh-alpha.md").exists());
        assert!(dir.join("sdh-beta.md").exists());
    }

    #[test]
    fn test_render_idempotent_overwrite() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        render_skills_to_repo(&db, tmp.path()).unwrap();
        let n = render_skills_to_repo(&db, tmp.path()).unwrap();
        assert_eq!(n, 2);
    }

    /// End-to-end guard: the REAL bundle (via seed_bundled_templates) must carry
    /// exactly the twelve sdh-* workflow skills, each with valid frontmatter, and
    /// they must render to a repo's docs/sdh_skills/. Catches a missing/renamed
    /// or malformed skill template at test time instead of at app runtime.
    #[test]
    fn test_real_bundle_renders_all_twelve_skills() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.db");
        std::mem::forget(tmp);
        let db = AppDb::new(path).unwrap();
        crate::template_seeder::seed_bundled_templates(&db).unwrap();

        let skills = list_skill_templates(&db).unwrap();
        let names: Vec<&str> = skills.iter().map(|(n, _)| n.as_str()).collect();
        for expected in [
            "sdh-cross-repo-req-send",
            "sdh-cross-repo-req-answer",
            "sdh-cross-repo-announce-send",
            "sdh-cross-repo-announce-read",
            "sdh-api-contract-maintain",
            "sdh-api-contract-consume",
            "sdh-feature-flow-docs",
            "sdh-phase-workflow",
            "sdh-visual-mockups",
            "sdh-release-lifecycle",
            "sdh-release-closure",
            "sdh-retro",
        ] {
            assert!(
                names.contains(&expected),
                "bundle missing skill {}",
                expected
            );
        }
        assert_eq!(skills.len(), 12, "exactly twelve sdh-* skills in the bundle");

        for (name, content) in &skills {
            assert!(
                content.starts_with("---\n"),
                "{} lacks YAML frontmatter",
                name
            );
            assert!(
                content.contains("name:"),
                "{} frontmatter lacks name:",
                name
            );
            assert!(
                content.contains("description:"),
                "{} frontmatter lacks description:",
                name
            );
        }

        let repo = TempDir::new().unwrap();
        let n = render_skills_to_repo(&db, repo.path()).unwrap();
        assert_eq!(n, 12);
        assert!(repo
            .path()
            .join("docs/sdh_skills/sdh-cross-repo-req-send.md")
            .exists());
    }
}
