// Renders SDH workflow-rule skills from bundled `_global/skill.<name>.md.tmpl`
// templates to two targets: user-level Claude Code skills
// (`~/.claude/skills/<name>/SKILL.md`) and per-repo reference copies
// (`<repo>/docs/sdh_skills/<name>.md`). Single source -> two render targets.

use crate::db::{rule_event, AppDb};
use crate::models::{lookup, Repository, RulesRefresh, SyncIssue, SyncIssueCode};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const SKILL_PREFIX: &str = "skill.";
const SKILL_SUFFIX: &str = ".md.tmpl";
/// Namespace prefix for skills this app owns. Only `sdh-*` entries are ever
/// pruned — user-authored skills sharing the directory are never touched.
const SKILL_NAMESPACE: &str = "sdh-";

/// The skill a template file name stands for: `skill.<name>.md.tmpl` → `<name>`.
/// `None` for any other file, including the degenerate `skill.md.tmpl` whose
/// prefix and suffix overlap and which therefore names no skill at all.
pub fn skill_name_of(file_name: &str) -> Option<&str> {
    file_name
        .strip_prefix(SKILL_PREFIX)?
        .strip_suffix(SKILL_SUFFIX)
        .filter(|name| !name.is_empty())
}

/// The `_global` template of the global CLAUDE.md section, and the rule name it
/// goes by in the rule history.
pub const GLOBAL_SECTION_FILE: &str = "claude.md.global.tmpl";
pub const GLOBAL_SECTION_RULE: &str = "claude.md.global";

