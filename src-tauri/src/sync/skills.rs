// Renders SDH workflow-rule skills from bundled `_global/skill.<name>.md.tmpl`
// templates to two targets: user-level Claude Code skills
// (`~/.claude/skills/<name>/SKILL.md`) and per-repo reference copies
// (`<repo>/docs/sdh_skills/<name>.md`). Single source -> two render targets.

use crate::db::{conflict_note, rule_event, AppDb};
use crate::models::{lookup, Repository, RuleConflictSide, RulesRefresh, SyncIssue, SyncIssueCode};
use std::collections::{BTreeSet, HashMap, HashSet};
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
    ///
    /// T-000206: copies of a skill in `quiet` (`RulesRefresh::quiet_conflicts`)
    /// are left out. Their conflict was reported, held copies included, by the
    /// pass that opened it; repeating them on every sync is the noise this
    /// filter exists to stop. Every reader of held copies goes through here.
    pub fn held_issues(&self, label: &str, quiet: &[String]) -> Vec<SyncIssue> {
        self.held
            .iter()
            .filter(|name| !quiet.contains(name))
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

/// F-000044 / Р16: in a shared repo where this install is a contributor or a
/// pending helper, the rule copies under `docs/sdh_skills/` are the OWNER's:
/// he renders and commits them, this install only receives them through git.
/// Rendering its own copies there leaves them untracked, and the owner's
/// committed ones then cannot be fast-forwarded in (git refuses to overwrite
/// untracked files). So such a repo is left out of every pass that renders
/// copies or collects hand-back markers; the owner role and ordinary repos are
/// unchanged.
pub fn skips_rule_copies(db: &AppDb, repo_id: i64) -> bool {
    db.get_shared_state(repo_id)
        .map(|s| {
            matches!(
                s.shared_role.as_deref(),
                Some("contributor") | Some("pending")
            )
        })
        .unwrap_or(false)
}

/// Every repo with a local path that takes part in rule copies: the portfolio
/// minus the contributor / pending shared repos (`skips_rule_copies`).
fn rules_repos(db: &AppDb) -> rusqlite::Result<Vec<Repository>> {
    let mut repos = db.list_repos_with_local_path(None)?;
    repos.retain(|r| !skips_rule_copies(db, r.id));
    Ok(repos)
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

/// T-000206: `<dir>/<name>.md.conflict`, the reason file of `name`.
fn conflict_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.md{}", name, CONFLICT_SUFFIX))
}

/// Remove a file this app placed or consumes — an approval marker, a reason
/// file. Already gone counts as removed: a concurrent pass (or the owner) may
/// have cleared it, and the outcome is the same.
fn remove_if_present(path: &Path) -> io::Result<()> {
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

/// Take the rules lock. `pub(crate)` for a caller that must run its own steps
/// and a `refresh_rules_locked` under ONE hold of it (T-000206: resolving a
/// conflict); the lock is not re-entrant, so taking it twice deadlocks.
pub(crate) fn rules_guard() -> std::sync::MutexGuard<'static, ()> {
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

/// T-000206: reason file dropped beside a copy whose hand-back is in conflict,
/// `<name>.md.conflict`, telling the agent in that repo why its edit was not
/// taken. Like the marker it goes AFTER `.md`, so neither the prune pass (which
/// only touches `*.md`) nor the collection (which only reads `*.approved`)
/// ever mistakes it for a copy or a marker.
const CONFLICT_SUFFIX: &str = ".conflict";

/// T-000206: separator of the repo labels of a conflict — in its `conflict`
/// event and in the `RuleConflict` target. For display only: the sides a
/// conflict is decided on are its stored repo ids, never these labels.
const CONFLICT_REPOS_SEPARATOR: &str = ", ";

/// One approval marker found in a repo's `docs/sdh_skills/`.
#[derive(Debug, Clone)]
pub(crate) struct FoundMarker {
    /// The skill it flags: `<name>` of `<name>.md.approved`.
    pub name: String,
    /// The copy it sits beside: `<name>.md`.
    pub stem: String,
    /// The marker file itself.
    pub path: PathBuf,
}

impl FoundMarker {
    /// The marker's file name, `<name>.md.approved`.
    fn file_name(&self) -> String {
        format!("{}{}", self.stem, APPROVED_SUFFIX)
    }
}

/// One repo's `docs/sdh_skills/` as read in one pass.
#[derive(Debug)]
pub(crate) struct RepoScan {
    pub repo_id: i64,
    /// `rule_copy_label` of the repo.
    pub label: String,
    pub dir: PathBuf,
    pub markers: Vec<FoundMarker>,
}

/// T-000206: the approval markers of one repo — pure I/O, nothing written,
/// cleared or recorded, so it can also serve a read-only look at a conflict.
/// `None` when the repo has no local path or its `docs/sdh_skills/` cannot be
/// read (no copies yet, or a checkout missing on this machine): nothing in it
/// was seen this pass.
pub(crate) fn scan_repo_markers(repo: &Repository) -> Option<RepoScan> {
    let path = repo.local_path.as_deref()?;
    let dir = Path::new(path).join("docs").join("sdh_skills");
    let entries = fs::read_dir(&dir).ok()?;
    let mut markers = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(stem) = file_name.strip_suffix(APPROVED_SUFFIX) else {
            continue;
        };
        let Some(name) = stem.strip_suffix(".md") else {
            continue;
        };
        markers.push(FoundMarker {
            name: name.to_string(),
            stem: stem.to_string(),
            path: entry.path(),
        });
    }
    markers.sort_by(|a, b| a.path.cmp(&b.path));
    Some(RepoScan {
        repo_id: repo.id,
        // T-000198: one unambiguous label for every finding of this pass —
        // issues and the `<skill> (<repo>)` lines alike.
        label: rule_copy_label(repo),
        dir,
        markers,
    })
}

/// What one collection pass found beside the repo copies.
#[derive(Debug, Default)]
struct Collected {
    taken: Vec<String>,
    lost: Vec<String>,
    superseded: Vec<String>,
    errors: Vec<SyncIssue>,
    /// T-000206: see `RulesRefresh::quiet_conflicts`.
    quiet_conflicts: Vec<String>,
}

/// One repo's handed-back copy of a skill, as found beside its marker.
#[derive(Debug)]
struct HandedCopy {
    repo_id: i64,
    /// `rule_copy_label` of the repo.
    label: String,
    content: String,
    marker: PathBuf,
    /// The marker's approval line, trimmed; `None` when the marker is empty.
    note: Option<String>,
}

/// What the markers of one pass amount to, before anything is decided.
#[derive(Debug, Default)]
struct Found {
    /// Skill -> its handed-back copies; sorted by skill, copies by label, so
    /// reports and reason files read the same on every pass.
    handed: Vec<(String, Vec<HandedCopy>)>,
    /// Repos whose `docs/sdh_skills/` was read this pass.
    read: HashSet<i64>,
    /// `(skill, repo id)`: a flagged copy that could not be read.
    unread: HashSet<(String, i64)>,
}

/// A conflict open before this pass. `sides` is `Err` when its stored repo
/// ids do not parse.
struct OpenConflict {
    sides: Result<BTreeSet<i64>, String>,
}

/// Whether the handed-back copies of one skill disagree.
fn in_conflict(copies: &[HandedCopy]) -> bool {
    copies.iter().any(|c| c.content != copies[0].content)
}

impl Collected {
    /// T-000174: append an outcome to the rule's history. A failure is one
    /// more `RuleFailed` issue, never a reason to stop the pass.
    fn record(
        &mut self,
        db: &AppDb,
        rule: &str,
        kind: &str,
        repo: Option<&str>,
        note: Option<&str>,
    ) {
        if let Err(e) = db.insert_rule_event(rule, kind, repo, note) {
            let target = match repo {
                Some(repo) => format!("{}: rule_events", repo),
                None => "rule_events".to_string(),
            };
            self.rule_failed(rule, target, e);
        }
    }

    /// One step about `rule` failed: reported, never fatal.
    fn rule_failed(&mut self, rule: &str, target: impl Into<String>, e: impl std::fmt::Display) {
        self.errors.push(
            SyncIssue::new(SyncIssueCode::RuleFailed, rule)
                .target(target)
                .detail(e),
        );
    }

    /// Sort the per-pass lists, so a report reads the same however the
    /// filesystem happened to order the files.
    fn sorted(mut self) -> Self {
        self.taken.sort();
        self.lost.sort();
        self.superseded.sort();
        self.quiet_conflicts.sort();
        self
    }
}

/// T-000206: the conflicts open before this pass, read BEFORE scanning, so the
/// pass that opens a conflict still reports it in full and only the passes
/// after it keep quiet. A failed read is reported and leaves the map empty:
/// every live conflict then reads as new (loud, never silent) and none closes.
fn read_open_conflicts(db: &AppDb, out: &mut Collected) -> HashMap<String, OpenConflict> {
    match db.list_rule_conflicts() {
        Ok(rows) => rows
            .into_iter()
            .map(|(rule, raw)| {
                let sides = crate::db::parse_conflict_repo_ids(&raw);
                (rule, OpenConflict { sides })
            })
            .collect(),
        Err(e) => {
            out.rule_failed("", "rule_conflicts", e);
            HashMap::new()
        }
    }
}

/// Sort the markers of a pass into its outcomes. Unknown, superseded and lost
/// markers are settled (and recorded) right here; everything else is returned
/// as a hand-back for the decisions that follow.
///
/// - **unknown** — the marker names a skill the bundle does not have. A repo
///   copy cannot invent a rule.
/// - **superseded** (T-000207) — `settings_edit` names the skill the owner has
///   just saved or reset. That explicit edit wins over a pending hand-back of
///   the same skill: not taken, marker cleared, reported.
/// - **lost** (T-000207) — the copy beside the marker is byte-equal to the
///   current rule. The edit the marker stood for was overwritten before anyone
///   collected it; taking it up would report a success and flag an untouched
///   rule as a fork. Marker cleared, loss reported, rule left alone.
/// - **unread** (T-000206) — the copy beside the marker cannot be read. Kept in
///   `Found::unread`: that side was not seen, which matters for a conflict.
fn classify(
    db: &AppDb,
    scans: &[RepoScan],
    known: &HashMap<String, String>,
    settings_edit: Option<&str>,
    followed: &HashSet<String>,
    out: &mut Collected,
) -> Found {
    let mut found = Found::default();
    let mut handed: HashMap<String, Vec<HandedCopy>> = HashMap::new();
    for scan in scans {
        found.read.insert(scan.repo_id);
        let label = scan.label.as_str();
        for marker in &scan.markers {
            let name = marker.name.as_str();
            let Some(current) = known.get(name) else {
                out.errors.push(
                    SyncIssue::new(SyncIssueCode::RuleUnknownSkill, label)
                        .target(marker.file_name()),
                );
                continue;
            };
            // A rule that follows an owner (shared_canon) has its text from his
            // repo; a hand-back of it is set aside like the owner's own edit.
            if settings_edit == Some(name) || followed.contains(name) {
                match remove_if_present(&marker.path) {
                    Ok(()) => {
                        out.superseded.push(format!("{} ({})", name, label));
                        out.record(db, name, rule_event::SUPERSEDED, Some(label), None);
                    }
                    Err(e) => {
                        out.rule_failed(name, format!("{}: {}", label, marker.file_name()), e)
                    }
                }
                continue;
            }
            match fs::read_to_string(scan.dir.join(&marker.stem)) {
                Ok(content) if content == *current => match remove_if_present(&marker.path) {
                    Ok(()) => {
                        out.lost.push(format!("{} ({})", name, label));
                        out.record(db, name, rule_event::LOST, Some(label), None);
                    }
                    Err(e) => {
                        out.rule_failed(name, format!("{}: {}", label, marker.file_name()), e)
                    }
                },
                Ok(content) => {
                    // The marker may carry the approval line. Empty is allowed;
                    // when present it is worth having in the log and, since
                    // T-000174, in the rule's history.
                    let note = fs::read_to_string(&marker.path)
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
                            repo_id: scan.repo_id,
                            label: label.to_string(),
                            content,
                            marker: marker.path.clone(),
                            note,
                        });
                }
                Err(e) => {
                    found.unread.insert((name.to_string(), scan.repo_id));
                    out.rule_failed(name, format!("{}: {}", label, marker.stem), e);
                }
            }
        }
    }
    let mut handed: Vec<(String, Vec<HandedCopy>)> = handed.into_iter().collect();
    handed.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, copies) in &mut handed {
        copies.sort_by(|a, b| a.label.cmp(&b.label));
    }
    found.handed = handed;
    found
}

