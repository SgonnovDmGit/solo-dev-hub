//! T-000241: the rule-proposal channel's commands — the contributor's own
//! proposals and the owner's decision machine.
//!
//! The transport itself lives in `sync::shared::sync_rule_proposals` (Phase
//! A); what is here is the local work: listing both sides, withdrawing a
//! proposal, and the strict-stepped decision machine (`accept`/`reject`)
//! that writes the immutable receipt and then applies the proposed text to
//! the owner's canon.

use std::path::{Path, PathBuf};

use tauri::State;

use crate::db::AppDb;
use crate::git_ops::{commit_paths, current_branch, GitBinary};
use crate::git_sync::{self, FileChange};
use crate::models::{
    acceptance_pub_state, acceptance_status, decision_action, proposal_status, RuleAcceptance,
    RuleDecisionFile, RuleProposalFile, RuleProposalView,
};
use crate::sync::rule_proposals as rp;
use crate::sync::shared_structure::{self as ss, ProjectStructure};

/// An open proposal's revision changed since the caller read it.
const ERR_PROPOSAL_CHANGED: &str = "proposal_changed";

fn git() -> Result<GitBinary, String> {
    crate::git_ops::cached_git().ok_or_else(|| "git not found".to_string())
}

fn repo_root(repo: &crate::models::Repository) -> Result<PathBuf, String> {
    repo.local_path
        .as_deref()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .ok_or_else(|| {
            format!(
                "{}: no local checkout",
                repo.github_name.as_deref().unwrap_or("repo")
            )
        })
}

fn load_structure(root: &Path) -> Result<ProjectStructure, String> {
    let bytes = std::fs::read(root.join(ss::STRUCTURE_REL_PATH))
        .map_err(|e| format!("{}: {}", ss::STRUCTURE_REL_PATH, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("{}: invalid JSON: {}", ss::STRUCTURE_REL_PATH, e))
}

/// The shared dev branch for reads and writes outside a sync pass. The
/// owner's structure file is the source when the worktree carries it; a
/// contributor's checkout is its own participant branch — `.sdh/project.json`
/// need not be in it — so the accepted pin in `repositories.shared_import`
/// is the fallback (it is the same branch the transport used).
fn dev_branch_for(db: &AppDb, repo_id: i64, root: &Path) -> Option<String> {
    if let Ok(s) = load_structure(root) {
        return Some(s.main.dev_branch);
    }
    ss::read_shared_import(db, repo_id).pinned_dev_branch
}

/// The proposal file's `(path, blob sha)` at `rev`, if present.
fn proposal_at(
    git: &GitBinary,
    root: &Path,
    rev: &str,
    rule: &str,
    login: &str,
) -> Result<Option<(String, String)>, String> {
    let path = format!(
        "{}/{}",
        rp::RULE_PROPOSALS_REL_DIR,
        rp::proposal_file_name(rule, login)
    );
    git_sync::blob_sha_at(git, root, rev, &path).map(|s| s.map(|sha| (path, sha)))
}

// ---------------------------------------------------------------------------
// The sweeps (startup + list): resume unfinished local work, retry
// publications, GC terminal proposal rows.
// ---------------------------------------------------------------------------

/// What one repo's sweep did — for tests and diagnostics; the sweep never
/// fails the list it ran before, its issues are reported alongside.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct RuleProposalSweep {
    /// `pending_apply` proposal rows whose text is now canon.
    pub applied_proposals: usize,
    /// Acceptance rows that moved at least one step forward.
    pub resumed_acceptances: usize,
    /// Terminal proposal rows the GC dropped (`max_seq` survives).
    pub gc_proposals: usize,
    /// Problems the sweep hit — it retries next time.
    pub issues: Vec<String>,
}

/// The local-work sweep's contributor half — DB only: a `pending_apply`
/// row's text becomes the local canon, then the row flips `applied`.
pub(crate) fn sweep_pending_apply(db: &AppDb, repo_id: i64) -> usize {
    let mut n = 0;
    for p in db.list_pending_apply_proposals().unwrap_or_default() {
        if p.repo_id != repo_id {
            continue;
        }
        let file = format!("skill.{}.md.tmpl", p.rule);
        if db
            .upsert_template_file("_global", &file, &p.text, true)
            .is_err()
        {
            continue;
        }
        if db.mark_rule_proposal_applied(repo_id, &p.file_name).is_ok() {
            n += 1;
        }
    }
    n
}

/// The local-work sweep's owner half: every unfinished acceptance resumes
/// at its first incomplete step — `prepared` → the receipt step, `decided`
/// → canon + delivery, `applied` → delivery, `rendered` → nothing left.
pub(crate) fn sweep_acceptances(
    db: &AppDb,
    git: &GitBinary,
    root: &Path,
    repo_id: i64,
    dev_branch: &str,
    claude_dir: Option<&Path>,
) -> (usize, Vec<String>) {
    let mut resumed = 0;
    let mut issues = Vec::new();
    for a in db.list_unfinished_rule_acceptances().unwrap_or_default() {
        if a.repo_id != repo_id {
            continue;
        }
        let before = a.status.clone();
        let key = format!(
            "{}.{}.{}",
            a.rule,
            a.proposer_login,
            &a.decided_rev[..a.decided_rev.len().min(8)]
        );
        let r = (|| -> Result<RuleAcceptance, String> {
            let a = if a.status == acceptance_status::PREPARED {
                publish_receipt(db, git, root, repo_id, dev_branch, &a)?
            } else {
                a
            };
            apply_decision(db, &a, claude_dir)
        })();
        match r {
            Ok(a) => {
                if a.status != before {
                    resumed += 1;
                }
            }
            Err(e) => issues.push(format!("{}: {}", key, e)),
        }
    }
    (resumed, issues)
}