/// T-000174: the rule a `_global` template file stands for — a skill name, or
/// `claude.md.global` for the global CLAUDE.md section. `None` for every other
/// template. The single place that decides "this file is a rule": its history,
/// its reset and what the frontend lists as a rule all follow from it.
pub fn rule_name_of(file_name: &str) -> Option<&str> {
    skill_name_of(file_name)
        .or_else(|| (file_name == GLOBAL_SECTION_FILE).then_some(GLOBAL_SECTION_RULE))
}

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
        if let Some(name) = skill_name_of(&f.file_name) {
            out.push((name.to_string(), f.content));
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

/// Outcome of rendering one repo's `docs/sdh_skills/`.
#[derive(Debug, Default)]
pub struct RepoRender {
    /// Skills rendered (written, or already identical).
    pub written: usize,
    /// Skills whose copy was left as is because an approval marker still sits
    /// beside it — an unresolved hand-back.
    pub held: Vec<String>,
}

impl RepoRender {
    /// The held copies as issues, keyed by `label` (the repo).
    pub fn held_issues(&self, label: &str) -> Vec<SyncIssue> {
        self.held
            .iter()
            .map(|name| SyncIssue::new(SyncIssueCode::RuleHeld, label).target(name.clone()))
            .collect()
    }
}

/// T-000198: the repo label of a held rule copy. Both renders of a project
/// sync report held copies, and the pass keeps one line per distinct issue, so
/// the label must be one both use AND unambiguous: the full `owner/name`, or
/// the checkout path for a local-only repo — never the last segment alone,
/// which two owners can share.
pub fn rule_copy_label(repo: &Repository) -> String {
    repo.github_name
        .clone()
        .or_else(|| repo.local_path.clone())
        .unwrap_or_else(|| repo.display_name())
}

/// Render each skill to `<repo_base>/docs/sdh_skills/<name>.md` (flat reference
/// copy for non-Claude-Code agents).
///
/// T-000207: a copy with `<name>.md.approved` beside it is NOT written (nor
/// pruned). Every render runs after the collection, so a marker that survived
/// it is unresolved — a conflict, or a text that could not be stored — and its
/// copy is the only place the agent's edit still exists. Overwriting it would
/// make the next pass report the edit "lost". The skip is returned in `held`.
pub fn render_skills_to_repo(db: &AppDb, repo_base: &Path) -> Result<RepoRender, String> {
    let skills = list_skill_templates(db)?;
    let keep: HashSet<String> = skills.iter().map(|(name, _)| name.clone()).collect();
    let dir = repo_base.join("docs").join("sdh_skills");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut out = RepoRender::default();
    for (name, content) in &skills {
        if has_marker(&dir, name) {
            out.held.push(name.clone());
            continue;
        }
        write_if_changed(&dir.join(format!("{}.md", name)), content)?;
        out.written += 1;
    }
    // T-000149: drop stale `sdh-*.md` reference copies no longer in the bundle.
    prune_stale_repo_skills(&dir, &keep)?;
    Ok(out)
}

/// Whether `<dir>/<name>.md.approved` exists.
fn has_marker(dir: &Path, name: &str) -> bool {
    dir.join(format!("{}.md{}", name, APPROVED_SUFFIX)).exists()
}

/// Remove an approval marker. Already gone counts as removed: a concurrent pass
/// (or the owner) may have cleared it, and the outcome is the same.
fn clear_marker(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Serialises every collect-then-render pass in the process. The auto-sync
/// timer, a manual sync and a Settings save can otherwise interleave: one pass
/// stores a hand-back and is about to clear its marker while another already
/// sees a copy equal to the new rule and reports the edit "lost".
static RULES_LOCK: Mutex<()> = Mutex::new(());

fn rules_guard() -> std::sync::MutexGuard<'static, ()> {
    // A panic in another pass leaves no state behind this lock worth refusing over.
    RULES_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    errors: Vec<SyncIssue>,
}

impl Collected {
    /// T-000174: append an outcome to the rule's history. A failure is one
    /// more `RuleFailed` issue, never a reason to stop the pass.
    fn record(&mut self, db: &AppDb, rule: &str, kind: &str, repo: &str, note: Option<&str>) {
        if let Err(e) = db.insert_rule_event(rule, kind, Some(repo), note) {
            self.errors.push(
                SyncIssue::new(SyncIssueCode::RuleFailed, rule)
                    .target(format!("{}: rule_events", repo))
                    .detail(e),
            );
        }
    }
}

/// One repo's handed-back copy of a skill, as found beside its marker.
#[derive(Debug)]
struct HandedCopy {
    /// `rule_copy_label` of the repo.
    label: String,
    content: String,
    marker: PathBuf,
    /// The marker's approval line, trimmed; `None` when the marker is empty.
    note: Option<String>,
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
/// Lost and superseded entries read `<skill> (<repo>)`, the repo labelled by
/// `rule_copy_label` (full `owner/name`, or the path of a local-only repo).
///
/// T-000174: every outcome that is not an error is also written to the rule's
/// history (`rule_events`) — `taken` once per repo whose copy was taken, with
/// the marker's approval line, plus `lost` and `superseded` with their repo.
/// The report above is a toast that is gone in seconds; the history is what
/// answers "where did this rule's text come from" afterwards. A failed history
/// write is reported as `RuleFailed` like any other step here and never undoes
/// or blocks the outcome it describes.
fn collect_approved_skill_edits(
    db: &AppDb,
    repos: &[Repository],
    settings_edit: Option<&str>,
) -> Collected {
    let mut out = Collected::default();
    let known: HashMap<String, String> = match list_skill_templates(db) {
        Ok(v) => v.into_iter().collect(),
        Err(e) => {
            out.errors
                .push(SyncIssue::lookup(lookup::SKILLS, "").detail(e));
            return out;
        }
    };

    // skill name -> handed-back copies, one per repo that flagged it.
    let mut handed: HashMap<String, Vec<HandedCopy>> = HashMap::new();

    for repo in repos {
        let Some(path) = repo.local_path.as_deref() else {
            continue;
        };
        let dir = Path::new(path).join("docs").join("sdh_skills");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue; // no copies here yet — nothing can have been handed back
        };
        // T-000198: one unambiguous label for every finding of this pass —
        // issues and the `<skill> (<repo>)` lines alike.
        let label = rule_copy_label(repo);
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
                out.errors.push(
                    SyncIssue::new(SyncIssueCode::RuleUnknownSkill, label.clone()).target(marker),
                );
                continue;
            };
            if settings_edit == Some(name) {
                match clear_marker(&entry.path()) {
                    Ok(()) => {
                        out.superseded.push(format!("{} ({})", name, label));
                        out.record(db, name, rule_event::SUPERSEDED, &label, None);
                    }
                    Err(e) => out.errors.push(
                        SyncIssue::new(SyncIssueCode::RuleFailed, name)
                            .target(format!("{}: {}", label, marker))
                            .detail(e),
                    ),
                }
                continue;
            }
            let copy = dir.join(stem);
            match fs::read_to_string(&copy) {
                Ok(content) if content == *current => match clear_marker(&entry.path()) {
                    Ok(()) => {
                        out.lost.push(format!("{} ({})", name, label));
                        out.record(db, name, rule_event::LOST, &label, None);
                    }
                    Err(e) => out.errors.push(
                        SyncIssue::new(SyncIssueCode::RuleFailed, name)
                            .target(format!("{}: {}", label, marker))
                            .detail(e),
                    ),
                },
                Ok(content) => {
                    // The marker may carry the approval line. Empty is allowed;
                    // when present it is worth having in the log and, since
                    // T-000174, in the rule's history.
                    let note = fs::read_to_string(entry.path())
                        .ok()
                        .map(|note| note.trim().to_string())
                        .filter(|note| !note.is_empty());
                    if let Some(note) = &note {
                        eprintln!("[skills] {} hands back {}: {}", label, name, note);
                    }
                    handed
                        .entry(name.to_string())
                        .or_default()
                        .push(HandedCopy {
                            label: label.clone(),
                            content,
                            marker: entry.path(),
                            note,
                        });
                }
                Err(e) => out.errors.push(
                    SyncIssue::new(SyncIssueCode::RuleFailed, name)
                        .target(format!("{}: {}", label, stem))
                        .detail(e),
                ),
            }
        }
    }

    for (name, copies) in handed {
        let first = &copies[0].content;
        if let Some(other) = copies.iter().find(|copy| copy.content != *first) {
            out.errors.push(
                SyncIssue::new(SyncIssueCode::VersionConflict, name.clone())
                    .target(format!("{}, {}", copies[0].label, other.label)),
            );
            continue; // markers stay put: nobody's edit is thrown away
        }
        let file_name = format!("{}{}{}", SKILL_PREFIX, name, SKILL_SUFFIX);
        if let Err(e) = db.upsert_template_file("_global", &file_name, first, true) {
            out.errors.push(
                SyncIssue::new(SyncIssueCode::RuleFailed, name.clone())
                    .target(file_name.clone())
                    .detail(e),
            );
            continue;
        }
        for copy in &copies {
            // Recorded before the marker is cleared, and whether or not that
            // succeeds: the text is already the rule at this point.
            out.record(
                db,
                &name,
                rule_event::TAKEN,
                &copy.label,
                copy.note.as_deref(),
            );
            let marker = &copy.marker;
            if let Err(e) = clear_marker(marker) {
                // The text is already stored; a surviving marker now sits beside
                // a copy equal to the rule, so the next pass reports it as lost
                // instead of taking it up again.
                out.errors.push(
                    SyncIssue::new(SyncIssueCode::RuleFailed, name.clone())
                        .target(marker.display().to_string())
                        .detail(e),
                );
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
    let _guard = rules_guard();
    let mut report = RulesRefresh::default();
    let repos = match db.list_repos_with_local_path(None) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(SyncIssue::lookup(lookup::REPOS, "").detail(e));
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
                // Already rendered (and its held copies reported) by the
                // everywhere pass when the target is a stored checkout.
                Ok(_) if everywhere => {}
                Ok(r) => {
                    report.repos = 1;
                    report
                        .errors
                        .extend(r.held_issues(&base.display().to_string()));
                }
                Err(e) => report.errors.push(
                    SyncIssue::new(SyncIssueCode::ManagedFileFailed, base.display().to_string())
                        .target("docs/sdh_skills/")
                        .detail(e),
                ),
            }
        }
        RenderScope::Global => match claude_dir.map(|dir| render_skills_global(db, dir)) {
            Some(Ok(n)) => report.skills = n,
            Some(Err(e)) => report.errors.push(claude_skills_issue(e)),
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
) -> (usize, usize, Vec<SyncIssue>) {
    let mut errors = Vec::new();
    let skills = match claude_dir.map(|dir| render_skills_global(db, dir)) {
        Some(Ok(n)) => n,
        Some(Err(e)) => {
            errors.push(claude_skills_issue(e));
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
        // T-000198: the same label the project-sync skeleton pass uses, so a
        // held copy seen by both renders of one pass is reported once.
        let label = rule_copy_label(repo);
        match render_skills_to_repo(db, base) {
            Ok(r) => {
                touched += 1;
                errors.extend(r.held_issues(&label));
            }
            Err(e) => errors.push(
                SyncIssue::new(SyncIssueCode::ManagedFileFailed, label)
                    .target("docs/sdh_skills/")
                    .detail(e),
            ),
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
    let _guard = rules_guard();
    let mut report = RulesRefresh::default();
    let repos = match db.list_repos_with_local_path(None) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(SyncIssue::lookup(lookup::REPOS, "").detail(e));
            Vec::new()
        }
    };
    report.absorb(collect_approved_skill_edits(db, &repos, None));

    let seeded = match crate::template_seeder::seed_bundled_templates(db) {
        Ok(n) => n,
        Err(e) => {
            report
                .errors
                .push(SyncIssue::lookup(lookup::SEEDING, "").detail(e));
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

/// A failed render of the user-level skills.
fn claude_skills_issue(e: impl std::fmt::Display) -> SyncIssue {
    SyncIssue::new(SyncIssueCode::ManagedFileFailed, "~/.claude")
        .target("skills/")
        .detail(e)
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
            // A flagged copy is an unresolved hand-back (here: of a name the
            // bundle does not know) — the agent's text, not a stale output.
            if stem.starts_with(SKILL_NAMESPACE) && !keep.contains(stem) && !has_marker(dir, stem) {
                fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
                pruned += 1;
            }
        }
    }
    Ok(pruned)
}

/// Test support shared by every module that exercises a hand-back.
#[cfg(test)]
pub(crate) mod test_support {
    use super::APPROVED_SUFFIX;
    use std::fs;
    use std::path::Path;

    /// Overwrite the copy of `name` in `skills_dir` with `text` and drop the
    /// approval marker beside it, carrying `note` — what a handing-back agent does.
    pub(crate) fn hand_back(skills_dir: &Path, name: &str, text: &str, note: &str) {
        fs::write(skills_dir.join(format!("{}.md", name)), text).unwrap();
        fs::write(
            skills_dir.join(format!("{}.md{}", name, APPROVED_SUFFIX)),
            note,
        )
        .unwrap();
    }

    /// Append a line naming `repo` to its rendered copy of `name` and hand it
    /// back. Returns the edited text — distinct per repo, so two repos handing
    /// back the same skill this way conflict.
    pub(crate) fn hand_back_edit(repo: &Path, name: &str) -> String {
        let skills = repo.join("docs").join("sdh_skills");
        let current = fs::read_to_string(skills.join(format!("{}.md", name))).unwrap();
        let edited = format!("{}\n\nHANDED BACK FROM {}\n", current, repo.display());
        hand_back(&skills, name, &edited, "");
        edited
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
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
    fn test_skill_name_of() {
        assert_eq!(skill_name_of("skill.sdh-retro.md.tmpl"), Some("sdh-retro"));
        assert_eq!(skill_name_of("skill.md.tmpl"), None, "affixes overlap");
        assert_eq!(skill_name_of("skill..md.tmpl"), None, "empty name");
        assert_eq!(skill_name_of(".gitignore.tmpl"), None);
    }

    #[test]
    fn test_rule_name_of() {
        assert_eq!(rule_name_of("skill.sdh-retro.md.tmpl"), Some("sdh-retro"));
        assert_eq!(
            rule_name_of("claude.md.global.tmpl"),
            Some("claude.md.global")
        );
        assert_eq!(rule_name_of("claude.md.section.tmpl"), None);
        assert_eq!(rule_name_of(".gitignore.tmpl"), None);
        assert_eq!(rule_name_of("skill.md.tmpl"), None);
    }

    #[test]
    fn test_clearing_a_marker_that_is_already_gone_is_not_an_error() {
        let tmp = TempDir::new().unwrap();
        assert!(clear_marker(&tmp.path().join("sdh-alpha.md.approved")).is_ok());
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
        let n = render_skills_to_repo(&db, tmp.path()).unwrap().written;
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
        let n = render_skills_to_repo(&db, tmp.path()).unwrap().written;
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
            result.unwrap().written,
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
        assert_eq!(
            errors[0].code,
            SyncIssueCode::VersionConflict,
            "{}",
            errors[0]
        );
        assert_eq!(errors[0].subject, "sdh-alpha");
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

    /// Two GitHub repos of different owners sharing a last segment, both with
    /// their copies rendered. Returns their `docs/sdh_skills` dirs.
    fn same_named_repos(db: &AppDb, one: &TempDir, two: &TempDir) -> (PathBuf, PathBuf) {
        insert_repo(db, "a/foo", &one.path().to_string_lossy());
        insert_repo(db, "b/foo", &two.path().to_string_lossy());
        render_skills_to_repo(db, one.path()).unwrap();
        render_skills_to_repo(db, two.path()).unwrap();
        (
            one.path().join("docs/sdh_skills"),
            two.path().join("docs/sdh_skills"),
        )
    }

    /// T-000198 review: the collection labels repos unambiguously, so the same
    /// refusal in `a/foo` and `b/foo` stays two issues after the per-pass dedupe.
    #[test]
    fn test_unknown_skill_flags_in_same_named_repos_stay_distinct() {
        let db = db_with_skill();
        let one = TempDir::new().unwrap();
        let two = TempDir::new().unwrap();
        let (s1, s2) = same_named_repos(&db, &one, &two);
        hand_back(&s1, "sdh-invented", "X", "");
        hand_back(&s2, "sdh-invented", "X", "");
        let repos = db.list_repos_with_local_path(None).unwrap();

        let Collected { errors, .. } = collect_approved_skill_edits(&db, &repos, None);

        let mut subjects: Vec<&str> = errors.iter().map(|e| e.subject.as_str()).collect();
        subjects.sort();
        assert_eq!(subjects, vec!["a/foo", "b/foo"], "{:?}", errors);
    }

    /// T-000198 review: a marker that cannot be cleared names the repo it sits
    /// in. A directory under the marker's name makes `remove_file` fail on
    /// every platform, without a test seam.
    #[test]
    fn test_a_stuck_marker_names_its_repo() {
        let db = db_with_skill();
        let one = TempDir::new().unwrap();
        let two = TempDir::new().unwrap();
        let (s1, s2) = same_named_repos(&db, &one, &two);
        // A marker beside an unchanged copy: the "lost" path, which clears it.
        fs::create_dir(s1.join("sdh-alpha.md.approved")).unwrap();
        fs::create_dir(s2.join("sdh-alpha.md.approved")).unwrap();
        let repos = db.list_repos_with_local_path(None).unwrap();

        let Collected { errors, .. } = collect_approved_skill_edits(&db, &repos, None);

        let mut targets: Vec<&str> = errors.iter().filter_map(|e| e.target.as_deref()).collect();
        targets.sort();
        assert_eq!(
            targets,
            vec![
                "a/foo: sdh-alpha.md.approved",
                "b/foo: sdh-alpha.md.approved"
            ],
            "{:?}",
            errors
        );
        assert!(errors.iter().all(|e| e.code == SyncIssueCode::RuleFailed));
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
        assert_eq!(
            errors[0].code,
            SyncIssueCode::RuleUnknownSkill,
            "{}",
            errors[0]
        );
        assert_eq!(
            errors[0].target.as_deref(),
            Some("sdh-invented.md.approved")
        );
    }

    // ── T-000174: the collection leaves a history behind ─────────────────────

    /// `(kind, repo, note)` of a rule's history, oldest first.
    fn history(db: &AppDb, rule: &str) -> Vec<(String, Option<String>, Option<String>)> {
        let mut events: Vec<_> = db
            .list_rule_events(rule)
            .unwrap()
            .into_iter()
            .map(|e| (e.kind, e.repo, e.note))
            .collect();
        events.reverse();
        events
    }

    /// One `taken` per repo whose copy was taken, carrying that repo's approval
    /// line — the toast that used to be the only trace is gone in seconds.
    #[test]
    fn test_an_uptake_records_taken_per_repo_with_the_marker_note() {
        let db = db_with_skill();
        let one = TempDir::new().unwrap();
        let two = TempDir::new().unwrap();
        let (r1, s1) = rendered_repo(&db, &one, "one");
        let (r2, s2) = rendered_repo(&db, &two, "two");
        hand_back(
            &s1,
            "sdh-beta",
            "AGREED TEXT",
            "  approved in chat 2026-10-19
",
        );
        hand_back(&s2, "sdh-beta", "AGREED TEXT", "");
        let (l1, l2) = (rule_copy_label(&r1), rule_copy_label(&r2));

        let Collected { taken, errors, .. } = collect_approved_skill_edits(&db, &[r1, r2], None);

        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        assert_eq!(taken, vec!["sdh-beta".to_string()]);
        let mut events = history(&db, "sdh-beta");
        events.sort();
        let mut expected = vec![
            (
                "taken".to_string(),
                Some(l1),
                Some("approved in chat 2026-10-19".to_string()),
            ),
            ("taken".to_string(), Some(l2), None),
        ];
        expected.sort();
        assert_eq!(events, expected);
        assert!(
            history(&db, "sdh-alpha").is_empty(),
            "nothing else recorded"
        );
    }

    #[test]
    fn test_a_lost_edit_is_recorded_with_its_repo() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        let label = rule_copy_label(&repo);
        // A marker beside an unchanged copy: the edit never reached the rule.
        fs::write(skills.join("sdh-alpha.md.approved"), "approved").unwrap();

        let Collected { lost, .. } = collect_approved_skill_edits(&db, &[repo], None);

        assert_eq!(lost.len(), 1);
        assert_eq!(
            history(&db, "sdh-alpha"),
            vec![("lost".to_string(), Some(label), None)]
        );
    }

    #[test]
    fn test_a_superseded_handback_is_recorded_with_its_repo() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        let label = rule_copy_label(&repo);
        hand_back(&skills, "sdh-alpha", "AGENT TEXT", "approved");

        let Collected {
            superseded, taken, ..
        } = collect_approved_skill_edits(&db, &[repo], Some("sdh-alpha"));

        assert_eq!(superseded.len(), 1);
        assert!(taken.is_empty());
        assert_eq!(
            history(&db, "sdh-alpha"),
            vec![("superseded".to_string(), Some(label), None)]
        );
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
        let edited = hand_back_edit(repo_dir.path(), "sdh-debugging");
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
        assert_eq!(
            report.lost,
            vec![format!(
                "sdh-debugging ({})",
                repo_dir.path().to_string_lossy()
            )]
        );
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
        let edited = hand_back_edit(repo_dir.path(), "sdh-retro");

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
            "sdh-project-start",
        ] {
            assert!(
                names.contains(&expected),
                "bundle missing skill {}",
                expected
            );
        }
        assert_eq!(
            skills.len(),
            19,
            "exactly nineteen sdh-* skills in the bundle"
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
        let n = render_skills_to_repo(&db, repo.path()).unwrap().written;
        assert_eq!(n, 19);
        assert!(repo
            .path()
            .join("docs/sdh_skills/sdh-cross-repo-req-send.md")
            .exists());
    }
}
