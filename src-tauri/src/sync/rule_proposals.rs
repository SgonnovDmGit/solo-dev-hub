// T-000241 (v1.23.0): wire artifacts of the contributor rule-proposal
// channel — two `.sdh/` catalogs on the shared repo's dev branch, each with
// exactly one writer so no file ever has two authors:
//
// - `.sdh/rule-proposals/<rule>.<login>.json` — the contributor's mutable
//   proposal (a re-proposal bumps `seq`; `status` flips to `withdrawn` on
//   self-termination). Decisions never live here.
// - `.sdh/rule-decisions/<rule>.<login>.<oid>.json` — the owner's immutable
//   receipt, one file per decision, named for the FULL blob sha of the open
//   proposal version it closes, so a retry reuses the path and a re-proposal
//   is never closed by an older receipt.
//
// Both catalogs are excluded from every pathspec and the generic plumbing:
// `sync_rule_proposals` in `shared.rs` is their dedicated transport on the
// dev branch, and each install's state lives in the v36 tables. This module
// owns the formats, the filename grammar, the validators and the
// version-resolution function — pure logic, no git or UI.

use crate::models::{
    decision_action, proposal_status, RuleDecisionFile, RuleProposalFile, PROPOSAL_FORMAT,
};
use crate::sync::shared_structure::is_valid_login;

/// The catalogs' repo-relative directories.
pub const RULE_PROPOSALS_REL_DIR: &str = ".sdh/rule-proposals";
pub const RULE_DECISIONS_REL_DIR: &str = ".sdh/rule-decisions";

/// Largest artifact that is ever read — bounded BEFORE allocation, same cap
/// the structure file already uses via `show_file_capped`.
pub const MAX_ARTIFACT_BYTES: u64 = 64 * 1024;
/// Proposals inspected per sync (mutable, pruned by the GC).
pub const MAX_PROPOSAL_FILES: usize = 200;
/// Receipts inspected per sync (immutable, they accumulate) and the cap on
/// NEW receipt paths — creation past it is a hard error before side effects,
/// while reading, reusing and confirming an existing receipt always works.
pub const MAX_DECISION_FILES: usize = 2000;

// ---------------------------------------------------------------------------
// Filenames
// ---------------------------------------------------------------------------

/// `<rule>.<login>.json` — the proposal file of `login` for `rule`.
pub fn proposal_file_name(rule: &str, login: &str) -> String {
    format!("{}.{}.json", rule, login.to_ascii_lowercase())
}

/// `<rule>.<login>.<oid>.json` — the receipt closing the open proposal of
/// `login` for `rule` whose blob sha is `oid` (full, 40 hex).
pub fn decision_file_name(rule: &str, login: &str, oid: &str) -> String {
    format!("{}.{}.{}.json", rule, login.to_ascii_lowercase(), oid)
}

/// Parse `<rule>.<login>.json` into `(rule, login)`. `None` for anything that
/// is not exactly two segments — a receipt name has three, and rules never
/// contain dots, so it is rejected here by construction.
pub fn parse_proposal_file_name(name: &str) -> Option<(String, String)> {
    let stem = name.strip_suffix(".json")?;
    let (rule, login) = stem.rsplit_once('.')?;
    if rule.is_empty() || rule.contains('.') || login.is_empty() || login.contains('.') {
        return None;
    }
    Some((rule.to_string(), login.to_string()))
}