/// The terminal-row GC: a proposal that was withdrawn or resolved AND whose
/// origin-confirmed `remote_ver` tuple dominates the local version AND that
/// has not been touched for 30 days is dropped — the `rule_proposal_seq`
/// high-water is a separate row and survives, so a re-proposal never
/// reuses a seq the remote once saw.
pub(crate) fn gc_terminal_proposals(db: &AppDb, repo_id: i64) -> usize {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
    let mut n = 0;
    for p in db.list_rule_proposals(repo_id).unwrap_or_default() {
        let terminal = p.status == proposal_status::WITHDRAWN || p.resolved_action.is_some();
        if !terminal {
            continue;
        }
        let Some(rv) = p.remote_ver.as_deref() else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(rv) else {
            continue;
        };
        let rseq = v["seq"].as_i64().unwrap_or(-1);
        let rstatus = v["status"].as_str().unwrap_or("");
        // The full resolver's tuple: greater seq dominates; equal seq is
        // dominated by the terminal status or by being the same version
        // the origin already confirmed.
        let dominates = rseq > p.seq
            || (rseq == p.seq && (rstatus == proposal_status::WITHDRAWN || rstatus == p.status));
        if !dominates {
            continue;
        }
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(&p.updated_at) else {
            continue;
        };
        if at > cutoff {
            continue;
        }
        if db.delete_rule_proposal(repo_id, &p.file_name).is_ok() {
            n += 1;
        }
    }
    n
}

/// Run every sweep for one repo: the local-work resume (pending proposals
/// for any role, unfinished acceptances for the owner), the publication
/// retry (owner) and the terminal-row GC. Called at startup for every
/// shared repo and before the list commands answer — the state the UI sees
/// is the freshest the machine could reach.
pub fn sweep_repo_rule_channel(
    db: &AppDb,
    git: Option<&GitBinary>,
    repo_id: i64,
    claude_dir: Option<&Path>,
) -> RuleProposalSweep {
    let mut out = RuleProposalSweep::default();
    out.applied_proposals = sweep_pending_apply(db, repo_id);
    out.gc_proposals = gc_terminal_proposals(db, repo_id);
    let Some(git) = git else { return out };
    let Ok(repo) = db.get_repository(repo_id) else {
        return out;
    };
    let Ok(root) = repo_root(&repo) else {
        return out;
    };
    let shared = db
        .get_shared_state(repo_id)
        .map(|s| s.shared_mode && s.shared_role.as_deref() == Some("main"))
        .unwrap_or(false);
    if !shared {
        return out;
    }
    let Some(dev_branch) = dev_branch_for(db, repo_id, &root) else {
        return out;
    };
    let (n, issues) = sweep_acceptances(db, git, &root, repo_id, &dev_branch, claude_dir);
    out.resumed_acceptances = n;
    out.issues.extend(issues);
    out.issues
        .extend(crate::sync::shared::sweep_rule_publications(
            db,
            git,
            &repo,
            &root,
            &dev_branch,
        ));
    out
}