/// T-000206: decide every conflict that was open before this pass and is not
/// live in it. Returns the rules left **undecided**: still open, quiet, and
/// none of their hand-backs may be taken this pass (their markers stay).
///
/// "Not live" may only mean a side was not seen — a checkout missing on this
/// machine, an unreadable folder or copy, a repo left out of the pass. Closing
/// then would record a false "resolved outside the app", take the other side
/// as a lone hand-back, and let the unseen side overwrite it once it is back.
/// So a conflict closes only when every side still in the portfolio with a
/// local path was read (`all_sides_read`); the note says whether the owner's
/// own edit superseded it or it went away outside the app.
fn settle_open_conflicts(
    db: &AppDb,
    repos: &[Repository],
    open: &HashMap<String, OpenConflict>,
    found: &Found,
    settings_edit: Option<&str>,
    out: &mut Collected,
) -> HashSet<String> {
    let live: HashSet<&str> = found
        .handed
        .iter()
        .filter(|(_, copies)| in_conflict(copies))
        .map(|(name, _)| name.as_str())
        .collect();
    let mut gone: Vec<&String> = open
        .keys()
        .filter(|rule| !live.contains(rule.as_str()))
        .collect();
    gone.sort();
    let mut undecided = HashSet::new();
    for rule in gone {
        let decidable = match &open[rule].sides {
            Ok(sides) => match all_sides_read(db, rule, sides, found) {
                Ok(all) => all,
                Err(e) => {
                    out.rule_failed(rule, "repositories", e);
                    false
                }
            },
            Err(e) => {
                out.rule_failed(rule, "rule_conflicts.repo_ids", e);
                false
            }
        };
        let note = if settings_edit == Some(rule.as_str()) {
            // The owner's own save or reset set the markers aside this pass.
            conflict_note::SUPERSEDED_BY_OWNER
        } else {
            conflict_note::RESOLVED_OUTSIDE
        };
        if !(decidable && close_conflict(db, repos, rule, None, note, out)) {
            out.quiet_conflicts.push(rule.clone());
            undecided.insert(rule.clone());
        }
    }
    undecided
}