/// Parse `<rule>.<login>.<oid>.json` into `(rule, login, oid)`. A proposal
/// name (two segments) is rejected by the same construction.
pub fn parse_decision_file_name(name: &str) -> Option<(String, String, String)> {
    let stem = name.strip_suffix(".json")?;
    // `<rule>.<login>.<oid>` — the rule may not contain dots; the oid is the
    // last segment, the login the one before it.
    let (head, oid) = stem.rsplit_once('.')?;
    let (rule, login) = head.rsplit_once('.')?;
    if rule.is_empty()
        || rule.contains('.')
        || login.is_empty()
        || login.contains('.')
        || oid.is_empty()
    {
        return None;
    }
    Some((rule.to_string(), login.to_string(), oid.to_string()))
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Is `s` a usable RFC 3339 timestamp? `chrono::DateTime::parse_from_rfc3339`
/// accepts the full grammar; anything else is refused.
fn is_rfc3339(s: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(s).is_ok()
}

/// Is `s` a full lowercase hex blob sha (40 chars)?
pub fn is_blob_sha(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Does `rule` name a known skill? The grammar is `sdh-*` without dots; the
/// caller passes the set of names this install actually has — a proposal
/// cannot invent a rule.
pub fn is_valid_rule_name(rule: &str, known: &dyn Fn(&str) -> bool) -> bool {
    rule.starts_with("sdh-") && !rule.contains('.') && rule.len() > 4 && known(rule)
}

/// Validate a parsed proposal against its filename and the known rules.
/// Every violation is an explicit error — the transport refuses malformed
/// artifacts rather than guessing.
pub fn validate_proposal_file(
    file_name: &str,
    p: &RuleProposalFile,
    known: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    if p.format != PROPOSAL_FORMAT {
        return Err(format!("{}: unsupported format {}", file_name, p.format));
    }
    let (rule, login) = parse_proposal_file_name(file_name)
        .ok_or_else(|| format!("{}: not a <rule>.<login>.json name", file_name))?;
    if !crate::sync::shared_structure::logins_equal(&p.login, &login) {
        return Err(format!(
            "{}: login {} does not match the filename",
            file_name, p.login
        ));
    }
    if p.rule != rule {
        return Err(format!(
            "{}: rule {} does not match the filename",
            file_name, p.rule
        ));
    }
    if !is_valid_rule_name(&p.rule, known) {
        return Err(format!("{}: unknown rule {}", file_name, p.rule));
    }
    if !is_valid_login(&p.login) {
        return Err(format!("{}: invalid login {}", file_name, p.login));
    }
    if p.seq < 0 {
        return Err(format!("{}: negative seq {}", file_name, p.seq));
    }
    if !matches!(
        p.status.as_str(),
        proposal_status::OPEN | proposal_status::WITHDRAWN
    ) {
        return Err(format!("{}: unknown status {}", file_name, p.status));
    }
    if !is_rfc3339(&p.at) {
        return Err(format!("{}: `at` is not RFC 3339: {}", file_name, p.at));
    }
    Ok(())
}

/// Validate a parsed receipt against its filename and the known rules. The
/// filename `<oid>` is the FULL blob sha of the decided proposal version and
/// must equal `decided_rev` — the receipt names what it closes.
pub fn validate_decision_file(
    file_name: &str,
    d: &RuleDecisionFile,
    known: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    if d.format != PROPOSAL_FORMAT {
        return Err(format!("{}: unsupported format {}", file_name, d.format));
    }
    let (rule, login, oid) = parse_decision_file_name(file_name)
        .ok_or_else(|| format!("{}: not a <rule>.<login>.<oid>.json name", file_name))?;
    if !is_blob_sha(&oid) {
        return Err(format!("{}: <oid> is not a 40-hex blob sha", file_name));
    }
    if !crate::sync::shared_structure::logins_equal(&d.login, &login) {
        return Err(format!(
            "{}: login {} does not match the filename",
            file_name, d.login
        ));
    }
    if d.rule != rule {
        return Err(format!(
            "{}: rule {} does not match the filename",
            file_name, d.rule
        ));
    }
    if !is_valid_rule_name(&d.rule, known) {
        return Err(format!("{}: unknown rule {}", file_name, d.rule));
    }
    if !is_valid_login(&d.login) || !is_valid_login(&d.decided_by) {
        return Err(format!("{}: invalid login", file_name));
    }
    if !matches!(
        d.action.as_str(),
        decision_action::ACCEPT | decision_action::REJECT
    ) {
        return Err(format!("{}: unknown action {}", file_name, d.action));
    }
    if d.decided_rev != oid {
        return Err(format!(
            "{}: decided_rev {} does not match the filename oid",
            file_name, d.decided_rev
        ));
    }
    if d.seq < 0 {
        return Err(format!("{}: negative seq {}", file_name, d.seq));
    }
    if !is_rfc3339(&d.decided_at) {
        return Err(format!(
            "{}: `decided_at` is not RFC 3339: {}",
            file_name, d.decided_at
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Effective state
// ---------------------------------------------------------------------------

/// The byte-stable canonical JSON of a proposal wire file — the wire fields
/// only, resolution metadata never enters the serialization. Two versions
/// whose canonical content is equal are the same version.
pub fn canonical_proposal_json(p: &RuleProposalFile) -> String {
    serde_json::to_string(p).expect("a proposal wire file serializes")
}

/// sha256 of `bytes`, lowercase hex — the digest the resolver uses for
/// canonical-content comparison (`decided_rev` instead compares FULL git
/// blob shas, produced by `git hash-object`).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// Which side of a proposal version wins: the local DB row's wire view or
/// the blob on the dev tip. Greater `seq` wins; equal seq prefers the
/// terminal status (`withdrawn` beats `open`); still equal means identical
/// canonical content or a sha tiebreak (the byte order decides — a tiebreak
/// only, both sides are then byte-comparable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effective {
    /// The dev version dominates — pull it into the local row.
    Remote,
    /// The local version dominates — upload it to dev.
    Local,
    /// Canonical content identical — nothing to do.
    Same,
}

/// `effective(local_wire, dev_wire)` for one proposal version.
pub fn effective_version(local: &RuleProposalFile, dev: &RuleProposalFile) -> Effective {
    if local.seq != dev.seq {
        return if local.seq > dev.seq {
            Effective::Local
        } else {
            Effective::Remote
        };
    }
    if local.status != dev.status {
        // Terminal beats open at equal seq.
        return if local.status == proposal_status::WITHDRAWN {
            Effective::Local
        } else {
            Effective::Remote
        };
    }
    let l = canonical_proposal_json(local);
    let d = canonical_proposal_json(dev);
    if l == d {
        return Effective::Same;
    }
    if sha256_hex(l.as_bytes()) >= sha256_hex(d.as_bytes()) {
        Effective::Local
    } else {
        Effective::Remote
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(seq: i64, status: &str, text: &str) -> RuleProposalFile {
        RuleProposalFile {
            format: PROPOSAL_FORMAT,
            login: "helper".into(),
            rule: "sdh-alpha".into(),
            text: text.into(),
            at: "2026-11-08T00:00:00Z".into(),
            seq,
            status: status.into(),
        }
    }

    fn decision(action: &str, rev: &str) -> RuleDecisionFile {
        RuleDecisionFile {
            format: PROPOSAL_FORMAT,
            action: action.into(),
            login: "helper".into(),
            rule: "sdh-alpha".into(),
            decided_by: "owner".into(),
            decided_at: "2026-11-08T00:00:00Z".into(),
            decided_rev: rev.into(),
            seq: 8,
        }
    }

    #[test]
    fn filenames_roundtrip_and_reject_each_others_grammar() {
        let p = proposal_file_name("sdh-alpha", "Helper");
        assert_eq!(p, "sdh-alpha.helper.json");
        assert_eq!(
            parse_proposal_file_name(&p),
            Some(("sdh-alpha".into(), "helper".into()))
        );
        let d = decision_file_name("sdh-alpha", "Helper", &"a".repeat(40));
        assert_eq!(d, format!("sdh-alpha.helper.{}.json", "a".repeat(40)));
        assert_eq!(
            parse_decision_file_name(&d),
            Some(("sdh-alpha".into(), "helper".into(), "a".repeat(40)))
        );
        // A proposal name is not a receipt name and vice versa.
        assert!(parse_decision_file_name(&p).is_none());
        assert!(parse_proposal_file_name(&d).is_none());
        assert!(parse_proposal_file_name("x").is_none());
        assert!(parse_proposal_file_name("a.b.c.d.json").is_none());
    }

    #[test]
    fn the_proposal_validator_enforces_every_field() {
        let known = |r: &str| r == "sdh-alpha";
        let name = "sdh-alpha.helper.json";
        let good = prop(1, "open", "TEXT");
        assert!(validate_proposal_file(name, &good, &known).is_ok());

        let mut bad = good.clone();
        bad.format = 2;
        assert!(validate_proposal_file(name, &bad, &known)
            .unwrap_err()
            .contains("format"));
        let mut bad = good.clone();
        bad.login = "other".into();
        assert!(validate_proposal_file(name, &bad, &known)
            .unwrap_err()
            .contains("login"));
        let mut bad = good.clone();
        bad.rule = "sdh-beta".into();
        assert!(validate_proposal_file(name, &bad, &known)
            .unwrap_err()
            .contains("rule"));
        let mut bad = good.clone();
        bad.seq = -1;
        assert!(validate_proposal_file(name, &bad, &known)
            .unwrap_err()
            .contains("seq"));
        let mut bad = good.clone();
        bad.status = "decided".into();
        assert!(validate_proposal_file(name, &bad, &known)
            .unwrap_err()
            .contains("status"));
        let mut bad = good.clone();
        bad.at = "not a date".into();
        assert!(validate_proposal_file(name, &bad, &known)
            .unwrap_err()
            .contains("RFC 3339"));
        // An unknown rule is refused — a proposal cannot invent one.
        assert!(validate_proposal_file(name, &good, &|_| false).is_err());
        // And a filename that does not parse is refused.
        assert!(validate_proposal_file("weird.json", &good, &known).is_err());
    }

    #[test]
    fn the_decision_validator_enforces_every_field() {
        let known = |r: &str| r == "sdh-alpha";
        let rev = "b".repeat(40);
        let name = format!("sdh-alpha.helper.{}.json", rev);
        let good = decision("accept", &rev);
        assert!(validate_decision_file(&name, &good, &known).is_ok());

        let mut bad = good.clone();
        bad.action = "maybe".into();
        assert!(validate_decision_file(&name, &bad, &known)
            .unwrap_err()
            .contains("action"));
        let mut bad = good.clone();
        bad.decided_rev = "c".repeat(40);
        assert!(validate_decision_file(&name, &bad, &known)
            .unwrap_err()
            .contains("decided_rev"));
        // A truncated oid in the NAME is refused outright.
        let short = decision_file_name("sdh-alpha", "helper", "abc123");
        assert!(validate_decision_file(&short, &good, &known)
            .unwrap_err()
            .contains("40-hex"));
        let mut bad = good.clone();
        bad.decided_by = "Bad Login!".into();
        assert!(validate_decision_file(&name, &bad, &known)
            .unwrap_err()
            .contains("login"));
        let mut bad = good.clone();
        bad.decided_at = "yesterday".into();
        assert!(validate_decision_file(&name, &bad, &known)
            .unwrap_err()
            .contains("RFC 3339"));
    }

    #[test]
    fn effective_resolves_seq_then_status_then_content() {
        let local = prop(3, "open", "LOCAL");
        assert_eq!(
            effective_version(&local, &prop(2, "open", "DEV")),
            Effective::Local
        );
        assert_eq!(
            effective_version(&prop(2, "open", "L"), &local),
            Effective::Remote
        );
        // Equal seq: the terminal status wins.
        assert_eq!(
            effective_version(&prop(3, "withdrawn", "L"), &local),
            Effective::Local
        );
        assert_eq!(
            effective_version(&local, &prop(3, "withdrawn", "D")),
            Effective::Remote
        );
        // Identical canonical content is a no-op.
        assert_eq!(
            effective_version(&local, &prop(3, "open", "LOCAL")),
            Effective::Same
        );
        // Same seq + status, different text: the sha order decides — never a
        // tie, never an error.
        let a = prop(3, "open", "AAA");
        let b = prop(3, "open", "BBB");
        assert_ne!(effective_version(&a, &b), effective_version(&b, &a));
    }
}