/// The startup sweep: every repo that takes part in shared mode in any
/// way — its unfinished work resumes, its owed receipts retry.
pub fn sweep_rule_channels_at_startup(db: &AppDb) {
    let Some(git) = crate::git_ops::cached_git() else {
        return;
    };
    let claude = dirs::home_dir().map(|h| h.join(".claude"));
    for st in db.list_shared_states().unwrap_or_default() {
        let r = sweep_repo_rule_channel(db, Some(&git), st.repo_id, claude.as_deref());
        for i in r.issues {
            eprintln!("[startup] rule-proposal sweep, repo {}: {}", st.repo_id, i);
        }
    }
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

/// The blob sha this install's wire form of `p` hashes to — the same
/// digest `commit_files_to_branch` gives the uploaded bytes, so it is the
/// `rev` the dev tip will name once the upload lands.
pub(crate) fn local_rev(git: &GitBinary, root: &Path, p: &RuleProposalFile) -> Option<String> {
    let rel = format!(
        "{}/{}",
        rp::RULE_PROPOSALS_REL_DIR,
        rp::proposal_file_name(&p.rule, &p.login)
    );
    git_sync::hash_object_write(
        git,
        root,
        Path::new(&rel),
        rp::canonical_proposal_json(p).as_bytes(),
    )
    .ok()
}

/// Both catalogs at the local dev tip and the last-seen origin snapshot —
/// best-effort (a missing dev branch reads as two empty catalogs).
pub(crate) fn catalogs(
    git: Option<&GitBinary>,
    root: &Path,
    dev_branch: &str,
    db: &AppDb,
) -> (
    std::collections::BTreeMap<String, (String, RuleProposalFile)>,
    std::collections::BTreeMap<String, (String, RuleDecisionFile)>,
    std::collections::BTreeMap<String, (String, RuleProposalFile)>,
) {
    let empty = (
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::new(),
    );
    let Some(git) = git else { return empty };
    let local_ref = format!("refs/heads/{}", dev_branch);
    let Some(tip) = git_sync::rev_parse(git, root, &local_ref).ok().flatten() else {
        return empty;
    };
    let known: std::collections::HashSet<String> = crate::sync::skills::list_skill_templates(db)
        .map(|v| v.into_iter().map(|(name, _)| name).collect())
        .unwrap_or_default();
    let known_fn = |r: &str| known.contains(r);
    let mut issues = Vec::new();
    let (dev_p, dev_d) =
        crate::sync::shared::read_proposal_catalogs(git, root, &tip, &known_fn, &mut issues);
    let origin_ref = format!("refs/remotes/origin/{}", dev_branch);
    let (origin_p, _) = if matches!(git_sync::rev_parse(git, root, &origin_ref), Ok(Some(_))) {
        let mut skip = Vec::new();
        crate::sync::shared::read_proposal_catalogs(git, root, &origin_ref, &known_fn, &mut skip)
    } else {
        (
            std::collections::BTreeMap::new(),
            std::collections::BTreeMap::new(),
        )
    };
    (dev_p, dev_d, origin_p)
}

/// Contributor: own proposal rows joined with what the dev tip and the
/// last-seen origin snapshot carry — `on_dev`/`on_origin` are the UI's
/// "sent / confirmed" markers.
pub(crate) fn list_contributor_proposals(
    db: &AppDb,
    git: Option<&GitBinary>,
    repo: &crate::models::Repository,
) -> Result<Vec<RuleProposalView>, String> {
    let root = repo_root(repo).ok();
    let dev_branch = root.as_deref().and_then(|r| dev_branch_for(db, repo.id, r));
    let (dev_p, _dev_d, origin_p) = match (root.as_deref(), dev_branch.as_deref()) {
        (Some(r), Some(b)) => catalogs(git, r, b, db),
        _ => Default::default(),
    };
    let mut out = Vec::new();
    for p in db.list_rule_proposals(repo.id).map_err(|e| e.to_string())? {
        let wire = RuleProposalFile {
            format: crate::models::PROPOSAL_FORMAT,
            login: p.login.clone(),
            rule: p.rule.clone(),
            text: p.text.clone(),
            at: p.at.clone(),
            seq: p.seq,
            status: p.status.clone(),
        };
        let rev = match (git, root.as_deref()) {
            (Some(g), Some(r)) => local_rev(g, r, &wire),
            _ => None,
        };
        let on_dev = rev
            .as_deref()
            .is_some_and(|sha| dev_p.get(&p.file_name).is_some_and(|(s, _)| s == sha));
        let on_origin = rev
            .as_deref()
            .is_some_and(|sha| origin_p.get(&p.file_name).is_some_and(|(s, _)| s == sha));
        out.push(RuleProposalView {
            file_name: p.file_name.clone(),
            rule: p.rule.clone(),
            login: p.login.clone(),
            seq: p.seq,
            status: p.status.clone(),
            text: p.text.clone(),
            at: p.at.clone(),
            rev,
            on_dev,
            on_origin,
            local_state: Some(p.local_state.clone()),
            resolved_action: p.resolved_action.clone(),
            resolved_by: p.resolved_by.clone(),
            resolved_at: p.resolved_at.clone(),
            acceptance_status: None,
            acceptance_pub_state: None,
            acceptance_action: None,
        });
    }
    Ok(out)
}

/// Owner: the proposals the local dev tip carries — unpushed ones
/// included — each resolved against the dev receipts by exact blob sha and
/// joined with this install's acceptance row (the `queued` marker).
pub(crate) fn list_owner_proposals(
    db: &AppDb,
    git: Option<&GitBinary>,
    repo: &crate::models::Repository,
) -> Result<Vec<RuleProposalView>, String> {
    let root = repo_root(repo)?;
    let dev_branch = dev_branch_for(db, repo.id, &root)
        .ok_or_else(|| "the shared dev branch is not known".to_string())?;
    let (dev_p, dev_d, _origin_p) = catalogs(git, &root, &dev_branch, db);
    let acceptances = db.list_rule_acceptances(repo.id).unwrap_or_default();
    let mut out = Vec::new();
    for (file_name, (sha, p)) in &dev_p {
        let receipt = dev_d
            .values()
            .find(|(_, d)| ss::logins_equal(&d.login, &p.login) && d.decided_rev == *sha);
        let acc = acceptances.iter().find(|a| {
            a.rule == p.rule
                && ss::logins_equal(&a.proposer_login, &p.login)
                && a.decided_rev == *sha
        });
        out.push(RuleProposalView {
            file_name: file_name.clone(),
            rule: p.rule.clone(),
            login: p.login.clone(),
            seq: p.seq,
            status: p.status.clone(),
            text: p.text.clone(),
            at: p.at.clone(),
            rev: Some(sha.clone()),
            on_dev: true,
            on_origin: false,
            local_state: None,
            resolved_action: receipt.map(|(_, d)| d.action.clone()),
            resolved_by: receipt.map(|(_, d)| d.decided_by.clone()),
            resolved_at: receipt.map(|(_, d)| d.decided_at.clone()),
            acceptance_status: acc.map(|a| a.status.clone()),
            acceptance_pub_state: acc.map(|a| a.pub_state.clone()),
            acceptance_action: acc.map(|a| a.action.clone()),
        });
    }
    Ok(out)
}

/// The proposals of one shared repo — the owner's read comes off the local
/// dev tip, the contributor's off its own rows; either way the sweeps ran
/// first so the answer is the freshest the machine could reach.
#[tauri::command]
pub fn list_rule_proposals(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<RuleProposalView>, String> {
    let git = crate::git_ops::cached_git();
    let claude = dirs::home_dir().map(|h| h.join(".claude"));
    sweep_repo_rule_channel(&db, git.as_ref(), repo_id, claude.as_deref());
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let state = db.get_shared_state(repo_id).map_err(|e| e.to_string())?;
    if state.shared_mode && state.shared_role.as_deref() == Some("main") {
        return list_owner_proposals(&db, git.as_ref(), &repo);
    }
    list_contributor_proposals(&db, git.as_ref(), &repo)
}

/// Owner: the decision pipeline rows for the repo — the sweeps run first
/// so unfinished rows resume before they are shown.
#[tauri::command]
pub fn list_rule_acceptances(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<RuleAcceptance>, String> {
    let git = crate::git_ops::cached_git();
    let claude = dirs::home_dir().map(|h| h.join(".claude"));
    sweep_repo_rule_channel(&db, git.as_ref(), repo_id, claude.as_deref());
    db.list_rule_acceptances(repo_id).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Withdraw (contributor)
// ---------------------------------------------------------------------------

/// Flip the proposal to `withdrawn`; the transport uploads it on the next
/// pass. Only the proposal's owner may withdraw it.
#[tauri::command]
pub fn withdraw_rule_proposal(
    db: State<AppDb>,
    repo_id: i64,
    file_name: String,
) -> Result<(), String> {
    let p = db
        .get_rule_proposal(repo_id, &file_name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{}: no such proposal", file_name))?;
    if p.status != proposal_status::OPEN {
        return Err(format!("{}: not open", file_name));
    }
    db.set_rule_proposal_status(repo_id, &file_name, proposal_status::WITHDRAWN)
        .map_err(|e| e.to_string())?;
    let _ = db.insert_rule_event(
        &p.rule,
        crate::db::rule_event::PROPOSAL_WITHDRAWN,
        None,
        Some(&file_name),
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// The decision machine (owner): accept / reject, strict steps per spec R4.
// ---------------------------------------------------------------------------

/// Step 0+1: find or prepare the acceptance row. Returns the row to resume
/// from — an existing one continues from its own status, never re-runs the
/// steps already done.
pub(crate) fn prepare_acceptance(
    db: &AppDb,
    git: &GitBinary,
    root: &Path,
    dev_rev: &str,
    dev_tip: &str,
    repo_id: i64,
    rule: &str,
    proposer_login: &str,
    expected_rev: &str,
    action: &str,
) -> Result<RuleAcceptance, String> {
    if let Some(a) = db
        .get_rule_acceptance(repo_id, rule, proposer_login, expected_rev)
        .map_err(|e| e.to_string())?
    {
        if a.action != action {
            return Err(format!(
                "already decided '{}' for this revision; cannot {} it",
                a.action, action
            ));
        }
        return Ok(a);
    }
    // New key — the receipt-path pre-check at the pinned dev tip.
    let rname = rp::decision_file_name(rule, proposer_login, expected_rev);
    let rpath = format!("{}/{}", rp::RULE_DECISIONS_REL_DIR, rname);
    if let Some(sha) = git_sync::blob_sha_at(git, root, dev_rev, &rpath)? {
        let bytes = git_sync::show_file_capped(git, root, dev_rev, &rpath, rp::MAX_ARTIFACT_BYTES)?
            .ok_or_else(|| format!("{}: unreadable", rpath))?;
        let d: RuleDecisionFile =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {}", rpath, e))?;
        let _ = sha;
        if d.action != action {
            return Err(format!(
                "{}: an immutable receipt holds action '{}' — never overwritten",
                rpath, d.action
            ));
        }
        // Identical receipt: create the row decided and resume at canon.
        let now = crate::db::utc_now_rfc3339();
        let a = RuleAcceptance {
            repo_id,
            rule: rule.to_string(),
            proposer_login: proposer_login.to_string(),
            decided_rev: expected_rev.to_string(),
            action: action.to_string(),
            receipt_json: serde_json::to_string(&d).unwrap_or_default(),
            proposal_text: read_proposal_text(git, root, dev_rev, rule, proposer_login)?,
            status: acceptance_status::DECIDED.to_string(),
            pub_state: acceptance_pub_state::PENDING.to_string(),
            base_canon_hash: String::new(),
            canon_hash: None,
            created_at: now.clone(),
            updated_at: now,
        };
        db.insert_rule_acceptance(&a).map_err(|e| e.to_string())?;
        return Ok(a);
    }
    // Absent — snapshot the open version; a moved revision refuses.
    let (path, sha) = proposal_at(git, root, dev_rev, rule, proposer_login)?
        .ok_or_else(|| format!("{}: no open proposal on dev", rule))?;
    if sha != expected_rev {
        return Err(ERR_PROPOSAL_CHANGED.to_string());
    }
    let bytes = git_sync::show_file_capped(git, root, dev_rev, &path, rp::MAX_ARTIFACT_BYTES)?
        .ok_or_else(|| format!("{}: unreadable", path))?;
    let p: RuleProposalFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {}", path, e))?;
    if p.status != proposal_status::OPEN {
        return Err(ERR_PROPOSAL_CHANGED.to_string());
    }
    let canon = db
        .get_template_file("_global", &format!("skill.{}.md.tmpl", rule))
        .ok()
        .flatten()
        .map(|t| t.content)
        .unwrap_or_default();
    // Wire logins are lowercase — the proposal file names and the receipt's
    // `decided_by` follow the same grammar.
    let decided_by = db.github_login().unwrap_or_default().to_ascii_lowercase();
    let receipt = RuleDecisionFile {
        format: 1,
        action: action.to_string(),
        login: proposer_login.to_string(),
        rule: rule.to_string(),
        decided_by,
        decided_at: crate::db::utc_now_rfc3339(),
        decided_rev: expected_rev.to_string(),
        seq: p.seq,
    };
    let now = crate::db::utc_now_rfc3339();
    let a = RuleAcceptance {
        repo_id,
        rule: rule.to_string(),
        proposer_login: proposer_login.to_string(),
        decided_rev: expected_rev.to_string(),
        action: action.to_string(),
        receipt_json: serde_json::to_string(&receipt).map_err(|e| e.to_string())?,
        proposal_text: p.text.clone(),
        status: acceptance_status::PREPARED.to_string(),
        pub_state: acceptance_pub_state::PENDING.to_string(),
        base_canon_hash: rp::sha256_hex(canon.as_bytes()),
        canon_hash: None,
        created_at: now.clone(),
        updated_at: now,
    };
    db.insert_rule_acceptance(&a).map_err(|e| e.to_string())?;
    let _ = dev_tip;
    Ok(a)
}

pub(crate) fn read_proposal_text(
    git: &GitBinary,
    root: &Path,
    rev: &str,
    rule: &str,
    login: &str,
) -> Result<String, String> {
    let (path, _) = proposal_at(git, root, rev, rule, login)?
        .ok_or_else(|| format!("{}: proposal gone", rule))?;
    let bytes = git_sync::show_file_capped(git, root, rev, &path, rp::MAX_ARTIFACT_BYTES)?
        .ok_or_else(|| format!("{}: unreadable", path))?;
    let p: RuleProposalFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {}", path, e))?;
    Ok(p.text)
}

/// Step 2 — RECEIPT inside the repo's policy guard (the per-repo git
/// critical section): identical → decided; conflicting → error, never
/// overwritten; absent → revalidate the open revision, commit the receipt.
pub(crate) fn publish_receipt(
    db: &AppDb,
    git: &GitBinary,
    root: &Path,
    repo_id: i64,
    dev_branch: &str,
    a: &RuleAcceptance,
) -> Result<RuleAcceptance, String> {
    let _guard = crate::sync::shared::specs_policy_guard(repo_id);
    let _hold = _guard.lock().unwrap();
    let dev_rev = format!("refs/heads/{}", dev_branch);
    let rname = rp::decision_file_name(&a.rule, &a.proposer_login, &a.decided_rev);
    let rpath = format!("{}/{}", rp::RULE_DECISIONS_REL_DIR, rname);
    match git_sync::blob_sha_at(git, root, &dev_rev, &rpath)? {
        Some(sha) => {
            let bytes =
                git_sync::show_file_capped(git, root, &dev_rev, &rpath, rp::MAX_ARTIFACT_BYTES)?
                    .ok_or_else(|| format!("{}: unreadable", rpath))?;
            let text = String::from_utf8_lossy(&bytes).to_string();
            if text.trim() == a.receipt_json.trim() {
                // Identical already there — a crash between commit and the
                // status write resumes here.
                let mut a = a.clone();
                set_status(db, &mut a, acceptance_status::DECIDED)?;
                return Ok(a);
            }
            return Err(format!(
                "{}: an immutable receipt holds other bytes ({}): never overwritten",
                rpath, sha
            ));
        }
        None => {}
    }
    // Absent — the open revision must still be what was decided on.
    let (_, sha) = proposal_at(git, root, &dev_rev, &a.rule, &a.proposer_login)?
        .ok_or_else(|| format!("{}: proposal gone", a.rule))?;
    if sha != a.decided_rev {
        let mut a = a.clone();
        set_status(db, &mut a, acceptance_status::EXPIRED)?;
        return Ok(a);
    }
    // Commit the receipt to dev.
    let change = FileChange {
        rel_path: PathBuf::from(&rpath),
        content: Some(a.receipt_json.clone().into_bytes()),
    };
    let subject = "chore(sdh-sync): rule decisions";
    let body = format!(
        "Rule proposal decision written by Solo Dev Hub.\n\nSDH-Participant: {}",
        db.github_login().unwrap_or_default()
    );
    if current_branch(git, root)
        .ok()
        .flatten()
        .as_deref()
        .is_some_and(|c| c.eq_ignore_ascii_case(dev_branch))
    {
        let full = root.join(&rpath);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&full, &a.receipt_json).map_err(|e| e.to_string())?;
        commit_paths(
            git,
            root,
            dev_branch,
            &[PathBuf::from(&rpath)],
            subject,
            Some(&body),
            crate::sync::autocommit::SDH_COMMIT_NAME,
            crate::sync::autocommit::SDH_COMMIT_EMAIL,
        )
        .map_err(|e| e.to_string())?;
    } else {
        git_sync::refuse_checked_out(git, root, dev_branch, "the receipt retries later")
            .map_err(|e| format!("shared_blocked: {}", e))?;
        git_sync::commit_files_to_branch(
            git,
            root,
            dev_branch,
            &[change],
            &format!("{}\n\n{}", subject, body),
            crate::sync::autocommit::SDH_COMMIT_NAME,
            crate::sync::autocommit::SDH_COMMIT_EMAIL,
        )
        .map_err(|e| e.to_string())?;
    }
    let mut a = a.clone();
    set_status(db, &mut a, acceptance_status::DECIDED)?;
    Ok(a)
}

fn set_status(db: &AppDb, a: &mut RuleAcceptance, status: &str) -> Result<(), String> {
    db.set_rule_acceptance_status(
        a.repo_id,
        &a.rule,
        &a.proposer_login,
        &a.decided_rev,
        status,
        a.canon_hash.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    a.status = status.to_string();
    Ok(())
}

/// Steps 3+4 — canon + delivery (accept only). Returns the updated row.
/// A `reject` row skips both steps entirely: decided is where its content
/// work ends, and the pipeline marks it `rendered` — finished — so the
/// local-work sweep stops picking it up. The receipt's own journey lives
/// in `pub_state` and keeps publishing either way.
pub(crate) fn apply_decision(
    db: &AppDb,
    a: &RuleAcceptance,
    claude_dir: Option<&Path>,
) -> Result<RuleAcceptance, String> {
    let mut a = a.clone();
    if a.action != decision_action::ACCEPT {
        if a.status == acceptance_status::DECIDED {
            set_status(db, &mut a, acceptance_status::RENDERED)?;
            let note = format!("receipt {}", a.decided_rev);
            let _ = db.insert_rule_event(
                &a.rule,
                crate::db::rule_event::PROPOSAL_REJECTED,
                None,
                Some(&note),
            );
        }
        return Ok(a); // reject: no content delivery, ever.
    }
    if matches!(
        a.status.as_str(),
        s if s == acceptance_status::APPLIED || s == acceptance_status::RENDERED
            || s == acceptance_status::CONFLICTED || s == acceptance_status::SUPERSEDED
    ) {
        if a.status != acceptance_status::APPLIED {
            return Ok(a);
        }
        // applied → delivery below.
    }
    if a.status == acceptance_status::DECIDED {
        // Step 3: canon, atomically with the status + history event.
        let file_name = format!("skill.{}.md.tmpl", a.rule);
        let current = db
            .get_template_file("_global", &file_name)
            .ok()
            .flatten()
            .map(|t| t.content)
            .unwrap_or_default();
        let cur_hash = rp::sha256_hex(current.as_bytes());
        let prop_hash = rp::sha256_hex(a.proposal_text.as_bytes());
        let new_status = if cur_hash == a.base_canon_hash || cur_hash == prop_hash {
            acceptance_status::APPLIED
        } else {
            acceptance_status::CONFLICTED
        };
        if new_status == acceptance_status::APPLIED && cur_hash != prop_hash {
            db.upsert_template_file("_global", &file_name, &a.proposal_text, true)
                .map_err(|e| e.to_string())?;
        }
        let canon_hash = rp::sha256_hex(a.proposal_text.as_bytes());
        db.set_rule_acceptance_status(
            a.repo_id,
            &a.rule,
            &a.proposer_login,
            &a.decided_rev,
            new_status,
            Some(&canon_hash),
        )
        .map_err(|e| e.to_string())?;
        a.status = new_status.to_string();
        a.canon_hash = Some(canon_hash);
        let note = format!("receipt {}", a.decided_rev);
        let _ = db.insert_rule_event(
            &a.rule,
            crate::db::rule_event::PROPOSAL_ACCEPTED,
            None,
            Some(&note),
        );
        if a.status == acceptance_status::CONFLICTED {
            return Ok(a); // Stable: explicit UI actions only from here.
        }
    }
    if a.status == acceptance_status::APPLIED {
        // Step 4: delivery — render-only, no collect.
        let report = crate::sync::skills::deliver_rules_only(db, claude_dir);
        if report.errors.is_empty() {
            let mut a = a.clone();
            set_status(db, &mut a, acceptance_status::RENDERED)?;
            return Ok(a);
        }
        // Real I/O failure — retried by the sweep; the status stays applied.
    }
    Ok(a)
}

/// The decision machine's body, shared by the command and the tests:
/// `(repo_id, rule, proposer_login, expected_rev, action)` — `expected_rev`
/// is the blob sha of the open proposal version the caller saw, the CAS of
/// the decision.
pub(crate) fn decide(
    db: &AppDb,
    repo_id: i64,
    rule: &str,
    proposer_login: &str,
    expected_rev: &str,
    action: &str,
) -> Result<RuleAcceptance, String> {
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    let git = git()?;
    let dev_branch = dev_branch_for(db, repo_id, &root)
        .ok_or_else(|| "the shared dev branch is not known".to_string())?;
    let dev_rev = format!("refs/heads/{}", dev_branch);
    let tip = git_sync::rev_parse(&git, &root, &dev_rev)?
        .ok_or_else(|| format!("{}: no local dev branch", dev_branch))?;

    let a = prepare_acceptance(
        db,
        &git,
        &root,
        &dev_rev,
        &tip,
        repo_id,
        rule,
        proposer_login,
        expected_rev,
        action,
    )?;
    let a = if a.status == acceptance_status::PREPARED {
        publish_receipt(db, &git, &root, repo_id, &dev_branch, &a)?
    } else {
        a
    };
    apply_decision(db, &a, dirs::home_dir().as_deref())
}

/// The owner decision command.
#[tauri::command(async)]
pub fn decide_rule_proposal(
    db: State<AppDb>,
    repo_id: i64,
    rule: String,
    proposer_login: String,
    expected_rev: String,
    action: String,
) -> Result<RuleAcceptance, String> {
    if action != decision_action::ACCEPT && action != decision_action::REJECT {
        return Err(format!("action must be accept|reject, got {}", action));
    }
    decide(&db, repo_id, &rule, &proposer_login, &expected_rev, &action)
}

/// The explicit actions a `conflicted` acceptance waits for — there is no
/// automatic transition out of a canon divergence:
///
/// - `apply`   — force the proposal's snapshot over the diverged canon
///               (forced txn, then delivery resumes toward `rendered`);
/// - `dismiss` — leave the local application (`superseded`); the receipt
///               keeps publishing regardless — `pub_state` lives its own life.
#[tauri::command]
pub fn resolve_rule_acceptance_conflict(
    db: State<AppDb>,
    repo_id: i64,
    rule: String,
    proposer_login: String,
    decided_rev: String,
    resolution: String,
) -> Result<RuleAcceptance, String> {
    let a = db
        .get_rule_acceptance(repo_id, &rule, &proposer_login, &decided_rev)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{}.{}: no such acceptance", rule, proposer_login))?;
    if a.status != acceptance_status::CONFLICTED {
        return Err(format!(
            "{}.{}: not conflicted (status {})",
            rule, proposer_login, a.status
        ));
    }
    match resolution.as_str() {
        "apply" => {
            // The forced txn the conflicted state was waiting for: the
            // snapshot goes over whatever the canon holds now.
            let file_name = format!("skill.{}.md.tmpl", a.rule);
            db.upsert_template_file("_global", &file_name, &a.proposal_text, true)
                .map_err(|e| e.to_string())?;
            let canon_hash = rp::sha256_hex(a.proposal_text.as_bytes());
            let mut a = a.clone();
            db.set_rule_acceptance_status(
                repo_id,
                &rule,
                &proposer_login,
                &decided_rev,
                acceptance_status::APPLIED,
                Some(&canon_hash),
            )
            .map_err(|e| e.to_string())?;
            a.status = acceptance_status::APPLIED.to_string();
            a.canon_hash = Some(canon_hash);
            apply_decision(&db, &a, dirs::home_dir().as_deref())
        }
        "dismiss" => {
            let mut a = a.clone();
            set_status(&db, &mut a, acceptance_status::SUPERSEDED)?;
            Ok(a)
        }
        other => Err(format!("resolution must be apply|dismiss, got {}", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleProposal;

    fn db() -> AppDb {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        db.upsert_template_file("_global", "skill.sdh-alpha.md.tmpl", "CANON", false)
            .unwrap();
        db
    }

    fn acceptance(status: &str, pub_state: &str, action: &str, rev: &str) -> RuleAcceptance {
        RuleAcceptance {
            repo_id: 1,
            rule: "sdh-alpha".into(),
            proposer_login: "helper".into(),
            decided_rev: rev.into(),
            action: action.into(),
            receipt_json: "{}".into(),
            proposal_text: "PROPOSED".into(),
            status: status.into(),
            pub_state: pub_state.into(),
            base_canon_hash: rp::sha256_hex(b"CANON"),
            canon_hash: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn proposal(status: &str, resolved: Option<&str>, remote_ver: Option<&str>) -> RuleProposal {
        RuleProposal {
            repo_id: 1,
            file_name: "sdh-alpha.helper.json".into(),
            login: "helper".into(),
            rule: "sdh-alpha".into(),
            seq: 3,
            status: status.into(),
            local_state: "applied".into(),
            text: "T".into(),
            at: "2026-11-08T00:00:00Z".into(),
            resolved_action: resolved.map(str::to_string),
            resolved_by: None,
            resolved_at: None,
            resolved_rev: None,
            remote_ver: remote_ver.map(str::to_string),
            updated_at: String::new(),
        }
    }

    #[test]
    fn the_gc_drops_only_old_terminal_rows_whose_remote_dominates() {
        let db = db();
        let file = "sdh-alpha.helper.json";
        // Withdrawn + dominating remote + old → dropped; seq survives.
        db.upsert_rule_proposal(&proposal(
            proposal_status::WITHDRAWN,
            None,
            Some("{\"seq\":3,\"status\":\"withdrawn\",\"sha\":\"x\"}"),
        ))
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE rule_proposals SET updated_at = '2026-09-01T00:00:00Z' WHERE repo_id = 1",
                [],
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO rule_proposal_seq (repo_id, file_name, max_seq) VALUES (1, ?1, 7)",
                [file],
            )
            .unwrap();
        assert_eq!(gc_terminal_proposals(&db, 1), 1);
        assert!(db.get_rule_proposal(1, file).unwrap().is_none());
        assert_eq!(db.rule_proposal_max_seq(1, file).unwrap(), 7);

        // Fresh terminal row → kept (under 30 days).
        db.upsert_rule_proposal(&proposal(
            proposal_status::WITHDRAWN,
            None,
            Some("{\"seq\":3,\"status\":\"withdrawn\",\"sha\":\"x\"}"),
        ))
        .unwrap();
        assert_eq!(gc_terminal_proposals(&db, 1), 0);
        // Older remote (seq below the row's) → kept.
        db.set_rule_proposal_remote_ver(
            1,
            file,
            Some("{\"seq\":2,\"status\":\"open\",\"sha\":\"y\"}"),
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE rule_proposals SET updated_at = '2026-09-01T00:00:00Z' WHERE repo_id = 1",
                [],
            )
            .unwrap();
        assert_eq!(gc_terminal_proposals(&db, 1), 0);
        // An open row is never terminal.
        db.upsert_rule_proposal(&proposal(proposal_status::OPEN, None, None))
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE rule_proposals SET updated_at = '2026-09-01T00:00:00Z' WHERE repo_id = 1",
                [],
            )
            .unwrap();
        assert_eq!(gc_terminal_proposals(&db, 1), 0);
        // A resolved row whose remote confirmed the version → dropped.
        db.upsert_rule_proposal(&proposal(
            proposal_status::OPEN,
            Some("reject"),
            Some("{\"seq\":3,\"status\":\"open\",\"sha\":\"x\"}"),
        ))
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE rule_proposals SET updated_at = '2026-09-01T00:00:00Z' WHERE repo_id = 1",
                [],
            )
            .unwrap();
        assert_eq!(gc_terminal_proposals(&db, 1), 1);
    }

    #[test]
    fn the_sweep_applies_pending_proposals_to_the_canon() {
        let db = db();
        let mut p = proposal(proposal_status::OPEN, None, None);
        p.local_state = crate::models::proposal_local_state::PENDING_APPLY.to_string();
        p.text = "NEW CANON".into();
        db.upsert_rule_proposal(&p).unwrap();
        assert_eq!(sweep_pending_apply(&db, 1), 1);
        let canon = db
            .get_template_file("_global", "skill.sdh-alpha.md.tmpl")
            .unwrap()
            .unwrap();
        assert_eq!(canon.content, "NEW CANON");
        assert_eq!(
            db.get_rule_proposal(1, "sdh-alpha.helper.json")
                .unwrap()
                .unwrap()
                .local_state,
            "applied"
        );
        // Idempotent: nothing left to do.
        assert_eq!(sweep_pending_apply(&db, 1), 0);
    }

    #[test]
    fn a_rejected_decision_finishes_rendered_and_never_touches_the_canon() {
        let db = db();
        let rev = "a".repeat(40);
        db.insert_rule_acceptance(&acceptance(
            acceptance_status::DECIDED,
            acceptance_pub_state::PENDING,
            decision_action::REJECT,
            &rev,
        ))
        .unwrap();
        let a = db
            .get_rule_acceptance(1, "sdh-alpha", "helper", &rev)
            .unwrap()
            .unwrap();
        let a = apply_decision(&db, &a, None).unwrap();
        assert_eq!(a.status, acceptance_status::RENDERED);
        assert_eq!(a.pub_state, acceptance_pub_state::PENDING);
        let canon = db
            .get_template_file("_global", "skill.sdh-alpha.md.tmpl")
            .unwrap()
            .unwrap();
        assert_eq!(canon.content, "CANON");
        // The rejection is in the history once.
        let events = db.list_rule_events("sdh-alpha").unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == crate::db::rule_event::PROPOSAL_REJECTED)
                .count(),
            1
        );
    }

    #[test]
    fn a_conflicted_acceptance_only_moves_on_an_explicit_action() {
        let db = db();
        let rev = "b".repeat(40);
        // The canon diverged after the decision → conflicted is stable.
        db.upsert_template_file("_global", "skill.sdh-alpha.md.tmpl", "DIVERGED", true)
            .unwrap();
        db.insert_rule_acceptance(&acceptance(
            acceptance_status::DECIDED,
            acceptance_pub_state::PENDING,
            decision_action::ACCEPT,
            &rev,
        ))
        .unwrap();
        let a = db
            .get_rule_acceptance(1, "sdh-alpha", "helper", &rev)
            .unwrap()
            .unwrap();
        let a = apply_decision(&db, &a, None).unwrap();
        assert_eq!(a.status, acceptance_status::CONFLICTED);
        // The diverged canon is left alone until the owner chooses.
        assert_eq!(
            db.get_template_file("_global", "skill.sdh-alpha.md.tmpl")
                .unwrap()
                .unwrap()
                .content,
            "DIVERGED"
        );
    }

    #[test]
    fn dismiss_leaves_the_application_and_the_receipt_keeps_its_state() {
        let db = db();
        let rev = "c".repeat(40);
        db.insert_rule_acceptance(&acceptance(
            acceptance_status::CONFLICTED,
            acceptance_pub_state::PENDING,
            decision_action::ACCEPT,
            &rev,
        ))
        .unwrap();
        let a = db
            .get_rule_acceptance(1, "sdh-alpha", "helper", &rev)
            .unwrap()
            .unwrap();
        assert_eq!(a.status, acceptance_status::CONFLICTED);
        // `dismiss` → superseded; pub_state untouched.
        db.set_rule_acceptance_status(
            1,
            "sdh-alpha",
            "helper",
            &rev,
            acceptance_status::SUPERSEDED,
            None,
        )
        .unwrap();
        let a = db
            .get_rule_acceptance(1, "sdh-alpha", "helper", &rev)
            .unwrap()
            .unwrap();
        assert_eq!(a.status, acceptance_status::SUPERSEDED);
        assert_eq!(a.pub_state, acceptance_pub_state::PENDING);
        // Superseded stays publishable — its receipt must still reach dev.
        assert!(db
            .list_publishable_rule_acceptances()
            .unwrap()
            .iter()
            .any(|x| x.decided_rev == rev));
    }
}