/// T-000206: whether every side of a conflict that can still hand something
/// back was read this pass. Sides are repo ids, stable across a rename. A side
/// gone from the portfolio, or whose local path was cleared, can hand nothing
/// back any more and does not block; a side with a local path that was not
/// read — or whose copy of `rule` was unreadable — does.
fn all_sides_read(
    db: &AppDb,
    rule: &str,
    sides: &BTreeSet<i64>,
    found: &Found,
) -> rusqlite::Result<bool> {
    for &id in sides {
        let repo = match db.get_repository(id) {
            Ok(repo) => repo,
            Err(rusqlite::Error::QueryReturnedNoRows) => continue,
            Err(e) => return Err(e),
        };
        if repo.local_path.is_none() || skips_rule_copies(db, id) {
            // Р16: a contributor's checkout hands nothing back, so its side
            // cannot hold a conflict open.
            continue;
        }
        if !found.read.contains(&id) || found.unread.contains(&(rule.to_string(), id)) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// T-000206: close the conflict on `name` — row removed, `conflict_resolved`
/// recorded with `repo` (the accepted side, `None` if no side was accepted)
/// and `note`, reason files removed across the portfolio. The caller picks the
/// note. Returns `false`, having recorded and removed nothing, when the row
/// could not be removed: the conflict is still open, and a history saying
/// otherwise would be false.
fn close_conflict(
    db: &AppDb,
    repos: &[Repository],
    name: &str,
    repo: Option<&str>,
    note: &str,
    out: &mut Collected,
) -> bool {
    if let Err(e) = db.close_rule_conflict(name) {
        out.rule_failed(name, "rule_conflicts", e);
        return false;
    }
    out.record(db, name, rule_event::CONFLICT_RESOLVED, repo, Some(note));
    remove_reason_files(repos, name, &HashSet::new(), out);
    true
}

/// T-000206: the hand-backs of `name` disagree.
///
/// - **open** — no conflict was open before this pass: the row is opened with
///   the repo ids of the sides, a `conflict` event names them, and one
///   `RuleConflict` reports it. This is the only pass that reports it.
/// - **quiet** — it was open already: listed in `quiet_conflicts`, no report.
///   A repo that joined since is added to the stored sides (sides only grow
///   while the conflict lives) and a new `conflict` event shows it joined.
///
/// Either way each side gets the reason file, rewritten only when its content
/// changed, and a repo that has left the conflict loses it.
fn record_conflict(
    db: &AppDb,
    repos: &[Repository],
    name: &str,
    copies: &[HandedCopy],
    open: Option<&OpenConflict>,
    out: &mut Collected,
) {
    let live: BTreeSet<i64> = copies.iter().map(|c| c.repo_id).collect();
    let labels = copies
        .iter()
        .map(|c| c.label.as_str())
        .collect::<Vec<_>>()
        .join(CONFLICT_REPOS_SEPARATOR);
    match open {
        None => {
            if let Err(e) = db.open_rule_conflict(name, &live) {
                out.rule_failed(name, "rule_conflicts", e);
            }
            out.record(db, name, rule_event::CONFLICT, Some(&labels), None);
            out.errors
                .push(SyncIssue::new(SyncIssueCode::RuleConflict, name).target(labels));
        }
        Some(OpenConflict { sides: Ok(sides) }) => {
            out.quiet_conflicts.push(name.to_string());
            if !live.is_subset(sides) {
                let all: BTreeSet<i64> = sides.union(&live).copied().collect();
                if let Err(e) = db.set_rule_conflict_repos(name, &all) {
                    out.rule_failed(name, "rule_conflicts", e);
                }
                out.record(db, name, rule_event::CONFLICT, Some(&labels), None);
            }
        }
        Some(OpenConflict { sides: Err(e) }) => {
            // Left as stored: rewriting it from this pass alone could drop a
            // side that is not seen right now. The owner decides it in the app.
            out.quiet_conflicts.push(name.to_string());
            out.rule_failed(name, "rule_conflicts.repo_ids", e);
        }
    }

    let reason = conflict_reason(name, copies);
    let mut involved = HashSet::new();
    for copy in copies {
        let Some(dir) = copy.marker.parent() else {
            continue;
        };
        involved.insert(dir.to_path_buf());
        if let Err(e) = write_if_changed(&conflict_file(dir, name), &reason) {
            out.rule_failed(
                name,
                format!("{}: {}.md{}", copy.label, name, CONFLICT_SUFFIX),
                e,
            );
        }
    }
    remove_reason_files(repos, name, &involved, out);
}

/// Remove the reason file of `name` from every repo of the portfolio except
/// the `keep` skill dirs. Missing is the normal case, not an error.
fn remove_reason_files(
    repos: &[Repository],
    name: &str,
    keep: &HashSet<PathBuf>,
    out: &mut Collected,
) {
    for repo in repos {
        let Some(path) = repo.local_path.as_deref() else {
            continue;
        };
        let dir = Path::new(path).join("docs").join("sdh_skills");
        if keep.contains(&dir) {
            continue;
        }
        if let Err(e) = remove_if_present(&conflict_file(&dir, name)) {
            out.rule_failed(
                name,
                format!("{}: {}.md{}", rule_copy_label(repo), name, CONFLICT_SUFFIX),
                e,
            );
        }
    }
}

/// T-000206: the text of the reason file — English, because the agent working
/// in that repo is its reader. It names every repo of the conflict with the
/// time its marker was dropped, so the agent can tell which edit is its own.
fn conflict_reason(name: &str, copies: &[HandedCopy]) -> String {
    let sides: String = copies
        .iter()
        .map(|copy| {
            format!(
                "- {} \u{2014} marker {}\n",
                copy.label,
                marker_time(&copy.marker)
            )
        })
        .collect();
    format!(
        "# Hand-back not taken: conflict\n\n\
         Your edited copy of `{name}` was NOT taken up.\n\
         Another repo handed back a different text for the same skill:\n\n\
         {sides}\n\
         Nothing is lost: both markers and both copies stay as they are.\n\
         The owner chooses one text in Solo Dev Hub \u{2192} Rules.\n\n\
         Do not edit this copy again, re-create the marker or delete this file.\n\
         Sync removes it once the owner has decided.\n"
    )
}

/// A marker's mtime as `YYYY-MM-DD HH:MM`, local time.
fn marker_time(marker: &Path) -> String {
    match fs::metadata(marker).and_then(|m| m.modified()) {
        Ok(t) => chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// Store the agreed text of `name` — every copy handed back the same — and
/// clear the markers it came from.
fn take_up(db: &AppDb, name: String, copies: &[HandedCopy], out: &mut Collected) {
    let text = &copies[0].content;
    let file_name = format!("{}{}{}", SKILL_PREFIX, name, SKILL_SUFFIX);
    if let Err(e) = db.upsert_template_file("_global", &file_name, text, true) {
        out.rule_failed(&name, file_name, e);
        return;
    }
    for copy in copies {
        // Recorded before the marker is cleared, and whether or not that
        // succeeds: the text is already the rule at this point.
        out.record(
            db,
            &name,
            rule_event::TAKEN,
            Some(&copy.label),
            copy.note.as_deref(),
        );
        if let Err(e) = remove_if_present(&copy.marker) {
            // The text is already stored; a surviving marker now sits beside
            // a copy equal to the rule, so the next pass reports it as lost
            // instead of taking it up again.
            out.rule_failed(&name, copy.marker.display().to_string(), e);
        }
    }
    out.taken.push(name);
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
/// That is why it is private and reached only through `refresh_rules_locked`
/// (and `refresh_rules`, which takes the lock around it) or, at launch, through
/// `deliver_rules_at_startup`, which collects before it seeds.
///
/// The pass, in order:
/// 1. read the conflicts open before it (`read_open_conflicts`);
/// 2. scan every repo's markers (`scan_repo_markers`, read-only);
/// 3. classify them — unknown, superseded, lost, unread, handed back
///    (`classify`, T-000207);
/// 4. settle the conflicts open before this pass that are not live in it —
///    **close** one once every side was read, otherwise leave it
///    **undecided**: open, quiet, its hand-backs not taken
///    (`settle_open_conflicts`, T-000206);
/// 5. per handed-back skill: a disagreement is a conflict — **opened** and
///    reported once, **quiet** on every later pass, markers left in place,
///    because taking the last one silently would lose somebody's rule
///    (`record_conflict`, T-000206); an undecided skill is skipped; anything
///    else is taken up (`take_up`).
///
/// Lost and superseded entries read `<skill> (<repo>)`, the repo labelled by
/// `rule_copy_label` (full `owner/name`, or the path of a local-only repo).
///
/// T-000174: every outcome that is not an error is also written to the rule's
/// history (`rule_events`) — `taken` once per repo whose copy was taken, with
/// the marker's approval line, `lost` and `superseded` with their repo, and the
/// conflict events. The report is a toast that is gone in seconds; the history
/// is what answers "where did this rule's text come from" afterwards. A failed
/// history write is reported as `RuleFailed` like any other step here and never
/// undoes or blocks the outcome it describes.
fn collect_approved_skill_edits(
    db: &AppDb,
    repos: &[Repository],
    settings_edit: Option<&str>,
) -> Collected {
    let mut out = Collected::default();
    let open = read_open_conflicts(db, &mut out);
    let known: HashMap<String, String> = match list_skill_templates(db) {
        Ok(v) => v.into_iter().collect(),
        Err(e) => {
            out.errors
                .push(SyncIssue::lookup(lookup::SKILLS, "").detail(e));
            return out;
        }
    };
    let scans: Vec<RepoScan> = repos.iter().filter_map(scan_repo_markers).collect();
    let followed: HashSet<String> = super::shared_canon::followed_rules(db)
        .into_keys()
        .collect();
    let found = classify(db, &scans, &known, settings_edit, &followed, &mut out);
    // Closed first, so a `conflict_resolved` reads before whatever this pass
    // then takes up.
    let undecided = settle_open_conflicts(db, repos, &open, &found, settings_edit, &mut out);
    for (name, copies) in found.handed {
        if in_conflict(&copies) {
            record_conflict(db, repos, &name, &copies, open.get(&name), &mut out);
        } else if !undecided.contains(&name) {
            take_up(db, name, &copies, &mut out);
        }
    }
    out.sorted()
}

/// T-000206: the error of `resolve_conflict` when the rule has no open
/// conflict (it was decided meanwhile). The frontend matches the string.
pub const ERR_CONFLICT_GONE: &str = "rule_conflict_gone";
/// T-000206: the error of `resolve_conflict` when the side to accept is no
/// longer there — its marker was removed or its copy is unreadable.
pub const ERR_CONFLICT_SIDE_GONE: &str = "rule_conflict_side_gone";

/// T-000206: the markers of one rule across the portfolio, as they are on disk
/// right now.
#[derive(Debug, Default)]
struct LiveConflict {
    /// Sides whose copy could be read, sorted by label.
    sides: Vec<RuleConflictSide>,
    /// Every marker of the rule, readable copy or not — all of them are cleared
    /// when the conflict is decided.
    markers: Vec<PathBuf>,
}

/// Read the markers of `rule` in every repo of `repos`. Nothing is written. A
/// side whose copy cannot be read is left out of `sides` (the screen has
/// nothing to show for it) but its marker is still listed, so deciding the
/// conflict clears it too.
fn live_conflict(repos: &[Repository], rule: &str) -> LiveConflict {
    let mut live = LiveConflict::default();
    for scan in repos.iter().filter_map(scan_repo_markers) {
        for marker in scan.markers.iter().filter(|m| m.name == rule) {
            live.markers.push(marker.path.clone());
            let content = match fs::read_to_string(scan.dir.join(&marker.stem)) {
                Ok(content) => content,
                Err(e) => {
                    eprintln!(
                        "[skills] {}: {} unreadable, left out of the conflict view: {}",
                        scan.label, marker.stem, e
                    );
                    continue;
                }
            };
            live.sides.push(RuleConflictSide {
                repo: scan.label.clone(),
                marked_at: fs::metadata(&marker.path)
                    .and_then(|m| m.modified())
                    .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                    .ok(),
                note: fs::read_to_string(&marker.path)
                    .ok()
                    .map(|note| note.trim().to_string())
                    .filter(|note| !note.is_empty()),
                content,
            });
        }
    }
    live.sides.sort_by(|a, b| a.repo.cmp(&b.repo));
    live
}

/// T-000206: the sides of the hand-back conflict on `rule`, read live from the
/// portfolio — what the owner chooses between. Under the rules lock, so a
/// collection cannot clear a marker halfway through the read.
pub fn conflict_sides(db: &AppDb, rule: &str) -> Result<Vec<RuleConflictSide>, String> {
    let _guard = rules_guard();
    let repos = rules_repos(db).map_err(|e| e.to_string())?;
    Ok(live_conflict(&repos, rule).sides)
}

/// T-000206: the owner decides the open conflict on `rule`, all under ONE hold
/// of the rules lock so no collection runs between the steps.
///
/// `accept` = the label of the side whose text becomes the rule; `None`
/// rejects every side and leaves the text as it is. Then, in this order:
/// 1. the accepted text is stored as a custom rule;
/// 2. every approval marker of the rule is cleared across the portfolio;
/// 3. the conflict is closed (`close_conflict`): row gone, `conflict_resolved`
///    recorded — with the label when a side was accepted, which makes it the
///    rule's origin — and the reason files removed;
/// 4. the rule is delivered everywhere, with `rule` as the settings edit: a
///    marker that could not be cleared in step 2 is superseded, not taken.
///
/// Errors are the two `ERR_*` codes, a failed read of the repo list (all
/// checked before anything is touched) or a failed store of the accepted text. Past that point the decision is made: a
/// step that fails is reported in the returned report's errors and the
/// remaining steps still run, as in every other pass.
///
/// A conflict closes even when the live scan finds no side at all (markers
/// removed by hand, checkouts gone): the owner must be able to clear a state
/// the portfolio can no longer explain.
pub fn resolve_conflict(
    db: &AppDb,
    claude_dir: Option<&Path>,
    rule: &str,
    accept: Option<&str>,
) -> Result<RulesRefresh, String> {
    let _guard = rules_guard();
    let open = db.open_rule_conflicts().map_err(|e| e.to_string())?;
    if !open.contains(rule) {
        return Err(ERR_CONFLICT_GONE.to_string());
    }
    // A failed listing is refused outright: with no repos nothing is read, so
    // an accept would claim its side is gone and a reject would close the
    // conflict without clearing a single marker.
    let repos = rules_repos(db).map_err(|e| e.to_string())?;
    let live = live_conflict(&repos, rule);
    let accepted = match accept {
        Some(label) => Some(
            live.sides
                .iter()
                .find(|side| side.repo == label)
                .ok_or_else(|| ERR_CONFLICT_SIDE_GONE.to_string())?,
        ),
        None => None,
    };

    if let Some(side) = accepted {
        let file_name = format!("{}{}{}", SKILL_PREFIX, rule, SKILL_SUFFIX);
        db.upsert_template_file("_global", &file_name, &side.content, true)
            .map_err(|e| e.to_string())?;
    }
    let mut out = Collected::default();
    for marker in &live.markers {
        if let Err(e) = remove_if_present(marker) {
            out.rule_failed(rule, marker.display().to_string(), e);
        }
    }
    let (repo, note) = match accepted {
        Some(side) => (Some(side.repo.as_str()), conflict_note::ACCEPTED),
        None => (None, conflict_note::REJECTED_BY_OWNER),
    };
    // If this close fails the pass goes on: the next pass's settle retries it as
    // SUPERSEDED_BY_OWNER, so an accepted side then has no origin event
    // (accepted trade-off, the failure is reported in the outcome's errors).
    close_conflict(db, &repos, rule, repo, note, &mut out);

    let mut report = refresh_rules_locked(db, claude_dir, RenderScope::Everywhere, Some(rule));
    let mut errors = out.errors;
    errors.append(&mut report.errors);
    report.errors = errors;
    Ok(report)
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
    refresh_rules_locked(db, claude_dir, scope, settings_edit)
}

/// The body of `refresh_rules` for a caller that already holds `rules_guard()`
/// (T-000206: resolving a conflict runs its own steps and this refresh under
/// one hold of the lock).
pub(crate) fn refresh_rules_locked(
    db: &AppDb,
    claude_dir: Option<&Path>,
    scope: RenderScope,
    settings_edit: Option<&str>,
) -> RulesRefresh {
    let mut report = RulesRefresh::default();
    let repos = portfolio_repos(db, &mut report);

    let collected = collect_approved_skill_edits(db, &repos, settings_edit);
    report.absorb(collected);
    let everywhere = !report.taken.is_empty();
    render_scope(db, claude_dir, scope, everywhere, &repos, &mut report);
    report
}

/// T-000174: the checkout of `repo` on THIS machine — `None` without a local
/// path or when the directory is not there. The one predicate for "a rule is
/// rendered into this repo": the render and the Rules screen's repo count both
/// follow it, so the number shown is the number written to.
pub(crate) fn existing_checkout(repo: &Repository) -> Option<&Path> {
    let base = Path::new(repo.local_path.as_deref()?);
    base.is_dir().then_some(base)
}

/// T-000174: how many repos a rule reaches (`existing_checkout`).
pub fn count_render_targets(db: &AppDb) -> Result<usize, String> {
    let repos = rules_repos(db).map_err(|e| e.to_string())?;
    Ok(repos
        .iter()
        .filter(|repo| existing_checkout(repo).is_some())
        .count())
}

/// Every repo with a local path — what a collection covers. A failed listing
/// is reported and yields none: nothing is read, so nothing is decided.
fn portfolio_repos(db: &AppDb, report: &mut RulesRefresh) -> Vec<Repository> {
    match rules_repos(db) {
        Ok(v) => v,
        Err(e) => {
            report
                .errors
                .push(SyncIssue::lookup(lookup::REPOS, "").detail(e));
            Vec::new()
        }
    }
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
        let (skills, touched, errors) =
            render_skills_everywhere(db, claude_dir, repos, &report.quiet_conflicts);
        report.skills = skills;
        report.repos = touched;
        report.errors.extend(errors);
        if !matches!(scope, RenderScope::Repo(_)) {
            return;
        }
    }
    match scope {
        RenderScope::Repo(base) => {
            // Р16: a contributor's / pending helper's checkout takes no rule
            // copies. `repos` already leaves such a repo out, so it is found by
            // its path in the whole portfolio.
            if db
                .list_all_repos()
                .map(|all| {
                    all.iter().any(|r| {
                        r.local_path.as_deref().map(Path::new) == Some(base)
                            && skips_rule_copies(db, r.id)
                    })
                })
                .unwrap_or(false)
            {
                return;
            }
            // Rendered even after an everywhere pass: the target may not be among
            // the stored checkouts, and an unchanged file is not rewritten anyway.
            match render_skills_to_repo(db, base) {
                // Already rendered (and its held copies reported) by the
                // everywhere pass when the target is a stored checkout.
                Ok(_) if everywhere => {}
                Ok(r) => {
                    report.repos = 1;
                    report.errors.extend(
                        r.held_issues(&base.display().to_string(), &report.quiet_conflicts),
                    );
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
        self.quiet_conflicts = collected.quiet_conflicts;
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
/// Private since T-000207: reached only through `render_scope`, from
/// `refresh_rules_locked` and `deliver_rules_at_startup`, both of which collect
/// the hand-backs in `repos` before this overwrites them.
///
/// Returns `(skills, repos, errors)`. Errors are per-target and non-fatal: one
/// unwritable repo must not stop the rest of the portfolio from being updated.
fn render_skills_everywhere(
    db: &AppDb,
    claude_dir: Option<&Path>,
    repos: &[Repository],
    quiet: &[String],
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
        // A stored local_path can point at a checkout that only exists on
        // another machine. That is normal portfolio state, not a failure —
        // skipped silently rather than drowning the real errors.
        let Some(base) = existing_checkout(repo) else {
            continue;
        };
        // T-000198: the same label the project-sync skeleton pass uses, so a
        // held copy seen by both renders of one pass is reported once.
        let label = rule_copy_label(repo);
        match render_skills_to_repo(db, base) {
            Ok(r) => {
                touched += 1;
                errors.extend(r.held_issues(&label, quiet));
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
    let repos = portfolio_repos(db, &mut report);
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
    use crate::db::conflict_note;
    use crate::db::rules::rule_history;
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
        assert!(remove_if_present(&tmp.path().join("sdh-alpha.md.approved")).is_ok());
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
        assert_eq!(errors[0].code, SyncIssueCode::RuleConflict, "{}", errors[0]);
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
        let mut events: Vec<_> = rule_history(&db, "sdh-beta")
            .into_iter()
            .map(|e| (e.kind, e.repo, e.note))
            .collect();
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
            rule_history(&db, "sdh-alpha").is_empty(),
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
        let events = rule_history(&db, "sdh-alpha");
        assert_eq!(events.len(), 1);
        assert_eq!(
            (
                events[0].kind.as_str(),
                events[0].repo.as_deref(),
                events[0].note.as_deref()
            ),
            ("lost", Some(label.as_str()), None)
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
        let events = rule_history(&db, "sdh-alpha");
        assert_eq!(events.len(), 1);
        assert_eq!(
            (
                events[0].kind.as_str(),
                events[0].repo.as_deref(),
                events[0].note.as_deref()
            ),
            ("superseded", Some(label.as_str()), None)
        );
    }

    // ── T-000206: a conflict is reported once and explained to the agent ─────

    /// Two stored repos handing back different texts of `sdh-alpha`. Returns
    /// their checkouts and labels.
    fn conflicting_repos(db: &AppDb) -> (TempDir, TempDir, String, String) {
        let one = TempDir::new().unwrap();
        let two = TempDir::new().unwrap();
        let (r1, s1) = rendered_repo(db, &one, "one");
        let (r2, s2) = rendered_repo(db, &two, "two");
        hand_back(&s1, "sdh-alpha", "VERSION ONE", "");
        hand_back(&s2, "sdh-alpha", "VERSION TWO", "");
        let (l1, l2) = (rule_copy_label(&r1), rule_copy_label(&r2));
        (one, two, l1, l2)
    }

    fn reason_file(repo: &TempDir, name: &str) -> PathBuf {
        repo.path()
            .join("docs/sdh_skills")
            .join(format!("{}.md{}", name, CONFLICT_SUFFIX))
    }

    /// Issues of `code` about `skill` — as their subject (a conflict) or their
    /// target (a held copy).
    fn count(errors: &[SyncIssue], code: SyncIssueCode, skill: &str) -> usize {
        errors
            .iter()
            .filter(|e| {
                e.code == code && (e.subject == skill || e.target.as_deref() == Some(skill))
            })
            .count()
    }

    /// The conflict used to fire a `RuleConflict` toast, plus a `RuleHeld`
    /// line per held copy, on EVERY sync, and lived nowhere else. Now the pass
    /// that opens it reports it in full, records it, and leaves each agent a
    /// note; every later pass keeps quiet about it.
    #[test]
    fn test_a_conflict_is_reported_once_then_kept_quiet() {
        let db = db_with_skill();
        let home = TempDir::new().unwrap();
        let (one, two, l1, l2) = conflicting_repos(&db);

        let first = refresh_rules(&db, Some(home.path()), RenderScope::Everywhere, None);

        assert_eq!(
            count(&first.errors, SyncIssueCode::RuleConflict, "sdh-alpha"),
            1,
            "{:?}",
            first.errors
        );
        assert_eq!(
            count(&first.errors, SyncIssueCode::RuleHeld, "sdh-alpha"),
            2,
            "the opening pass reports the held copies too: {:?}",
            first.errors
        );
        assert!(first.quiet_conflicts.is_empty());
        assert!(db.open_rule_conflicts().unwrap().contains("sdh-alpha"));
        let events = rule_history(&db, "sdh-alpha");
        assert_eq!(events.len(), 1, "{:?}", events);
        assert_eq!(events[0].kind, "conflict");
        let repos = events[0].repo.clone().unwrap();
        assert!(repos.contains(&l1) && repos.contains(&l2), "{}", repos);
        for repo in [&one, &two] {
            let reason = fs::read_to_string(reason_file(repo, "sdh-alpha")).unwrap();
            assert!(reason.starts_with("# Hand-back not taken: conflict\n"));
            assert!(reason.contains("`sdh-alpha`"), "{}", reason);
            assert!(
                reason.contains(&format!("- {} — marker ", l1))
                    && reason.contains(&format!("- {} — marker ", l2)),
                "{}",
                reason
            );
        }

        let second = refresh_rules(&db, Some(home.path()), RenderScope::Everywhere, None);

        assert_eq!(
            count(&second.errors, SyncIssueCode::RuleConflict, "sdh-alpha"),
            0,
            "{:?}",
            second.errors
        );
        assert_eq!(
            count(&second.errors, SyncIssueCode::RuleHeld, "sdh-alpha"),
            0,
            "{:?}",
            second.errors
        );
        assert_eq!(second.quiet_conflicts, vec!["sdh-alpha".to_string()]);
        assert_eq!(rule_history(&db, "sdh-alpha").len(), 1, "no second event");
        // Still in conflict: nothing taken, nothing thrown away.
        assert!(reason_file(&one, "sdh-alpha").exists());
        assert_eq!(
            fs::read_to_string(one.path().join("docs/sdh_skills/sdh-alpha.md")).unwrap(),
            "VERSION ONE"
        );
    }

    /// The owner may settle the conflict by hand - delete one marker. The next
    /// pass closes it, takes the hand-back that is left, and tidies up after
    /// itself in every repo.
    #[test]
    fn test_a_conflict_resolved_outside_the_app_is_closed_and_the_rest_taken() {
        let db = db_with_skill();
        let (one, two, _, l2) = conflicting_repos(&db);
        let repos = db.list_repos_with_local_path(None).unwrap();
        collect_approved_skill_edits(&db, &repos, None);
        assert!(reason_file(&one, "sdh-alpha").exists());
        fs::remove_file(one.path().join("docs/sdh_skills/sdh-alpha.md.approved")).unwrap();

        let Collected {
            taken,
            errors,
            quiet_conflicts,
            ..
        } = collect_approved_skill_edits(&db, &repos, None);

        assert!(errors.is_empty(), "{:?}", errors);
        assert!(quiet_conflicts.is_empty());
        assert_eq!(taken, vec!["sdh-alpha".to_string()]);
        assert_eq!(
            db.get_template_file("_global", "skill.sdh-alpha.md.tmpl")
                .unwrap()
                .unwrap()
                .content,
            "VERSION TWO"
        );
        assert!(db.open_rule_conflicts().unwrap().is_empty());
        assert!(!reason_file(&one, "sdh-alpha").exists());
        assert!(!reason_file(&two, "sdh-alpha").exists());
        let events: Vec<_> = rule_history(&db, "sdh-alpha")
            .into_iter()
            .map(|e| (e.kind, e.repo, e.note))
            .collect();
        assert_eq!(
            events[1..],
            [
                (
                    "conflict_resolved".to_string(),
                    None,
                    Some(conflict_note::RESOLVED_OUTSIDE.to_string())
                ),
                ("taken".to_string(), Some(l2), None),
            ]
        );
    }

    /// `n` stored GitHub repos `acme/r<i>`, each handing back its own text of
    /// `sdh-alpha` (`VERSION <i>`). GitHub labels, so a test can rename one.
    fn conflict_of(db: &AppDb, n: usize) -> Vec<TempDir> {
        (0..n)
            .map(|i| {
                let dir = TempDir::new().unwrap();
                insert_repo(db, &format!("acme/r{}", i), &dir.path().to_string_lossy());
                render_skills_to_repo(db, dir.path()).unwrap();
                hand_back(
                    &dir.path().join("docs/sdh_skills"),
                    "sdh-alpha",
                    &format!("VERSION {}", i),
                    "",
                );
                dir
            })
            .collect()
    }

    fn portfolio(db: &AppDb) -> Vec<Repository> {
        db.list_repos_with_local_path(None).unwrap()
    }

    /// The side leaves the conflict: its marker is removed by hand.
    fn drop_out(dir: &TempDir) {
        fs::remove_file(dir.path().join("docs/sdh_skills/sdh-alpha.md.approved")).unwrap();
    }

    fn resolved(db: &AppDb) -> Vec<crate::models::RuleEvent> {
        rule_history(db, "sdh-alpha")
            .into_iter()
            .filter(|e| e.kind == rule_event::CONFLICT_RESOLVED)
            .collect()
    }

    fn rule_text(db: &AppDb) -> String {
        db.get_template_file("_global", "skill.sdh-alpha.md.tmpl")
            .unwrap()
            .unwrap()
            .content
    }

    /// T-000206: nothing about `sdh-alpha` was decided this pass — the
    /// conflict is still open and quiet, nothing was taken, and `keeper`
    /// (a side that was read) still has its marker, its copy and its note.
    fn assert_conflict_still_open(
        db: &AppDb,
        taken: &[String],
        quiet: &[String],
        keeper: &TempDir,
        keeper_text: &str,
    ) {
        assert!(taken.is_empty(), "{:?}", taken);
        assert_eq!(quiet, ["sdh-alpha".to_string()]);
        assert!(db.open_rule_conflicts().unwrap().contains("sdh-alpha"));
        assert!(resolved(db).is_empty(), "{:?}", resolved(db));
        assert_eq!(rule_text(db), "ALPHA BODY");
        let skills = keeper.path().join("docs/sdh_skills");
        assert!(skills.join("sdh-alpha.md.approved").exists());
        assert_eq!(
            fs::read_to_string(skills.join("sdh-alpha.md")).unwrap(),
            keeper_text
        );
        assert!(reason_file(keeper, "sdh-alpha").exists());
    }

    /// T-000206: a side missing from this pass is not a side that gave up.
    /// Closing then would record a false "resolved outside the app" and take
    /// the other side, which the missing one overwrites once it is back.
    #[test]
    fn test_an_open_conflict_with_a_side_left_out_of_the_pass_is_not_closed() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        let without_r0: Vec<Repository> = portfolio(&db)
            .into_iter()
            .filter(|r| r.github_name.as_deref() != Some("acme/r0"))
            .collect();

        let c = collect_approved_skill_edits(&db, &without_r0, None);

        assert_conflict_still_open(&db, &c.taken, &c.quiet_conflicts, &dirs[1], "VERSION 1");
    }

    /// Same, for a side whose checkout is not on this machine.
    #[test]
    fn test_an_open_conflict_with_a_missing_checkout_is_not_closed() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE repositories SET local_path = 'F:/definitely/not/cloned/here'
                 WHERE github_name = 'acme/r0'",
                [],
            )
            .unwrap();

        let c = collect_approved_skill_edits(&db, &portfolio(&db), None);

        assert_conflict_still_open(&db, &c.taken, &c.quiet_conflicts, &dirs[1], "VERSION 1");
    }

    /// Same, for a side whose folder is readable but whose copy is not.
    #[test]
    fn test_an_open_conflict_with_an_unreadable_copy_is_not_closed() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        fs::write(
            dirs[0].path().join("docs/sdh_skills/sdh-alpha.md"),
            [0xff, 0xfe, 0x00, 0xc3],
        )
        .unwrap();

        let c = collect_approved_skill_edits(&db, &portfolio(&db), None);

        assert_conflict_still_open(&db, &c.taken, &c.quiet_conflicts, &dirs[1], "VERSION 1");
        assert!(c.errors.iter().any(|e| e.code == SyncIssueCode::RuleFailed));
    }

    /// A failed repo listing reads no side at all, so it closes nothing.
    #[test]
    fn test_a_failed_repo_listing_closes_no_conflict() {
        let db = db_with_skill();
        let home = TempDir::new().unwrap();
        let dirs = conflict_of(&db, 2);
        refresh_rules(&db, Some(home.path()), RenderScope::UptakeOnly, None);
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE repositories")
            .unwrap();

        let report = refresh_rules(&db, Some(home.path()), RenderScope::UptakeOnly, None);

        assert!(!report.errors.is_empty(), "the listing failure is reported");
        assert_conflict_still_open(
            &db,
            &report.taken,
            &report.quiet_conflicts,
            &dirs[1],
            "VERSION 1",
        );
    }

    /// Sides leave one by one. While two still differ the conflict stays open
    /// and quiet, and only the leaver loses its reason file; once one side is
    /// left it closes as resolved outside the app and that side is taken.
    #[test]
    fn test_a_three_repo_conflict_closes_when_one_side_is_left() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 3);

        let first = collect_approved_skill_edits(&db, &portfolio(&db), None);
        let conflict: Vec<_> = first
            .errors
            .iter()
            .filter(|e| e.code == SyncIssueCode::RuleConflict)
            .collect();
        assert_eq!(conflict.len(), 1, "{:?}", first.errors);
        assert_eq!(
            conflict[0].target.as_deref(),
            Some("acme/r0, acme/r1, acme/r2"),
            "every side is named"
        );

        drop_out(&dirs[1]);
        let second = collect_approved_skill_edits(&db, &portfolio(&db), None);
        assert!(second.taken.is_empty());
        assert_eq!(second.quiet_conflicts, vec!["sdh-alpha".to_string()]);
        assert!(resolved(&db).is_empty());
        assert!(!reason_file(&dirs[1], "sdh-alpha").exists(), "the leaver's");
        assert!(reason_file(&dirs[0], "sdh-alpha").exists());
        assert!(reason_file(&dirs[2], "sdh-alpha").exists());

        drop_out(&dirs[2]);
        let third = collect_approved_skill_edits(&db, &portfolio(&db), None);
        assert!(third.errors.is_empty(), "{:?}", third.errors);
        assert_eq!(third.taken, vec!["sdh-alpha".to_string()]);
        assert_eq!(rule_text(&db), "VERSION 0");
        assert!(db.open_rule_conflicts().unwrap().is_empty());
        let closed = resolved(&db);
        assert_eq!(closed.len(), 1);
        assert_eq!(
            closed[0].note.as_deref(),
            Some(conflict_note::RESOLVED_OUTSIDE)
        );
        for dir in &dirs {
            assert!(!reason_file(dir, "sdh-alpha").exists());
        }
    }

    /// A side is known by its repo id, not by its label: renaming the repo
    /// on GitHub does not make it a stranger that blocks the conflict forever.
    #[test]
    fn test_a_renamed_side_does_not_block_closing() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE repositories SET github_name = 'acme/renamed' WHERE github_name = 'acme/r0'",
                [],
            )
            .unwrap();
        drop_out(&dirs[1]);

        let c = collect_approved_skill_edits(&db, &portfolio(&db), None);

        assert_eq!(c.taken, vec!["sdh-alpha".to_string()]);
        assert_eq!(rule_text(&db), "VERSION 0");
        assert!(db.open_rule_conflicts().unwrap().is_empty());
        assert_eq!(resolved(&db).len(), 1);
    }

    /// A side removed from the portfolio can hand nothing back any more: it
    /// does not block the conflict either.
    #[test]
    fn test_a_deleted_side_does_not_block_closing() {
        let db = db_with_skill();
        let _dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        db.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM repositories WHERE github_name = 'acme/r1'", [])
            .unwrap();

        let c = collect_approved_skill_edits(&db, &portfolio(&db), None);

        assert_eq!(c.taken, vec!["sdh-alpha".to_string()]);
        assert_eq!(rule_text(&db), "VERSION 0");
        assert!(db.open_rule_conflicts().unwrap().is_empty());
        assert_eq!(resolved(&db).len(), 1);
    }

    /// A repo joining an open conflict is not a new conflict (no toast), but
    /// the history shows it joined, and it becomes a side that must be read.
    #[test]
    fn test_a_repo_joining_an_open_conflict_is_recorded_quietly() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        let third = TempDir::new().unwrap();
        insert_repo(&db, "acme/r2", &third.path().to_string_lossy());
        render_skills_to_repo(&db, third.path()).unwrap();
        hand_back(
            &third.path().join("docs/sdh_skills"),
            "sdh-alpha",
            "VERSION 2",
            "",
        );

        let c = collect_approved_skill_edits(&db, &portfolio(&db), None);

        assert!(
            c.errors
                .iter()
                .all(|e| e.code != SyncIssueCode::RuleConflict),
            "{:?}",
            c.errors
        );
        assert_eq!(c.quiet_conflicts, vec!["sdh-alpha".to_string()]);
        let conflicts: Vec<_> = rule_history(&db, "sdh-alpha")
            .into_iter()
            .filter(|e| e.kind == rule_event::CONFLICT)
            .collect();
        assert_eq!(conflicts.len(), 2, "{:?}", conflicts);
        assert_eq!(
            conflicts[1].repo.as_deref(),
            Some("acme/r0, acme/r1, acme/r2")
        );
        assert!(reason_file(&third, "sdh-alpha").exists());

        // The join pass stored the newcomer as a side.
        let stored = db.list_rule_conflicts().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(
            crate::db::parse_conflict_repo_ids(&stored[0].1).unwrap(),
            portfolio(&db).iter().map(|r| r.id).collect::<BTreeSet<_>>(),
            "the join pass recorded r0, r1 and r2 as the sides"
        );

        // The newcomer is a side now: the other two give up, but a pass that
        // leaves it out has not seen it, so it must not close the conflict.
        drop_out(&dirs[0]);
        drop_out(&dirs[1]);
        let without: Vec<Repository> = portfolio(&db)
            .into_iter()
            .filter(|r| r.github_name.as_deref() != Some("acme/r2"))
            .collect();
        let c = collect_approved_skill_edits(&db, &without, None);
        assert_conflict_still_open(&db, &c.taken, &c.quiet_conflicts, &third, "VERSION 2");
    }

    /// Once closed, a conflict that comes back is a new one: reported in full.
    #[test]
    fn test_a_conflict_that_comes_back_after_closing_is_reported_again() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        drop_out(&dirs[1]);
        let closed = collect_approved_skill_edits(&db, &portfolio(&db), None);
        assert_eq!(closed.taken, vec!["sdh-alpha".to_string()]);
        for (i, dir) in dirs.iter().enumerate() {
            hand_back(
                &dir.path().join("docs/sdh_skills"),
                "sdh-alpha",
                &format!("AGAIN {}", i),
                "",
            );
        }

        let again = collect_approved_skill_edits(&db, &portfolio(&db), None);

        assert_eq!(
            again
                .errors
                .iter()
                .filter(|e| e.code == SyncIssueCode::RuleConflict)
                .count(),
            1,
            "{:?}",
            again.errors
        );
        assert!(again.quiet_conflicts.is_empty());
        assert!(db.open_rule_conflicts().unwrap().contains("sdh-alpha"));
    }

    /// Stored sides that cannot be parsed prove nothing: the conflict stays
    /// open and quiet, and the damage is reported.
    #[test]
    fn test_unparseable_sides_keep_the_conflict_open() {
        let db = db_with_skill();
        let dirs = conflict_of(&db, 2);
        collect_approved_skill_edits(&db, &portfolio(&db), None);
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE rule_conflicts SET repo_ids = 'not json'", [])
            .unwrap();
        drop_out(&dirs[0]);

        let c = collect_approved_skill_edits(&db, &portfolio(&db), None);

        assert_conflict_still_open(&db, &c.taken, &c.quiet_conflicts, &dirs[1], "VERSION 1");
        assert!(c.errors.iter().any(|e| e.code == SyncIssueCode::RuleFailed));
    }

    /// The reason file sits in `docs/sdh_skills/` beside the copies, so both
    /// passes that read that folder must leave it alone.
    #[test]
    fn test_a_reason_file_survives_the_prune_and_is_not_a_marker() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        let known = skills.join(format!("sdh-alpha.md{}", CONFLICT_SUFFIX));
        let unknown = skills.join(format!("sdh-obsolete.md{}", CONFLICT_SUFFIX));
        fs::write(&known, "reason").unwrap();
        fs::write(&unknown, "reason").unwrap();

        let rendered = render_skills_to_repo(&db, tmp.path()).unwrap();
        let collected = collect_approved_skill_edits(&db, &[repo], None);

        assert!(known.exists() && unknown.exists(), "the prune left them");
        assert!(
            rendered.held.is_empty(),
            "not a marker: {:?}",
            rendered.held
        );
        assert!(collected.taken.is_empty() && collected.lost.is_empty());
        assert!(collected.errors.is_empty(), "{:?}", collected.errors);
    }

    /// The history is a record of what happened, never a precondition for it:
    /// a broken `rule_events` must not stop an uptake.
    #[test]
    fn test_a_failed_history_write_does_not_block_an_uptake() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        hand_back(&skills, "sdh-alpha", "ALPHA REVISED", "approved");
        db.conn
            .lock()
            .unwrap()
            .execute_batch("DROP TABLE rule_events")
            .unwrap();

        let Collected { taken, errors, .. } = collect_approved_skill_edits(&db, &[repo], None);

        assert_eq!(taken, vec!["sdh-alpha".to_string()]);
        assert_eq!(
            db.get_template_file("_global", "skill.sdh-alpha.md.tmpl")
                .unwrap()
                .unwrap()
                .content,
            "ALPHA REVISED"
        );
        assert!(!skills.join("sdh-alpha.md.approved").exists());
        assert_eq!(errors.len(), 1, "{:?}", errors);
        assert_eq!(errors[0].code, SyncIssueCode::RuleFailed);
    }

    // ── F-000044 / Р16: a contributor's repo takes no rule copies ───────────────

    fn repo_with_role(db: &AppDb, dir: &TempDir, name: &str, role: Option<&str>) -> Repository {
        let repo = db
            .insert_local_repository(&dir.path().to_string_lossy(), name, None, None)
            .unwrap();
        db.set_shared_role(repo.id, role).unwrap();
        db.get_repository(repo.id).unwrap()
    }

    #[test]
    fn test_a_contributor_or_pending_repo_gets_no_rule_copies_and_loses_none() {
        let db = db_with_skill();
        let ordinary = TempDir::new().unwrap();
        let contributor = TempDir::new().unwrap();
        let pending = TempDir::new().unwrap();
        let owner = TempDir::new().unwrap();
        repo_with_role(&db, &ordinary, "acme/one", None);
        repo_with_role(&db, &contributor, "acme/two", Some("contributor"));
        repo_with_role(&db, &pending, "acme/three", Some("pending"));
        let owner_repo = repo_with_role(&db, &owner, "me/four", Some("main"));
        db.set_shared_mode(owner_repo.id, true).unwrap();
        // The contributor's clone holds the owner's tracked copies, one of them
        // a name this install's bundle no longer has: not pruned.
        let held = contributor.path().join("docs/sdh_skills");
        fs::create_dir_all(&held).unwrap();
        fs::write(held.join("sdh-alpha.md"), "THE OWNER'S ALPHA").unwrap();
        fs::write(held.join("sdh-old.md"), "THE OWNER'S OLD").unwrap();

        let report = refresh_rules(&db, None, RenderScope::Everywhere, None);

        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.repos, 2, "the ordinary repo and the owner's");
        assert!(ordinary
            .path()
            .join("docs/sdh_skills/sdh-alpha.md")
            .exists());
        assert!(owner.path().join("docs/sdh_skills/sdh-alpha.md").exists());
        assert_eq!(
            fs::read_to_string(held.join("sdh-alpha.md")).unwrap(),
            "THE OWNER'S ALPHA"
        );
        assert!(!held.join("sdh-beta.md").exists());
        assert!(held.join("sdh-old.md").exists(), "no prune either");
        assert!(!pending.path().join("docs").exists(), "nothing written");
        assert_eq!(count_render_targets(&db).unwrap(), 2);
        // The single-repo render of init docs honours it too.
        let report = refresh_rules(&db, None, RenderScope::Repo(pending.path()), None);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(!pending.path().join("docs").exists());
    }

    #[test]
    fn test_a_contributors_hand_back_marker_is_not_collected() {
        let db = db_with_skill();
        let tmp = TempDir::new().unwrap();
        let (repo, skills) = rendered_repo(&db, &tmp, "a");
        hand_back(&skills, "sdh-alpha", "ALPHA REVISED", "approved");
        db.set_shared_role(repo.id, Some("contributor")).unwrap();

        let report = refresh_rules(&db, None, RenderScope::UptakeOnly, None);

        assert!(report.taken.is_empty() && report.lost.is_empty());
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(rule_text(&db), "ALPHA BODY");
        assert!(skills.join("sdh-alpha.md.approved").exists(), "left alone");
        assert_eq!(
            fs::read_to_string(skills.join("sdh-alpha.md")).unwrap(),
            "ALPHA REVISED"
        );
    }

    #[test]
    fn test_a_contributor_side_does_not_hold_a_conflict_open() {
        let db = db_with_skill();
        let (one, two) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        let (_, s1) = rendered_repo(&db, &one, "a");
        let (r2, s2) = rendered_repo(&db, &two, "b");
        hand_back(&s1, "sdh-alpha", "ALPHA ONE", "");
        hand_back(&s2, "sdh-alpha", "ALPHA TWO", "");
        let report = refresh_rules(&db, None, RenderScope::UptakeOnly, None);
        assert!(!report.errors.is_empty(), "the conflict is reported");
        assert!(db.open_rule_conflicts().unwrap().contains("sdh-alpha"));

        // The second repo becomes a contributor's: its marker is no longer read
        // and cannot keep the conflict open; the first one's text is taken.
        db.set_shared_role(r2.id, Some("contributor")).unwrap();
        let report = refresh_rules(&db, None, RenderScope::UptakeOnly, None);

        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(db.open_rule_conflicts().unwrap().is_empty());
        assert_eq!(rule_text(&db), "ALPHA ONE");
    }

    #[test]
    fn test_a_hand_back_of_a_rule_that_follows_an_owner_is_set_aside() {
        let db = db_with_skill();
        let shared = TempDir::new().unwrap();
        let mine = TempDir::new().unwrap();
        let shared_repo = repo_with_role(&db, &shared, "acme/shared", Some("contributor"));
        let (_, skills) = rendered_repo(&db, &mine, "mine");
        db.set_setting(
            super::super::shared_canon::RULES_FROM_MAIN_KEY,
            &format!(
                r#"{{"sdh-alpha":{{"login":"owner","repo_id":{},"committed_at":1}}}}"#,
                shared_repo.id
            ),
        )
        .unwrap();
        hand_back(&skills, "sdh-alpha", "ALPHA REVISED", "approved");

        let report = refresh_rules(&db, None, RenderScope::UptakeOnly, None);

        assert!(report.taken.is_empty());
        assert_eq!(report.superseded.len(), 1, "{:?}", report.superseded);
        assert_eq!(rule_text(&db), "ALPHA BODY");
        assert!(!skills.join("sdh-alpha.md.approved").exists());
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
