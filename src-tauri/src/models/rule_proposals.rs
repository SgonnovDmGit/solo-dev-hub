// T-000241 (v1.23.0): a contributor's approved rule copy becomes a proposal
// to the owner — applied locally at once, carried to the owner on the dev
// branch, and answered by an immutable decision receipt that travels back.
//
// Two wire artifacts, split by writer so no file ever has two authors:
// `.sdh/rule-proposals/<rule>.<login>.json` (the contributor, mutable — a
// re-proposal bumps seq) and `.sdh/rule-decisions/<rule>.<login>.<oid>.json`
// (the owner, immutable — one file per decision, named for the full blob sha
// of the open proposal version it closes).
//
// The local side lives in SQLite (migration v36): `rule_proposals` for the
// contributor's own proposals, `rule_acceptances` for the owner's decisions,
// `rule_proposal_seq` for the seq high-water mark that outlives the GC.

use serde::{Deserialize, Serialize};

/// Format version of both wire artifacts.
pub const PROPOSAL_FORMAT: u32 = 1;

/// `.sdh/rule-proposals/<rule>.<login>.json` — the contributor's artifact,
/// mutable across re-proposals (seq bumps once per logical version).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleProposalFile {
    pub format: u32,
    pub login: String,
    pub rule: String,
    pub text: String,
    /// RFC 3339 timestamp of this version.
    pub at: String,
    /// Logical version, i64 >= 0; assigned once at creation and copied
    /// unchanged through every upload / retry.
    pub seq: i64,
    /// `open` | `withdrawn` — never a decision; decisions live in receipts.
    pub status: String,
}

/// `.sdh/rule-decisions/<rule>.<login>.<oid>.json` — the owner's immutable
/// receipt; `<oid>` is the full 40-hex blob sha of the open proposal version
/// this receipt closes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleDecisionFile {
    pub format: u32,
    /// `accept` | `reject`.
    pub action: String,
    pub login: String,
    pub rule: String,
    pub decided_by: String,
    pub decided_at: String,
    /// Full blob sha (40 hex) of the open proposal version decided on.
    pub decided_rev: String,
    pub seq: i64,
}

/// The closed vocabulary of `rule_proposals.status` (wire `open` |
/// `withdrawn`).
pub mod proposal_status {
    pub const OPEN: &str = "open";
    pub const WITHDRAWN: &str = "withdrawn";
}

/// The closed vocabulary of `rule_proposals.local_state`: has this install
/// applied the proposal's text to its own canon yet.
pub mod proposal_local_state {
    pub const PENDING_APPLY: &str = "pending_apply";
    pub const APPLIED: &str = "applied";
}

/// The closed vocabulary of `rule_acceptances.action`.
pub mod decision_action {
    pub const ACCEPT: &str = "accept";
    pub const REJECT: &str = "reject";
}

/// The closed vocabulary of `rule_acceptances.status` — the local work
/// pipeline. `conflicted` / `expired` / `superseded` are stable side states:
/// `superseded` means the owner left the local application (receipt still
/// publishes); `expired` means the prepared decision found the open version
/// gone before its receipt was written.
pub mod acceptance_status {
    pub const PREPARED: &str = "prepared";
    pub const DECIDED: &str = "decided";
    pub const APPLIED: &str = "applied";
    pub const RENDERED: &str = "rendered";
    pub const CONFLICTED: &str = "conflicted";
    pub const EXPIRED: &str = "expired";
    pub const SUPERSEDED: &str = "superseded";
}

/// The closed vocabulary of `rule_acceptances.pub_state` — the receipt's own
/// journey to origin, orthogonal to `status` and never rolling it back.
pub mod acceptance_pub_state {
    pub const PENDING: &str = "pending";
    pub const BLOCKED: &str = "blocked";
    pub const PUBLISHED: &str = "published";
    /// The receipt's decided_rev is no longer the open version on dev — a
    /// re-proposal overtook it; the receipt itself still stands.
    pub const OVERTAKEN: &str = "overtaken";
}

/// A `rule_proposals` row — this install's own proposal plus the receipt
/// resolution overlay (`resolved_*`, never serialized onto the wire).
#[derive(Debug, Clone, PartialEq)]
pub struct RuleProposal {
    pub repo_id: i64,
    /// `<rule>.<login>.json` — the proposal file name on dev.
    pub file_name: String,
    pub login: String,
    pub rule: String,
    pub seq: i64,
    /// Wire status: `open` | `withdrawn`.
    pub status: String,
    /// `pending_apply` | `applied` — the local canon write.
    pub local_state: String,
    /// The proposed rule text.
    pub text: String,
    /// RFC 3339 timestamp of this version.
    pub at: String,
    /// Resolution overlay: the receipt matching THIS version, if any.
    pub resolved_action: Option<String>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<String>,
    pub resolved_rev: Option<String>,
    /// `{seq,status,sha}` JSON tuple of the last origin-confirmed dev
    /// version — the GC's dominance check.
    pub remote_ver: Option<String>,
    pub updated_at: String,
}

/// A `rule_acceptances` row — one immutable owner decision plus its local
/// work pipeline and publication state.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleAcceptance {
    pub repo_id: i64,
    pub rule: String,
    pub proposer_login: String,
    /// Full blob sha of the open proposal version decided on.
    pub decided_rev: String,
    /// `accept` | `reject`.
    pub action: String,
    /// The immutable receipt bytes, exactly as published.
    pub receipt_json: String,
    /// Validated snapshot of the proposal's text — the canon is applied
    /// FROM this, never by re-reading the mutable proposal file.
    pub proposal_text: String,
    /// prepared | decided | applied | rendered | conflicted | expired |
    /// superseded.
    pub status: String,
    /// pending | blocked | published | overtaken.
    pub pub_state: String,
    /// sha256 of the canon the decision was taken against.
    pub base_canon_hash: String,
    /// sha256 of the canon this row applied, once it did.
    pub canon_hash: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
