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
    })
}

const PROPOSAL_COLS: &str = "repo_id, file_name, login, rule, seq, status, local_state,
     text, at, resolved_action, resolved_by, resolved_at, resolved_rev,
     remote_ver, updated_at";

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
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

const ACCEPTANCE_COLS: &str = "repo_id, rule, proposer_login, decided_rev, action,
     receipt_json, proposal_text, status, pub_state, base_canon_hash,
     canon_hash, created_at, updated_at";

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

    /// This install's proposals for one repo and rule, file name order.
    pub fn list_rule_proposals_for_rule(
        &self,
        repo_id: i64,
        rule: &str,
    ) -> SqlResult<Vec<RuleProposal>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM rule_proposals
             WHERE repo_id = ?1 AND rule = ?2 ORDER BY file_name",
            PROPOSAL_COLS
        ))?;
        let rows = stmt.query_map(rusqlite::params![repo_id, rule], proposal_from_row)?;
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
                 resolved_rev, remote_ver, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
             ON CONFLICT(repo_id, file_name) DO UPDATE SET
                 login=excluded.login, rule=excluded.rule, seq=excluded.seq,
                 status=excluded.status, local_state=excluded.local_state,
                 text=excluded.text, at=excluded.at,
                 resolved_action=excluded.resolved_action,
                 resolved_by=excluded.resolved_by,
                 resolved_at=excluded.resolved_at,
                 resolved_rev=excluded.resolved_rev,
                 remote_ver=excluded.remote_ver,
                 updated_at=excluded.updated_at",
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
                utc_now_rfc3339()
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
        tx.execute(
            "INSERT INTO rule_proposals (repo_id, file_name, login, rule, seq,
                 status, local_state, text, at, updated_at)
             VALUES (?1,?2,?3,?4,?5,'open','pending_apply',?6,?7,?8)
             ON CONFLICT(repo_id, file_name) DO UPDATE SET
                 seq=excluded.seq, status=excluded.status,
                 local_state=excluded.local_state, text=excluded.text,
                 at=excluded.at, resolved_action=NULL, resolved_by=NULL,
                 resolved_at=NULL, resolved_rev=NULL, updated_at=excluded.updated_at",
            rusqlite::params![
                repo_id,
                file_name,
                login,
                rule,
                seq,
                text,
                at,
                utc_now_rfc3339()
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
            "UPDATE rule_proposals SET local_state = 'applied'
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name],
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
            "UPDATE rule_proposals SET local_state = 'applied', updated_at = ?3
             WHERE repo_id = ?1 AND file_name = ?2",
            rusqlite::params![repo_id, file_name, utc_now_rfc3339()],
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
                 pub_state, base_canon_hash, canon_hash, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
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
                utc_now_rfc3339(),
                utc_now_rfc3339()
            ],
        )?;
        Ok(())
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
             WHERE status IN ('prepared','decided','applied')
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
