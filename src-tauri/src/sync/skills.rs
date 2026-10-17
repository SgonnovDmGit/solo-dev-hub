// Renders SDH workflow-rule skills from bundled `_global/skill.<name>.md.tmpl`
// templates to two targets: user-level Claude Code skills
// (`~/.claude/skills/<name>/SKILL.md`) and per-repo reference copies
// (`<repo>/docs/sdh_skills/<name>.md`). Single source -> two render targets.

use crate::db::AppDb;
use crate::models::{Repository, RulesRefresh};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

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

/// Write `content` to `path` only when it differs from what is already there.
/// T-000203: both render passes now run on every rule save, not only during a
/// project sync, so an unconditional write would rewrite eighteen files per repo
/// across the whole portfolio on each save and churn mtimes for nothing.
fn write_if_changed(path: &Path, content: &str) -> Result<(), String> {
    if let Ok(existing) = fs::read_to_string(path) {
        if existing == content {
            return Ok(());
        }
    }
    fs::write(path, content).map_err(|e| e.to_string())
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
        write_if_changed(&dir.join("SKILL.md"), content)?;
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
        write_if_changed(&dir.join(format!("{}.md", name)), content)?;
        n += 1;
    }
    // T-000149: drop stale `sdh-*.md` reference copies no longer in the bundle.
    prune_stale_repo_skills(&dir, &keep)?;
    Ok(n)
}

/// Marker dropped beside a rendered skill copy: this copy was edited on purpose
/// and the owner has already approved the text. A suffix rather than a prefix so
/// it sorts next to the file it belongs to, and so the prune pass — which only
/// touches `sdh-*.md` — leaves it alone.
const APPROVED_SUFFIX: &str = ".approved";

/// What one collection pass found beside the repo copies.
#[derive(Debug, Default)]
struct Collected {
    taken: Vec<String>,
    lost: Vec<String>,
    superseded: Vec<String>,
    errors: Vec<String>,
}

