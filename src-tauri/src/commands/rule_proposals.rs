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
    RuleDecisionFile, RuleProposal, RuleProposalFile,
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
// Lists
// ---------------------------------------------------------------------------

/// Contributor: own proposal rows for the repo.
#[tauri::command]
pub fn list_rule_proposals(db: State<AppDb>, repo_id: i64) -> Result<Vec<RuleProposal>, String> {
    db.list_rule_proposals(repo_id).map_err(|e| e.to_string())
}

/// Owner: the decision pipeline rows for the repo.
#[tauri::command]
pub fn list_rule_acceptances(
    db: State<AppDb>,
    repo_id: i64,
) -> Result<Vec<RuleAcceptance>, String> {
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
fn prepare_acceptance(
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
    let decided_by = db.github_login().unwrap_or_default();
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

fn read_proposal_text(
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
fn publish_receipt(
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
fn apply_decision(
    db: &AppDb,
    a: &RuleAcceptance,
    claude_dir: Option<&Path>,
) -> Result<RuleAcceptance, String> {
    let mut a = a.clone();
    if a.action != decision_action::ACCEPT {
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

/// The owner decision command. `(repo_id, rule, proposer_login, expected_rev,
/// action)` — `expected_rev` is the blob sha of the open proposal version the
/// UI showed, CAS of the decision.
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
    let repo = db.get_repository(repo_id).map_err(|e| e.to_string())?;
    let root = repo_root(&repo)?;
    let git = git()?;
    let structure = load_structure(&root)?;
    let dev_branch = structure.main.dev_branch;
    let dev_rev = format!("refs/heads/{}", dev_branch);
    let tip = git_sync::rev_parse(&git, &root, &dev_rev)?
        .ok_or_else(|| format!("{}: no local dev branch", dev_branch))?;

    let a = prepare_acceptance(
        &db,
        &git,
        &root,
        &dev_rev,
        &tip,
        repo_id,
        &rule,
        &proposer_login,
        &expected_rev,
        &action,
    )?;
    let a = if a.status == acceptance_status::PREPARED {
        publish_receipt(&db, &git, &root, repo_id, &dev_branch, &a)?
    } else {
        a
    };
    let a = apply_decision(&db, &a, dirs::home_dir().as_deref())?;
    Ok(a)
}
