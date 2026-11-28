// T-000241 (v1.23.0): the local side of the contributor rule-proposal
// channel (migration v36). The wire side is `.sdh/rule-proposals/` and
// `.sdh/rule-decisions/` on the shared repo's dev branch; these three tables
// are what each install remembers between syncs.
//
// - `rule_proposals` — this install's OWN proposals (contributor side):
//   the mutable version row plus the receipt resolution overlay.
// - `rule_proposal_seq` — seq high-water per proposal file; the GC may drop
//   a terminal row but never the high-water, so a re-proposal cannot reuse
//   a seq the remote once saw.
// - `rule_acceptances` — the owner's decisions (main side): one immutable
//   row per receipt, its local work pipeline (`status`) and the receipt's
//   own journey to origin (`pub_state`, orthogonal — never rolls `status`
//   back).

use super::*;
use crate::models::{RuleAcceptance, RuleProposal};
use rusqlite::OptionalExtension;

fn proposal_from_row(row: &Row) -> SqlResult<RuleProposal> {
    Ok(RuleProposal {
        repo_id: row.get(0)?,
        file_name: row.get(1)?,
        login: row.get(2)?,
        rule: row.get(3)?,
        seq: row.get(4)?,
        status: row.get(5)?,
        local_state: row.get(6)?,
        text: row.get(7)?,
        at: row.get(8)?,
        resolved_action: row.get(9)?,
        resolved_by: row.get(10)?,
        resolved_at: row.get(11)?,
        resolved_rev: row.get(12)?,
        remote_ver: row.get(13)?,
        updated_at: row.get(14)?,
        base_canon_hash: row.get(15)?,
    })
}

const PROPOSAL_COLS: &str = "repo_id, file_name, login, rule, seq, status, local_state,
     text, at, resolved_action, resolved_by, resolved_at, resolved_rev,
     remote_ver, updated_at, base_canon_hash";