/// T-000205: take an approved rule edit back out of a repo's own copy.
///
/// The copies under `docs/sdh_skills/` are normally outputs, rewritten on every
/// sync. This is the single exception: an agent that edits the skill it is
/// reading and drops `<name>.md.approved` beside it is handing the text back,
/// and that marker stands for the owner's approval, given in chat before the
/// file was written. Nothing here verifies the approval — see the risk recorded
/// in T-000205; what it does give is a trail, since every uptake is reported.
///
/// **Order matters and is the whole trap.** This must run before any render pass
/// touches these repos: the render overwrites unconditionally, so rendering
/// first destroys the edit at the exact moment it was about to be collected.
/// That is why it is private and reached only through `refresh_rules`.
///
/// Two repos handing back the same skill with *different* text in one pass is a
/// conflict: both are refused and their markers left in place. Taking the last
/// one silently would lose somebody's rule without a trace.
///
/// T-000207 adds two outcomes that are not an uptake:
/// - **lost** — the copy beside the marker is byte-equal to the current rule.
///   The edit the marker stood for was overwritten before anyone collected it;
///   taking the copy up would report a success and flag an untouched rule as a
///   fork. The marker is cleared and the loss reported, the rule left alone.
/// - **superseded** — `settings_edit` names the skill the owner has just saved
///   in Settings. That explicit edit wins over a pending hand-back of the same
///   skill: the hand-back is not taken, its marker is cleared, and it is
///   reported so the owner knows an agent's text was set aside.
///
/// Lost and superseded entries read `<skill> (<repo>)`.
fn collect_approved_skill_edits(
    db: &AppDb,
    repos: &[Repository],
    settings_edit: Option<&str>,
) -> Collected {
    let mut out = Collected::default();
    let known: HashMap<String, String> = match list_skill_templates(db) {
        Ok(v) => v.into_iter().collect(),
        Err(e) => {
            out.errors.push(format!("skill list: {}", e));
            return out;
        }
    };

    // skill name -> handed-back copies, one per repo that flagged it.
    let mut handed: HashMap<String, Vec<(String, String, PathBuf)>> = HashMap::new();

    for repo in repos {
        let Some(path) = repo.local_path.as_deref() else {
            continue;
        };
        let dir = Path::new(path).join("docs").join("sdh_skills");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue; // no copies here yet — nothing can have been handed back
        };
        let label = repo.display_name();
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(marker) = file_name.to_str() else {
                continue;
            };
            let Some(stem) = marker.strip_suffix(APPROVED_SUFFIX) else {
                continue;
            };
            let Some(name) = stem.strip_suffix(".md") else {
                continue;
            };
            let Some(current) = known.get(name) else {
                out.errors.push(format!(
                    "{}: {} is flagged but is not a known skill — a rule cannot be invented in a repo copy",
                    label, marker
                ));
                continue;
            };
            if settings_edit == Some(name) {
                match fs::remove_file(entry.path()) {
                    Ok(()) => out.superseded.push(format!("{} ({})", name, label)),
                    Err(e) => out
                        .errors
                        .push(format!("{}: marker not cleared: {}", name, e)),
                }
                continue;
            }
            let copy = dir.join(stem);
            match fs::read_to_string(&copy) {
                Ok(content) if content == *current => match fs::remove_file(entry.path()) {
                    Ok(()) => out.lost.push(format!("{} ({})", name, label)),
                    Err(e) => out
                        .errors
                        .push(format!("{}: marker not cleared: {}", name, e)),
                },
                Ok(content) => {
                    // The marker may carry the approval line. Empty is allowed;
                    // when present it is worth having in the log.
                    if let Ok(note) = fs::read_to_string(entry.path()) {
                        let note = note.trim();
                        if !note.is_empty() {
                            eprintln!("[skills] {} hands back {}: {}", label, name, note);
                        }
                    }
                    handed.entry(name.to_string()).or_default().push((
                        label.clone(),
                        content,
                        entry.path(),
                    ));
                }
                Err(e) => out
                    .errors
                    .push(format!("{}: cannot read {}: {}", label, stem, e)),
            }
        }
    }

    for (name, copies) in handed {
        let first = &copies[0].1;
        if let Some(other) = copies.iter().find(|(_, content, _)| content != first) {
            out.errors.push(format!(
                "{} was handed back with different text by {} and {} — resolve it and re-flag one of them",
                name, copies[0].0, other.0
            ));
            continue; // markers stay put: nobody's edit is thrown away
        }
        let file_name = format!("{}{}{}", SKILL_PREFIX, name, SKILL_SUFFIX);
        if let Err(e) = db.upsert_template_file("_global", &file_name, first, true) {
            out.errors
                .push(format!("{}: cannot store the new text: {}", name, e));
            continue;
        }
        for (_, _, marker) in &copies {
            if let Err(e) = fs::remove_file(marker) {
                // The text is already stored; a surviving marker now sits beside
                // a copy equal to the rule, so the next pass reports it as lost
                // instead of taking it up again.
                out.errors
                    .push(format!("{}: marker not cleared: {}", name, e));
            }
        }
        out.taken.push(name);
    }
    out.taken.sort();
    out.lost.sort();
    out.superseded.sort();
    out
}

/// What a `refresh_rules` pass renders when nothing was taken up. An uptake
/// always renders everywhere on top of this — a rule belongs to the whole
/// portfolio, not to the repo that happened to hand it back.
pub enum RenderScope<'a> {
    /// `~/.claude/skills/` plus every repo with a local checkout.
    Everywhere,
    /// One checkout only (init docs). `~/.claude/skills/` is left alone.
    Repo(&'a Path),
    /// `~/.claude/skills/` only (the global-rules button).
    Global,
    /// Nothing of its own: the caller renders the repos it owns afterwards
    /// (a project sync, through `write_repo_skeletons`).
    UptakeOnly,
}

/// T-000207: the one entry point that writes rule copies — collect the
/// hand-backs across the WHOLE portfolio, then render.
///
/// Before this, four paths re-rendered copies in every repo without collecting
/// first (an uptake in another project, startup after a seed, a Settings save,
/// init docs), so a pending edit anywhere outside the synced project was
/// overwritten and its surviving marker later "took up" the canonical text.
/// Collection therefore always covers every repo with a local path, whatever
/// `scope` renders.
///
/// `claude_dir` is `~/.claude`; `None` skips the user-level target only.
/// `settings_edit` names the skill the owner has just saved in Settings, whose
/// pending hand-backs are superseded rather than taken.
pub fn refresh_rules(
    db: &AppDb,
    claude_dir: Option<&Path>,
    scope: RenderScope,
    settings_edit: Option<&str>,
) -> RulesRefresh {
    let mut report = RulesRefresh::default();
    let repos = match db.list_repos_with_local_path(None) {
        Ok(v) => v,
        Err(e) => {
            report.errors.push(format!("repository list: {}", e));
            Vec::new()
        }
    };

    let collected = collect_approved_skill_edits(db, &repos, settings_edit);
    report.absorb(collected);
    let everywhere = !report.taken.is_empty();
    render_scope(db, claude_dir, scope, everywhere, &repos, &mut report);
    report
}

/// The render half of a refresh: `scope`, widened to everywhere when
/// `everywhere` is set (an uptake, or a startup seed that changed the bundle).
/// Only ever called after the collection over `repos` has run.
fn render_scope(
    db: &AppDb,
    claude_dir: Option<&Path>,
    scope: RenderScope,
    everywhere: bool,
    repos: &[Repository],
    report: &mut RulesRefresh,
) {
    let everywhere = everywhere || matches!(scope, RenderScope::Everywhere);
    if everywhere {
        let (skills, touched, errors) = render_skills_everywhere(db, claude_dir, repos);
        report.skills = skills;
        report.repos = touched;
        report.errors.extend(errors);
        if !matches!(scope, RenderScope::Repo(_)) {
            return;
        }
    }
    match scope {
        RenderScope::Repo(base) => {
            // Rendered even after an everywhere pass: the target may not be among
            // the stored checkouts, and an unchanged file is not rewritten anyway.
            match render_skills_to_repo(db, base) {
                Ok(_) if everywhere => {}
                Ok(_) => report.repos = 1,
                Err(e) => report.errors.push(format!("{}: {}", base.display(), e)),
            }
        }
        RenderScope::Global => match claude_dir.map(|dir| render_skills_global(db, dir)) {
            Some(Ok(n)) => report.skills = n,
            Some(Err(e)) => report.errors.push(format!("~/.claude/skills: {}", e)),
            None => {}
        },
        RenderScope::Everywhere | RenderScope::UptakeOnly => {}
    }
}

impl RulesRefresh {
    fn absorb(&mut self, collected: Collected) {
        self.taken = collected.taken;
        self.lost = collected.lost;
        self.superseded = collected.superseded;
        self.errors.extend(collected.errors);
    }
}

/// T-000203: apply the current rule text everywhere an agent reads it from —
/// the user-level Claude Code skills plus every repo with a local checkout.
///
/// Until this existed the two targets had unrelated triggers: `~/.claude/skills/`
/// only on the global-rules button, `docs/sdh_skills/` only on a project sync
/// (automatic solely for projects opted into auto-sync). An agreed rule could
/// therefore sit undelivered for days while agents kept working off the old text.
///
/// Private since T-000207: reached only through `refresh_rules`, which collects
/// the hand-backs in `repos` before this overwrites them.
///
/// Returns `(skills, repos, errors)`. Errors are per-target and non-fatal: one
/// unwritable repo must not stop the rest of the portfolio from being updated.
fn render_skills_everywhere(
    db: &AppDb,
    claude_dir: Option<&Path>,
    repos: &[Repository],
) -> (usize, usize, Vec<String>) {
    let mut errors = Vec::new();
    let skills = match claude_dir.map(|dir| render_skills_global(db, dir)) {
        Some(Ok(n)) => n,
        Some(Err(e)) => {
            errors.push(format!("~/.claude/skills: {}", e));
            0
        }
        None => 0,
    };
    let mut touched = 0;
    for repo in repos {
        let Some(path) = repo.local_path.as_deref() else {
            continue;
        };
        let base = Path::new(path);
        // A stored local_path can point at a checkout that only exists on
        // another machine. That is normal portfolio state, not a failure —
        // skip it silently rather than drowning the real errors.
        if !base.is_dir() {
            continue;
        }
        match render_skills_to_repo(db, base) {
            Ok(_) => touched += 1,
            Err(e) => {
                // A local-only repo has no github_name — fall back to its path
                // so the message still points at something the owner can find.
                let label = repo.github_name.as_deref().unwrap_or(path);
                errors.push(format!("{}: {}", label, e));
            }
        }
    }
    (skills, touched, errors)
}