fn acceptance_from_row(row: &Row) -> SqlResult<RuleAcceptance> {
    Ok(RuleAcceptance {
        repo_id: row.get(0)?,
        rule: row.get(1)?,
        proposer_login: row.get(2)?,
        decided_rev: row.get(3)?,
        action: row.get(4)?,
        receipt_json: row.get(5)?,
        proposal_text: row.get(6)?,
        status: row.get(7)?,
        pub_state: row.get(8)?,
        base_canon_hash: row.get(9)?,
        canon_hash: row.get(10)?,
        auto_decided: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

const ACCEPTANCE_COLS: &str = "repo_id, rule, proposer_login, decided_rev, action,
     receipt_json, proposal_text, status, pub_state, base_canon_hash,
     canon_hash, auto_decided, created_at, updated_at";

impl AppDb {
    // ---- rule_proposals ---------------------------------------------------

    /// This install's proposals for one shared repo, file name order.
    pub fn list_rule_proposals(&self, repo_id: i64) -> SqlResult<Vec<RuleProposal>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_proposals WHERE repo_id = ?1 ORDER BY file_name",
            PROPOSAL_COLS
        ))?;
        let rows = stmt.query_map([repo_id], proposal_from_row)?;
        rows.collect()
    }

    /// One proposal row by its file name.
    pub fn get_rule_proposal(
        &self,
        repo_id: i64,
        file_name: &str,
    ) -> SqlResult<Option<RuleProposal>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_proposals WHERE repo_id = ?1 AND file_name = ?2",
            PROPOSAL_COLS
        ))?;
        stmt.query_row(rusqlite::params![repo_id, file_name], proposal_from_row)
            .optional()
    }

    /// Insert or replace one proposal row wholesale (transport pull, local
    /// status flips). `updated_at` is stamped here.
    pub fn upsert_rule_proposal(&self, p: &RuleProposal) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO rule_proposals (repo_id, file_name, login, rule, seq, status,
                 local_state, text, at, resolved_action, resolved_by, resolved_at,
                 resolved_rev, remote_ver, updated_at, base_canon_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(repo_id, file_name) DO UPDATE SET
                 login=excluded.login, rule=excluded.rule, seq=excluded.seq,
                 status=excluded.status, local_state=excluded.local_state,
                 text=excluded.text, at=excluded.at,
                 resolved_action=excluded.resolved_action,
                 resolved_by=excluded.resolved_by,
                 resolved_at=excluded.resolved_at,
                 resolved_rev=excluded.resolved_rev,
                 remote_ver=excluded.remote_ver,
                 updated_at=excluded.updated_at,
                 base_canon_hash=excluded.base_canon_hash",
            rusqlite::params![
                p.repo_id,
                p.file_name,
                p.login,
                p.rule,
                p.seq,
                p.status,
                p.local_state,
                p.text,
                p.at,
                p.resolved_action,
                p.resolved_by,
                p.resolved_at,
                p.resolved_rev,
                p.remote_ver,
                utc_now_rfc3339(),
                p.base_canon_hash,
            ],
        )?;
        Ok(())
    }

    /// The receipt overlay of the inspected revision only. `None` clears the
    /// overlay — a fresh open version carries no resolution.
    pub fn set_rule_proposal_resolved(
        &self,
        repo_id: i64,
        file_name: &str,
        resolved: Option<(&str, &str, &str, &str)>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        let (action, by, at, rev) = match resolved {
            Some((a, b, t, r)) => (Some(a), Some(b), Some(t), Some(r)),
            None => (None, None, None, None),
        };
        conn.execute(
            "UPDATE rule_proposals
             SET resolved_action = ?3, resolved_by = ?4, resolved_at = ?5,
                 resolved_rev = ?6, updated_at = ?7
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name, action, by, at, rev, utc_now_rfc3339()],
        )?;
        Ok(())
    }

    /// The last origin-confirmed remote version tuple — GC bookkeeping only.
    pub fn set_rule_proposal_remote_ver(
        &self,
        repo_id: i64,
        file_name: &str,
        remote_ver: Option<&str>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_proposals SET remote_ver = ?3, updated_at = ?4
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name, remote_ver, utc_now_rfc3339()],
        )?;
        Ok(())
    }

    /// Terminal-row GC: drop a proposal row whose remote version dominates.
    /// The `rule_proposal_seq` high-water is a separate table and survives.
    pub fn delete_rule_proposal(&self, repo_id: i64, file_name: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM rule_proposals WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name],
        )?;
        Ok(())
    }

    /// Every proposal row not yet applied to the local canon — the
    /// local-work sweep's queue.
    pub fn list_pending_apply_proposals(&self) -> SqlResult<Vec<RuleProposal>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_proposals WHERE local_state = 'pending_apply'
             ORDER BY repo_id, file_name",
            PROPOSAL_COLS
        ))?;
        let rows = stmt.query_map([], proposal_from_row)?;
        rows.collect()
    }

    /// Everything of one repo — disable-shared cleanup.
    pub fn delete_rule_proposal_state(&self, repo_id: i64) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM rule_proposals WHERE repo_id = ?1", [repo_id])?;
        conn.execute(
            "DELETE FROM rule_proposal_seq WHERE repo_id = ?1",
            [repo_id],
        )?;
        conn.execute("DELETE FROM rule_acceptances WHERE repo_id = ?1", [repo_id])?;
        Ok(())
    }

    // ---- rule_proposal_seq -------------------------------------------------

    /// The seq high-water for a proposal file — 0 when never used.
    /// Read-side of `bump_rule_proposal_seq`'s write; surfaced for the tests
    /// that prove GC keeps the water high.
    #[allow(dead_code)]
    pub fn rule_proposal_max_seq(&self, repo_id: i64, file_name: &str) -> SqlResult<i64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT max_seq FROM rule_proposal_seq
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name],
            |r| r.get(0),
        )
        .optional()
        .map(|v| v.unwrap_or(0))
    }

    /// Claim the next seq for a proposal file: the high-water only ever
    /// rises, and a row deleted by the GC leaves it untouched.
    fn bump_rule_proposal_seq_on(
        conn: &Connection,
        repo_id: i64,
        file_name: &str,
    ) -> SqlResult<i64> {
        conn.execute(
            "INSERT INTO rule_proposal_seq (repo_id, file_name, max_seq)
             VALUES (?1, ?2, 1)
             ON CONFLICT(repo_id, file_name) DO UPDATE SET max_seq = max_seq + 1",
            rusqlite::params![repo_id, file_name],
        )?;
        conn.query_row(
            "SELECT max_seq FROM rule_proposal_seq
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name],
            |r| r.get(0),
        )
    }

    /// T-000241 R1: create (or re-propose) a proposal AND apply its text as
    /// the local canon, atomically — seq bump, proposal row (pending_apply →
    /// applied), the template upsert and the `taken` history event live or
    /// die together, so a crash cannot leave a canon with no proposal or a
    /// proposal that never landed. The event's note is formatted inside —
    /// `proposal to main <login> repo <id> seq <n>` — because the seq only
    /// exists after the bump. Returns the assigned seq.
    pub fn create_rule_proposal_txn(
        &self,
        repo_id: i64,
        file_name: &str,
        login: &str,
        rule: &str,
        text: &str,
        at: &str,
        event_repo: Option<&str>,
        main_login: &str,
    ) -> SqlResult<i64> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let seq = Self::bump_rule_proposal_seq_on(&tx, repo_id, file_name)?;
        // T-000264: the compare-and-swap base of this proposal chain. The
        // canon write below replaces the rule text with the proposal itself,
        // so hashing the canon AT re-proposal time would hash the previous
        // proposal — a base the owner's canon never matches, and every chain
        // link after the first would read as a spurious canon-diverged
        // conflict. The chain therefore inherits the base of its open
        // unresolved predecessor; a fresh chain (no row, or the last one
        // withdrawn / resolved) captures the canon as it stands BEFORE the
        // overwrite. No canon at all is no provable base — NULL, and the
        // proposal stays manual like every legacy one.
        let key = format!("skill.{}.md.tmpl", rule);
        let prior_base: Option<String> = tx
            .query_row(
                "SELECT base_canon_hash FROM rule_proposals
                 WHERE repo_id = ?1 AND file_name = ?2 AND status = 'open'",
                rusqlite::params![repo_id, file_name],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let canon_now: Option<String> = tx
            .query_row(
                "SELECT content FROM templates WHERE language_key = '_global' AND file_name = ?1",
                rusqlite::params![key],
                |r| r.get(0),
            )
            .optional()?;
        let base_canon_hash = prior_base.or_else(|| {
            canon_now.map(|c| {
                use sha2::Digest;
                let mut h = sha2::Sha256::new();
                h.update(c.as_bytes());
                format!("{:x}", h.finalize())
            })
        });
        tx.execute(
            "INSERT INTO rule_proposals (repo_id, file_name, login, rule, seq,
                 status, local_state, text, at, updated_at, base_canon_hash)
             VALUES (?1,?2,?3,?4,?5,'open','pending_apply',?6,?7,?8,?9)
             ON CONFLICT(repo_id, file_name) DO UPDATE SET
                 seq=excluded.seq, status=excluded.status,
                 local_state=excluded.local_state, text=excluded.text,
                 at=excluded.at, resolved_action=NULL, resolved_by=NULL,
                 resolved_at=NULL, resolved_rev=NULL, updated_at=excluded.updated_at,
                 base_canon_hash=excluded.base_canon_hash",
            rusqlite::params![
                repo_id,
                file_name,
                login,
                rule,
                seq,
                text,
                at,
                utc_now_rfc3339(),
                base_canon_hash,
            ],
        )?;
        // The canon write: the proposal's text is this install's rule now.
        tx.execute(
            "INSERT INTO templates (language_key, file_name, content, is_custom, updated_at)
             VALUES ('_global', ?1, ?2, 1, ?3)
             ON CONFLICT(language_key, file_name) DO UPDATE SET
                 content = excluded.content, is_custom = excluded.is_custom,
                 updated_at = excluded.updated_at",
            rusqlite::params![format!("skill.{}.md.tmpl", rule), text, utc_now_rfc3339()],
        )?;
        // `taken`, not `proposed`: the event is this install's new edit
        // origin — `proposed` is the OWNER's record that he saw an open
        // version on dev. The note carries the seq, which the bump just
        // assigned.
        let note = format!(
            "proposal to main {} repo {} seq {}",
            main_login, repo_id, seq
        );
        tx.execute(
            "INSERT INTO rule_events (rule, kind, repo, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![rule, rule_event::TAKEN, event_repo, note, utc_now_rfc3339()],
        )?;
        tx.execute(
            "UPDATE rule_proposals SET local_state = ?3
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![
                repo_id,
                file_name,
                crate::models::proposal_local_state::APPLIED
            ],
        )?;
        tx.commit()?;
        Ok(seq)
    }

    /// Pull the dev version into a local proposal row. Optimistic
    /// concurrency: the UPDATE only lands when the row the transport
    /// resolved from is still current — `seq` AND `updated_at` unchanged —
    /// so a local write in the window is never overwritten by the pull it
    /// just invalidated. Returns whether the pull landed.
    #[allow(clippy::too_many_arguments)]
    pub fn pull_rule_proposal(
        &self,
        repo_id: i64,
        file_name: &str,
        expected_seq: i64,
        expected_updated_at: &str,
        seq: i64,
        status: &str,
        text: &str,
        at: &str,
    ) -> SqlResult<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE rule_proposals SET seq=?5, status=?6, text=?7, at=?8,
                 resolved_action=NULL, resolved_by=NULL, resolved_at=NULL,
                 resolved_rev=NULL, updated_at=?9
             WHERE repo_id=?1 AND file_name=?2 AND seq=?3 AND updated_at=?4",
            rusqlite::params![
                repo_id,
                file_name,
                expected_seq,
                expected_updated_at,
                seq,
                status,
                text,
                at,
                utc_now_rfc3339()
            ],
        )?;
        Ok(n == 1)
    }

    /// Flip the wire status (open → withdrawn) in place; the seq and text
    /// stay — a withdrawn version still resolves against its receipts.
    pub fn set_rule_proposal_status(
        &self,
        repo_id: i64,
        file_name: &str,
        status: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_proposals SET status = ?3, updated_at = ?4
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name, status, utc_now_rfc3339()],
        )?;
        Ok(())
    }

    /// A crash between the proposal write and the canon write left the row
    /// at `pending_apply`: mark it applied once the text is in. The text is
    /// written by the caller first; this is the bookkeeping flip only.
    pub fn mark_rule_proposal_applied(&self, repo_id: i64, file_name: &str) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_proposals SET local_state = ?4, updated_at = ?3
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![
                repo_id,
                file_name,
                utc_now_rfc3339(),
                crate::models::proposal_local_state::APPLIED
            ],
        )?;
        Ok(())
    }

    // ---- rule_acceptances --------------------------------------------------

    /// Insert the `prepared` row of a decision — the first durable step of
    /// the decision machine. All fields arrive ready: the receipt bytes are
    /// immutable from this point, `seq` already assigned inside them.
    pub fn insert_rule_acceptance(&self, a: &RuleAcceptance) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO rule_acceptances (repo_id, rule, proposer_login,
                 decided_rev, action, receipt_json, proposal_text, status,
                 pub_state, base_canon_hash, canon_hash, auto_decided,
                 created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            rusqlite::params![
                a.repo_id,
                a.rule,
                a.proposer_login,
                a.decided_rev,
                a.action,
                a.receipt_json,
                a.proposal_text,
                a.status,
                a.pub_state,
                a.base_canon_hash,
                a.canon_hash,
                a.auto_decided,
                utc_now_rfc3339(),
                utc_now_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// T-000264: mark the row machine-decided. Runs right after the
    /// auto-decider's `decide` succeeds — same thread, same sweep, so no
    /// race with the owner is possible here.
    pub fn mark_rule_acceptance_auto_decided(
        &self,
        repo_id: i64,
        rule: &str,
        proposer_login: &str,
        decided_rev: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_acceptances SET auto_decided = 1
             WHERE repo_id=?1 AND rule=?2 AND proposer_login=?3 AND decided_rev=?4",
            rusqlite::params![repo_id, rule, proposer_login, decided_rev],
        )?;
        Ok(())
    }

    /// T-000264: receipts whose publication is blocked — the attention
    /// state of the proposals chip reads this.
    pub fn count_blocked_rule_receipts(&self) -> SqlResult<u32> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM rule_acceptances WHERE pub_state='blocked'",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    /// T-000264: machine decisions of the last 72 hours — the persisted
    /// "auto-decided: N" counter, derived from the rows themselves rather
    /// than a parallel notification store. `created_at`, not `updated_at`:
    /// the row is born at the decision moment, while updates keep landing
    /// as the receipt publishes — the window must measure deciding, not
    /// touching.
    pub fn count_recent_auto_decided(&self) -> SqlResult<u32> {
        // The cutoff is computed in Rust: rows are RFC3339 text, and a
        // SQLite `datetime('now', ...)` string (space-separated, no zone)
        // would compare against them by byte order — wrong in both
        // directions. Same-format RFC3339 strings order lexicographically.
        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(72)).to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM rule_acceptances
             WHERE auto_decided=1 AND created_at >= ?1",
            rusqlite::params![cutoff],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    /// One acceptance by its primary key.
    pub fn get_rule_acceptance(
        &self,
        repo_id: i64,
        rule: &str,
        proposer_login: &str,
        decided_rev: &str,
    ) -> SqlResult<Option<RuleAcceptance>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_acceptances
             WHERE repo_id = ?1 AND rule = ?2 AND proposer_login = ?3 AND decided_rev = ?4",
            ACCEPTANCE_COLS
        ))?;
        stmt.query_row(
            rusqlite::params![repo_id, rule, proposer_login, decided_rev],
            acceptance_from_row,
        )
        .optional()
    }

    /// Move the local work pipeline forward (or to a stable side state).
    /// `pub_state` is not touched — it lives its own life.
    pub fn set_rule_acceptance_status(
        &self,
        repo_id: i64,
        rule: &str,
        proposer_login: &str,
        decided_rev: &str,
        status: &str,
        canon_hash: Option<&str>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_acceptances SET status = ?5, canon_hash = COALESCE(?6, canon_hash),
                 updated_at = ?7
             WHERE repo_id = ?1 AND rule = ?2 AND proposer_login = ?3 AND decided_rev = ?4",
            rusqlite::params![
                repo_id,
                rule,
                proposer_login,
                decided_rev,
                status,
                canon_hash,
                utc_now_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// A permanent push refusal of the dev branch: every receipt of this
    /// repo still waiting to reach origin is marked `blocked` — visible in
    /// the UI, and still in the retry queue (`pending` + `blocked` are
    /// both publishable). Returns how many rows moved.
    pub fn block_pending_rule_acceptances(&self, repo_id: i64) -> SqlResult<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_acceptances SET pub_state = ?3, updated_at = ?2
             WHERE repo_id = ?1 AND pub_state = ?4",
            rusqlite::params![
                repo_id,
                utc_now_rfc3339(),
                crate::models::acceptance_pub_state::BLOCKED,
                crate::models::acceptance_pub_state::PENDING
            ],
        )
    }

    /// Move the receipt's publication state. Never touches `status`.
    pub fn set_rule_acceptance_pub_state(
        &self,
        repo_id: i64,
        rule: &str,
        proposer_login: &str,
        decided_rev: &str,
        pub_state: &str,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_acceptances SET pub_state = ?5, updated_at = ?6
             WHERE repo_id = ?1 AND rule = ?2 AND proposer_login = ?3 AND decided_rev = ?4",
            rusqlite::params![
                repo_id,
                rule,
                proposer_login,
                decided_rev,
                pub_state,
                utc_now_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// One repo's acceptances, newest first.
    pub fn list_rule_acceptances(&self, repo_id: i64) -> SqlResult<Vec<RuleAcceptance>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_acceptances
             WHERE repo_id = ?1 ORDER BY created_at DESC, rowid DESC",
            ACCEPTANCE_COLS
        ))?;
        let rows = stmt.query_map([repo_id], acceptance_from_row)?;
        rows.collect()
    }

    /// Local-work sweep queue: rows with an unfinished pipeline — the sweep
    /// resumes each at its first incomplete step. `conflicted`, `expired`
    /// and `superseded` are stable and stay out (a conflicted row is resumed
    /// only by its explicit UI actions).
    pub fn list_unfinished_rule_acceptances(&self) -> SqlResult<Vec<RuleAcceptance>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_acceptances
             WHERE status IN ('prepared','decided','applied','rendered')
             ORDER BY repo_id, rowid",
            ACCEPTANCE_COLS
        ))?;
        let rows = stmt.query_map([], acceptance_from_row)?;
        rows.collect()
    }

    /// Publication sweep queue: receipts that still owe the remote. Any
    /// pipeline state counts EXCEPT `prepared` (its receipt is not validated
    /// yet — Step 2 must run first) and `expired` (cancelled); `conflicted`
    /// and `superseded` included — their receipt still must reach dev.
    pub fn list_publishable_rule_acceptances(&self) -> SqlResult<Vec<RuleAcceptance>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_acceptances
             WHERE pub_state IN ('pending','blocked')
               AND status IN ('decided','applied','rendered','conflicted','superseded')
             ORDER BY repo_id, rowid",
            ACCEPTANCE_COLS
        ))?;
        let rows = stmt.query_map([], acceptance_from_row)?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{decision_action, proposal_status};

    /// A fresh database with the two repositories the tests' rows belong
    /// to (the tables reference `repositories(id)`).
    fn db() -> AppDb {
        let db = AppDb::new(PathBuf::from(":memory:")).unwrap();
        db.upsert_repository("Owner/api", None, None, None, None, None)
            .unwrap();
        db.upsert_repository("Owner/web", None, None, None, None, None)
            .unwrap();
        db
    }

    fn acceptance(
        repo_id: i64,
        rule: &str,
        login: &str,
        rev: &str,
        action: &str,
        status: &str,
        pub_state: &str,
    ) -> RuleAcceptance {
        RuleAcceptance {
            repo_id,
            rule: rule.into(),
            proposer_login: login.into(),
            decided_rev: rev.into(),
            action: action.into(),
            receipt_json: "{}".into(),
            proposal_text: "TEXT".into(),
            status: status.into(),
            pub_state: pub_state.into(),
            base_canon_hash: "h".into(),
            canon_hash: None,
            auto_decided: false,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn scan_missing_root_is_an_error_and_auto_decides_nothing() {
        let db = db();
        let tmp = tempfile::TempDir::new().unwrap();
        let missing = tmp.path().join("missing-checkout");
        db.set_repo_local_path(1, Some(missing.to_str().unwrap()))
            .unwrap();
        db.set_shared_role(1, Some("main")).unwrap();

        let scan = crate::commands::rule_proposals::scan_open_candidates(&db);
        assert!(
            scan.scan_errors
                .iter()
                .any(|e| e.contains("no local checkout")),
            "missing root must be a scan error, got {:?}",
            scan.scan_errors
        );
        assert!(scan.candidates.is_empty());

        let outcome = crate::commands::rule_proposals::auto_decide_rule_proposals(&db);
        assert_eq!(outcome.accepted, 0);
        assert_eq!(outcome.held, 0);
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e.contains("no local checkout")),
            "the scan error must reach the auto-decider, got {:?}",
            outcome.errors
        );
    }

    #[test]
    fn the_publishable_queue_retries_every_decided_state_but_never_prepared_or_expired() {
        let db = db();
        let rev = |n: usize| n.to_string().repeat(40);
        // Publishable: pub pending/blocked x decided..superseded.
        for (i, status) in ["decided", "applied", "rendered", "conflicted", "superseded"]
            .iter()
            .enumerate()
        {
            db.insert_rule_acceptance(&acceptance(
                1,
                "sdh-a",
                "p",
                &rev(i + 1),
                decision_action::ACCEPT,
                status,
                "pending",
            ))
            .unwrap();
        }
        db.insert_rule_acceptance(&acceptance(
            1,
            "sdh-a",
            "p",
            &rev(9),
            decision_action::ACCEPT,
            "decided",
            "blocked",
        ))
        .unwrap();
        // Excluded: prepared (Step 2 must run first), expired (cancelled),
        // and any settled pub state.
        for (i, status) in ["prepared", "expired"].iter().enumerate() {
            db.insert_rule_acceptance(&acceptance(
                1,
                "sdh-a",
                "p",
                &rev(20 + i),
                decision_action::ACCEPT,
                status,
                "pending",
            ))
            .unwrap();
        }
        for (i, ps) in ["published", "overtaken"].iter().enumerate() {
            db.insert_rule_acceptance(&acceptance(
                1,
                "sdh-a",
                "p",
                &rev(30 + i),
                decision_action::ACCEPT,
                "decided",
                ps,
            ))
            .unwrap();
        }
        let queue = db.list_publishable_rule_acceptances().unwrap();
        assert_eq!(
            queue.len(),
            6,
            "{:?}",
            queue
                .iter()
                .map(|a| (a.status.clone(), a.pub_state.clone()))
                .collect::<Vec<_>>()
        );
        assert!(queue.iter().all(|a| matches!(
            a.status.as_str(),
            "decided" | "applied" | "rendered" | "conflicted" | "superseded"
        )));
    }

    #[test]
    fn the_local_work_queue_resumes_the_whole_pipeline() {
        let db = db();
        let rev = |n: usize| n.to_string().repeat(40);
        for (i, status) in ["prepared", "decided", "applied", "rendered"]
            .iter()
            .enumerate()
        {
            db.insert_rule_acceptance(&acceptance(
                1,
                "sdh-a",
                "p",
                &rev(i + 1),
                decision_action::ACCEPT,
                status,
                "pending",
            ))
            .unwrap();
        }
        for (i, status) in ["conflicted", "expired", "superseded"].iter().enumerate() {
            db.insert_rule_acceptance(&acceptance(
                1,
                "sdh-a",
                "p",
                &rev(10 + i),
                decision_action::ACCEPT,
                status,
                "pending",
            ))
            .unwrap();
        }
        let queue = db.list_unfinished_rule_acceptances().unwrap();
        assert_eq!(queue.len(), 4);
        assert!(queue.iter().all(|a| matches!(
            a.status.as_str(),
            "prepared" | "decided" | "applied" | "rendered"
        )));
    }

    #[test]
    fn a_permanent_refusal_blocks_only_the_still_pending_receipts() {
        let db = db();
        let rev = |n: usize| n.to_string().repeat(40);
        for (i, ps) in ["pending", "blocked", "published", "overtaken"]
            .iter()
            .enumerate()
        {
            db.insert_rule_acceptance(&acceptance(
                1,
                "sdh-a",
                "p",
                &rev(i + 1),
                decision_action::ACCEPT,
                "decided",
                ps,
            ))
            .unwrap();
        }
        // Another repo's pending row is untouched.
        db.insert_rule_acceptance(&acceptance(
            2,
            "sdh-a",
            "p",
            &rev(9),
            decision_action::ACCEPT,
            "decided",
            "pending",
        ))
        .unwrap();
        assert_eq!(db.block_pending_rule_acceptances(1).unwrap(), 1);
        let rows = db.list_rule_acceptances(1).unwrap();
        let by_rev = |r: &str| rows.iter().find(|a| a.decided_rev == r).unwrap();
        assert_eq!(by_rev(&rev(1)).pub_state, "blocked");
        assert_eq!(by_rev(&rev(2)).pub_state, "blocked");
        assert_eq!(by_rev(&rev(3)).pub_state, "published");
        assert_eq!(by_rev(&rev(4)).pub_state, "overtaken");
        let other = db.list_rule_acceptances(2).unwrap();
        assert_eq!(other[0].pub_state, "pending");
    }

    /// T-000264: seed the canon the proposals will be based on.
    fn seed_canon(db: &AppDb, rule: &str, content: &str) {
        db.upsert_template_file(
            "_global",
            &format!("skill.{}.md.tmpl", rule),
            content,
            false,
        )
        .unwrap();
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(bytes);
        format!("{:x}", h.finalize())
    }

    fn propose(db: &AppDb, text: &str) {
        db.create_rule_proposal_txn(
            1,
            "sdh-a.p.json",
            "p",
            "sdh-a",
            text,
            "2026-11-08T00:00:00Z",
            None,
            "owner",
        )
        .unwrap();
    }

    #[test]
    fn the_first_proposal_captures_the_canon_hash_as_its_base() {
        let db = db();
        seed_canon(&db, "sdh-a", "CANON-V1");
        propose(&db, "T1");
        let row = db.get_rule_proposal(1, "sdh-a.p.json").unwrap().unwrap();
        assert_eq!(
            row.base_canon_hash.as_deref(),
            Some(sha256_hex(b"CANON-V1").as_str()),
            "the base is the pre-overwrite canon"
        );
    }

    #[test]
    fn a_chain_re_proposal_inherits_the_base_it_does_not_rehash_itself() {
        // The canon after the first proposal IS the first proposal's text.
        // Hashing it again would produce a base the owner's canon never
        // matches — every chain link would read as a spurious canon-diverged
        // conflict. The chain keeps the original fleet base instead.
        let db = db();
        seed_canon(&db, "sdh-a", "CANON-V1");
        propose(&db, "T1");
        propose(&db, "T2");
        let row = db.get_rule_proposal(1, "sdh-a.p.json").unwrap().unwrap();
        assert_eq!(row.text, "T2");
        assert_eq!(
            row.base_canon_hash.as_deref(),
            Some(sha256_hex(b"CANON-V1").as_str()),
            "chain inherits — never the hash of its own predecessor"
        );
    }

    #[test]
    fn a_withdrawn_chain_re_captures_and_a_canonless_proposal_stays_manual() {
        let db = db();
        seed_canon(&db, "sdh-a", "CANON-V1");
        propose(&db, "T1");
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE rule_proposals SET status = 'withdrawn' WHERE repo_id = 1",
                [],
            )
            .unwrap();
        // After withdrawal the chain is over: the next proposal re-captures
        // the canon as it stands — which is still T1 locally.
        propose(&db, "T3");
        let row = db.get_rule_proposal(1, "sdh-a.p.json").unwrap().unwrap();
        assert_eq!(
            row.base_canon_hash.as_deref(),
            Some(sha256_hex(b"T1").as_str()),
            "a new chain bases itself on the canon of its own moment"
        );
    }

    #[test]
    fn a_proposal_without_any_canon_carries_no_base() {
        // No template seeded at all: nothing provable, NULL — the auto
        // decider must treat it like every legacy proposal: manual.
        let db = db();
        propose(&db, "T1");
        let row = db.get_rule_proposal(1, "sdh-a.p.json").unwrap().unwrap();
        assert_eq!(row.base_canon_hash, None);
    }

    #[test]
    fn disable_cleanup_drops_proposals_acceptances_and_seq_marks() {
        let db = db();
        db.create_rule_proposal_txn(
            1,
            "sdh-a.p.json",
            "p",
            "sdh-a",
            "TEXT",
            "2026-11-08T00:00:00Z",
            None,
            "owner",
        )
        .unwrap();
        db.insert_rule_acceptance(&acceptance(
            1,
            "sdh-a",
            "p",
            &"1".repeat(40),
            decision_action::ACCEPT,
            "decided",
            "pending",
        ))
        .unwrap();
        db.delete_rule_proposal_state(1).unwrap();
        assert!(db.list_rule_proposals(1).unwrap().is_empty());
        assert!(db.list_rule_acceptances(1).unwrap().is_empty());
        assert_eq!(db.rule_proposal_max_seq(1, "sdh-a.p.json").unwrap(), 0);
    }

    #[test]
    fn a_pull_never_overwrites_a_concurrent_local_edit() {
        let db = db();
        db.create_rule_proposal_txn(
            1,
            "sdh-a.p.json",
            "p",
            "sdh-a",
            "V1",
            "2026-11-08T00:00:00Z",
            None,
            "owner",
        )
        .unwrap();
        let row = db.get_rule_proposal(1, "sdh-a.p.json").unwrap().unwrap();
        // A stale pull (old seq/updated_at) does not land.
        assert!(!db
            .pull_rule_proposal(
                1,
                "sdh-a.p.json",
                0,
                "stale",
                2,
                "open",
                "V2",
                "2026-11-09T00:00:00Z"
            )
            .unwrap());
        // A pull naming the row it read lands and clears the overlay.
        assert!(db
            .pull_rule_proposal(
                1,
                "sdh-a.p.json",
                row.seq,
                &row.updated_at,
                2,
                proposal_status::WITHDRAWN,
                "V2",
                "2026-11-09T00:00:00Z"
            )
            .unwrap());
        let after = db.get_rule_proposal(1, "sdh-a.p.json").unwrap().unwrap();
        assert_eq!(after.seq, 2);
        assert_eq!(after.status, proposal_status::WITHDRAWN);
        assert_eq!(after.text, "V2");
    }
}