/// Startup, on every launch: collect pending hand-backs, THEN seed the bundled
/// templates, then render everywhere if the seed changed anything or the
/// collection took up or lost something.
///
/// The order is the point (T-000207 follow-up). The seed rewrites non-custom
/// rows to the new bundle text, so collecting after it compares a stale marker's
/// copy — equal to the OLD canonical text — against the NEW row: the stale copy
/// would be taken up, flagged custom, and silently revert the bundle update.
/// Collected first, a genuine hand-back is stored as custom (which the seed then
/// leaves alone) and a stale marker is reported lost.
///
/// Returns `None` when there is nothing to tell and nothing was rendered.
pub fn deliver_rules_at_startup(db: &AppDb, claude_dir: Option<&Path>) -> Option<RulesRefresh> {
    let mut report = RulesRefresh::default();
    let repos = match db.list_repos_with_local_path(None) {
        Ok(v) => v,
        Err(e) => {
            report.errors.push(format!("repository list: {}", e));
            Vec::new()
        }
    };
    report.absorb(collect_approved_skill_edits(db, &repos, None));

    let seeded = match crate::template_seeder::seed_bundled_templates(db) {
        Ok(n) => n,
        Err(e) => {
            report.errors.push(format!("template seeding: {}", e));
            0
        }
    };
    let render = seeded > 0 || !report.taken.is_empty() || !report.lost.is_empty();
    if render {
        render_scope(
            db,
            claude_dir,
            RenderScope::UptakeOnly,
            true,
            &repos,
            &mut report,
        );
    }
    if render || !report.errors.is_empty() {
        Some(report)
    } else {
        None
    }
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
        assert!(
            dir.join("README.md").exists(),
            "non-sdh file left untouched"
        );
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

    #[test]
    fn test_render_skips_write_when_content_is_identical() {
        // T-000203: a rule save now renders across the whole portfolio, so an
        // unchanged skill must not be rewritten. Proven the strongest way
        // available: make the rendered file read-only and render again — a real
        // write would fail there, so a clean Ok means none was attempted.
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        render_skills_to_repo(&db, tmp.path()).unwrap();

        let f = tmp.path().join("docs/sdh_skills/sdh-alpha.md");
        let mut perms = fs::metadata(&f).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&f, perms).unwrap();

        let result = render_skills_to_repo(&db, tmp.path());

        // Restore write access before asserting, so the TempDir can be removed
        // even when the assertion below fails.
        let mut perms = fs::metadata(&f).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&f, perms).unwrap();

        assert_eq!(
            result.unwrap(),
            2,
            "identical content must not be rewritten"
        );
    }

    fn insert_repo(db: &AppDb, name: &str, local_path: &str) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO repositories (github_name, local_path) VALUES (?1, ?2)",
            rusqlite::params![name, local_path],
        )
        .unwrap();
    }

    // ── T-000205: taking an approved edit back out of a repo's copy ──────────

    /// A repo with its skill copies already rendered. Returns the `Repository`
    /// row and the `docs/sdh_skills` directory.
    fn rendered_repo(db: &AppDb, dir: &TempDir, name: &str) -> (Repository, std::path::PathBuf) {
        render_skills_to_repo(db, dir.path()).unwrap();
        let repo = db
            .insert_local_repository(&dir.path().to_string_lossy(), name, None, None)
            .unwrap();
        (repo, dir.path().join("docs/sdh_skills"))
    }

    fn hand_back(skills_dir: &Path, name: &str, text: &str, note: &str) {
        fs::write(skills_dir.join(format!("{}.md", name)), text).unwrap();
        fs::write(
            skills_dir.join(format!("{}.md{}", name, APPROVED_SUFFIX)),
            note,
        )
        .unwrap();
    }

    #[test]
    fn test_approved_edit_is_taken_up_and_the_marker_cleared() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        hand_back(&skills, "sdh-alpha", "ALPHA REVISED", "approved 2026-09-02");

        let Collected { taken, errors, .. } = collect_approved_skill_edits(&db, &[repo], None);

        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        assert_eq!(taken, vec!["sdh-alpha".to_string()]);
        assert_eq!(
            db.get_template_file("_global", "skill.sdh-alpha.md.tmpl")
                .unwrap()
                .unwrap()
                .content,
            "ALPHA REVISED",
            "the handed-back text becomes the rule"
        );
        assert!(
            !skills.join("sdh-alpha.md.approved").exists(),
            "marker cleared, so the next sync does not take it up again"
        );

        // And rendering afterwards now writes the new text, not the old one —
        // which is what makes uptake-then-render the correct order.
        render_skills_to_repo(&db, tmp.path()).unwrap();
        assert_eq!(
            fs::read_to_string(skills.join("sdh-alpha.md")).unwrap(),
            "ALPHA REVISED"
        );
    }

    #[test]
    fn test_an_edited_copy_without_a_marker_is_not_taken_up() {
        // Without the marker a copy is still just an output: it gets overwritten
        // on the next render, exactly as before.
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        fs::write(skills.join("sdh-alpha.md"), "SCRIBBLE").unwrap();

        let Collected { taken, errors, .. } = collect_approved_skill_edits(&db, &[repo], None);

        assert!(taken.is_empty());
        assert!(errors.is_empty());
        render_skills_to_repo(&db, tmp.path()).unwrap();
        assert_eq!(
            fs::read_to_string(skills.join("sdh-alpha.md")).unwrap(),
            "ALPHA BODY"
        );
    }

    #[test]
    fn test_conflicting_handbacks_are_refused_and_markers_kept() {
        // Taking the last one silently would lose the other repo's rule with no
        // trace of it ever having existed.
        let db = db_with_skill();
        let one = TempDir::new().unwrap();
        let two = TempDir::new().unwrap();
        let (r1, s1) = rendered_repo(&db, &one, "one");
        let (r2, s2) = rendered_repo(&db, &two, "two");
        hand_back(&s1, "sdh-alpha", "VERSION ONE", "");
        hand_back(&s2, "sdh-alpha", "VERSION TWO", "");

        let Collected { taken, errors, .. } = collect_approved_skill_edits(&db, &[r1, r2], None);

        assert!(taken.is_empty(), "nothing is taken up on a conflict");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("different text"), "{}", errors[0]);
        assert_eq!(
            db.get_template_file("_global", "skill.sdh-alpha.md.tmpl")
                .unwrap()
                .unwrap()
                .content,
            "ALPHA BODY",
            "the rule is untouched"
        );
        assert!(s1.join("sdh-alpha.md.approved").exists());
        assert!(s2.join("sdh-alpha.md.approved").exists());
    }

    #[test]
    fn test_two_repos_handing_back_identical_text_is_not_a_conflict() {
        let db = db_with_skill();
        let one = TempDir::new().unwrap();
        let two = TempDir::new().unwrap();
        let (r1, s1) = rendered_repo(&db, &one, "one");
        let (r2, s2) = rendered_repo(&db, &two, "two");
        hand_back(&s1, "sdh-beta", "AGREED TEXT", "");
        hand_back(&s2, "sdh-beta", "AGREED TEXT", "");

        let Collected { taken, errors, .. } = collect_approved_skill_edits(&db, &[r1, r2], None);

        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        assert_eq!(taken, vec!["sdh-beta".to_string()]);
        assert!(!s1.join("sdh-beta.md.approved").exists());
        assert!(!s2.join("sdh-beta.md.approved").exists());
    }

    #[test]
    fn test_a_flag_on_an_unknown_skill_is_refused() {
        // A repo copy cannot invent a rule: the name has to be one the bundle
        // already knows, otherwise a stray file would become portfolio canon.
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        hand_back(&skills, "sdh-invented", "BRAND NEW RULE", "");

        let Collected { taken, errors, .. } = collect_approved_skill_edits(&db, &[repo], None);

        assert!(taken.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("not a known skill"), "{}", errors[0]);
    }

    #[test]
    fn test_render_everywhere_covers_global_and_every_local_repo() {
        let db = db_with_skill();
        let home = TempDir::new().unwrap();
        let repo_a = TempDir::new().unwrap();
        let repo_b = TempDir::new().unwrap();
        insert_repo(&db, "acme/one", &repo_a.path().to_string_lossy());
        insert_repo(&db, "acme/two", &repo_b.path().to_string_lossy());

        let RulesRefresh {
            skills,
            repos,
            errors,
            ..
        } = refresh_rules(&db, Some(home.path()), RenderScope::Everywhere, None);

        assert_eq!(skills, 2);
        assert_eq!(repos, 2);
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        assert!(home.path().join("skills/sdh-alpha/SKILL.md").exists());
        assert!(repo_a.path().join("docs/sdh_skills/sdh-beta.md").exists());
        assert!(repo_b.path().join("docs/sdh_skills/sdh-beta.md").exists());
    }

    #[test]
    fn test_render_everywhere_skips_checkout_missing_on_this_machine() {
        // A stored local_path can name a clone that exists only on another
        // machine. That is normal portfolio state, not a failure: it must be
        // skipped silently instead of burying the real errors.
        let db = db_with_skill();
        let home = TempDir::new().unwrap();
        let live = TempDir::new().unwrap();
        insert_repo(&db, "acme/live", &live.path().to_string_lossy());
        insert_repo(&db, "acme/ghost", "F:/definitely/not/cloned/here");

        let RulesRefresh { repos, errors, .. } =
            refresh_rules(&db, Some(home.path()), RenderScope::Everywhere, None);

        assert_eq!(repos, 1, "only the checkout that exists is rendered");
        assert!(
            errors.is_empty(),
            "a missing checkout is not an error: {:?}",
            errors
        );
    }

    // ── T-000207: every render collects across the portfolio first ───────────

    /// Startup re-renders the whole portfolio after a seed that changed
    /// something. A hand-back waiting in any repo must be taken up first.
    #[test]
    fn test_startup_after_seed_collects_before_it_renders() {
        let tmp = TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        crate::template_seeder::seed_bundled_templates(&db).unwrap();
        let home = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();
        let (_, skills) = rendered_repo(&db, &repo_dir, "pending");
        let copy = skills.join("sdh-debugging.md");
        let edited = format!("{}\n\nHANDED BACK\n", fs::read_to_string(&copy).unwrap());
        hand_back(&skills, "sdh-debugging", &edited, "");
        // A new build: one bundled file is missing from the DB, so the seed
        // reports a change and the startup delivery runs.
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM templates WHERE language_key = '_global' AND file_name = 'todo.md.tmpl'",
                [],
            )
            .unwrap();

        let report = deliver_rules_at_startup(&db, Some(home.path()));

        assert!(
            report.is_some(),
            "the seed changed something, so delivery ran"
        );
        assert_eq!(
            db.get_template_file("_global", "skill.sdh-debugging.md.tmpl")
                .unwrap()
                .unwrap()
                .content,
            edited,
            "the pending edit became the rule"
        );
        assert_eq!(fs::read_to_string(&copy).unwrap(), edited);
        assert!(!skills.join("sdh-debugging.md.approved").exists());
    }

    /// T-000207 follow-up: a marker left over from the bug sits beside a copy
    /// equal to the OLD canonical text, and this launch's seed brings new bundle
    /// text. Collecting after the seed would take the stale copy up, flag it
    /// custom and silently revert the bundle update. It must be reported lost.
    #[test]
    fn test_startup_stale_marker_with_a_changed_bundle_is_lost_and_the_bundle_kept() {
        let tmp = TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        crate::template_seeder::seed_bundled_templates(&db).unwrap();
        // The previous build's text, as the database held it before upgrading.
        db.upsert_template_file(
            "_global",
            "skill.sdh-debugging.md.tmpl",
            "OLD CANONICAL",
            false,
        )
        .unwrap();
        let home = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();
        let (_, skills) = rendered_repo(&db, &repo_dir, "stale");
        fs::write(skills.join("sdh-debugging.md.approved"), "").unwrap();

        let report = deliver_rules_at_startup(&db, Some(home.path()))
            .expect("the seed changed something, so there is a report");

        let bundle =
            crate::template_seeder::bundled_file_content("_global", "skill.sdh-debugging.md.tmpl")
                .unwrap();
        let row = db
            .get_template_file("_global", "skill.sdh-debugging.md.tmpl")
            .unwrap()
            .unwrap();
        assert_eq!(row.content, bundle, "the bundle update is kept");
        assert!(
            !row.is_custom,
            "a stale marker must not mark the rule as a fork"
        );
        assert!(report.taken.is_empty(), "{:?}", report.taken);
        assert_eq!(report.lost, vec!["sdh-debugging (stale)".to_string()]);
        assert!(!skills.join("sdh-debugging.md.approved").exists());
        assert_eq!(
            fs::read_to_string(skills.join("sdh-debugging.md")).unwrap(),
            bundle,
            "and the copy now carries the new bundle text"
        );
    }

    /// T-000207 follow-up: the startup collection runs on every launch, not
    /// only when the seed changed something — a pending hand-back is taken up.
    #[test]
    fn test_startup_collects_even_when_the_bundle_is_unchanged() {
        let tmp = TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        crate::template_seeder::seed_bundled_templates(&db).unwrap();
        let home = TempDir::new().unwrap();
        let repo_dir = TempDir::new().unwrap();
        let (_, skills) = rendered_repo(&db, &repo_dir, "pending");
        let copy = skills.join("sdh-retro.md");
        let edited = format!("{}\n\nHANDED BACK\n", fs::read_to_string(&copy).unwrap());
        hand_back(&skills, "sdh-retro", &edited, "");

        let report = deliver_rules_at_startup(&db, Some(home.path()));

        assert_eq!(
            db.get_template_file("_global", "skill.sdh-retro.md.tmpl")
                .unwrap()
                .unwrap()
                .content,
            edited
        );
        assert!(!skills.join("sdh-retro.md.approved").exists());
        assert_eq!(report.map(|r| r.taken), Some(vec!["sdh-retro".to_string()]));
    }

    /// Nothing pending and an unchanged bundle: a normal launch renders nothing.
    #[test]
    fn test_startup_with_nothing_to_do_renders_nothing() {
        let tmp = TempDir::new().unwrap();
        let db = AppDb::new(tmp.path().join("test.db")).unwrap();
        crate::template_seeder::seed_bundled_templates(&db).unwrap();
        let home = TempDir::new().unwrap();

        let report = deliver_rules_at_startup(&db, Some(home.path()));

        assert!(report.is_none());
        assert!(!home.path().join("skills").exists());
    }

    /// End-to-end guard: the REAL bundle (via seed_bundled_templates) must carry
    /// exactly the eighteen sdh-* workflow skills, each with valid frontmatter, and
    /// they must render to a repo's docs/sdh_skills/. Catches a missing/renamed
    /// or malformed skill template at test time instead of at app runtime.
    #[test]
    fn test_real_bundle_renders_all_eighteen_skills() {
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
            "sdh-codex-consume",
            "sdh-codex-maintain",
            "sdh-agent-collaboration",
        ] {
            assert!(
                names.contains(&expected),
                "bundle missing skill {}",
                expected
            );
        }
        assert_eq!(
            skills.len(),
            18,
            "exactly eighteen sdh-* skills in the bundle"
        );

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
        assert_eq!(n, 18);
        assert!(repo
            .path()
            .join("docs/sdh_skills/sdh-cross-repo-req-send.md")
            .exists());
    }
}
