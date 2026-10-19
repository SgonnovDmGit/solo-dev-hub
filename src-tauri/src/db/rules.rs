// T-000174 (v1.19.0): history and conflict state of the workflow rules — the
// `sdh-*` skills and the global CLAUDE.md section (migration v33).
//
// A rule is keyed by its NAME (skill name, or `claude.md.global`), never by a
// templates row: the same name identifies it in the bundle, in every repo copy
// and in `~/.claude/skills/`.

use super::*;
use std::collections::{BTreeSet, HashSet};

/// The closed vocabulary of `rule_events.kind`, mirrored by the CHECK in
/// migration v33. Writers use these, so a typo is a compile error rather than a
/// row the database refuses at runtime.
pub mod rule_event {
    /// An agent's hand-back was taken into the database (one event per repo).
    pub const TAKEN: &str = "taken";
    /// The owner saved the rule in the app.
    pub const SAVED: &str = "saved";
    /// The owner reset the rule to the bundled text (T-000212).
    pub const RESET: &str = "reset";
    /// A pending hand-back was set aside because the owner's own edit of the
    /// same rule won.
    pub const SUPERSEDED: &str = "superseded";
    /// A marker sat beside a copy already equal to the rule: the edit it stood
    /// for was overwritten before anyone collected it.
    pub const LOST: &str = "lost";
    /// Two repos handed back different texts of the rule.
    pub const CONFLICT: &str = "conflict";
    /// A conflict was closed.
    ///
    /// **Convention:** `repo` set = the text handed back from that repo was
    /// accepted, which makes the event an origin of the current text (see
    /// `latest_origin_event`); `repo` NULL = both sides were rejected, or the
    /// conflict went away outside the app — the text did not change. `note`
    /// carries the human wording either way.
    pub const CONFLICT_RESOLVED: &str = "conflict_resolved";
}

/// T-000206: the `note` of a `conflict_resolved` event the app writes itself
/// when a conflict goes away without the owner picking a side. Stored as plain
/// English; the frontend translates them by value, so the wording here is a
/// contract — change it only together with those translations.
pub mod conflict_note {
    /// The owner saved or reset the rule while its hand-backs were in conflict:
    /// the explicit edit set both markers aside.
    pub const SUPERSEDED_BY_OWNER: &str = "superseded by the owner's edit";
    /// The conflict vanished on its own — a marker was removed, or the texts
    /// became equal — before anyone decided it in the app.
    pub const RESOLVED_OUTSIDE: &str = "resolved outside the app";
    /// The owner picked one side in the app: that side's text is now the rule.
    pub const ACCEPTED: &str = "accepted by the owner";
    /// The owner rejected every side in the app: the rule's text stayed as it was.
    pub const REJECTED_BY_OWNER: &str = "rejected by the owner";
}

/// T-000206: `rule_conflicts.repo_ids` — the sides of a conflict as a JSON
/// array of repository ids, sorted. Ids, not labels: a label follows the repo's
/// GitHub name and changes with a rename, an id does not.
fn conflict_repo_ids_json(repo_ids: &BTreeSet<i64>) -> String {
    serde_json::to_string(repo_ids).expect("a set of integers always serialises")
}

/// Parse `rule_conflicts.repo_ids` as written by `conflict_repo_ids_json`.
pub fn parse_conflict_repo_ids(raw: &str) -> Result<BTreeSet<i64>, String> {
    serde_json::from_str(raw).map_err(|e| format!("repo_ids {:?}: {}", raw, e))
}

fn rule_event_from_row(row: &Row) -> SqlResult<RuleEvent> {
    Ok(RuleEvent {
        id: row.get(0)?,
        rule: row.get(1)?,
        kind: row.get(2)?,
        repo: row.get(3)?,
        note: row.get(4)?,
        created_at: row.get(5)?,
    })
}

impl AppDb {
    /// Append one event to a rule's history, stamped now.
    pub fn insert_rule_event(
        &self,
        rule: &str,
        kind: &str,
        repo: Option<&str>,
        note: Option<&str>,
    ) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO rule_events (rule, kind, repo, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![rule, kind, repo, note, utc_now_rfc3339()],
        )?;
        Ok(())
    }

    /// A rule's whole history, newest first. `id` breaks ties between events
    /// written within one timestamp tick, so the order is the order of writing.
    pub fn list_rule_events(&self, rule: &str) -> SqlResult<Vec<RuleEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, rule, kind, repo, note, created_at FROM rule_events
             WHERE rule = ?1 ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([rule], rule_event_from_row)?;
        rows.collect()
    }

    /// Where the rule's current text came from: the newest event that wrote
    /// it — taken, saved, reset, or a conflict resolved by accepting one side
    /// (`repo` set, see `rule_event::CONFLICT_RESOLVED`). `None` when the text
    /// predates the history (v1.19.0) or was never changed.
    pub fn latest_origin_event(&self, rule: &str) -> SqlResult<Option<RuleEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, rule, kind, repo, note, created_at FROM rule_events
             WHERE rule = ?1
               AND (kind IN (?2, ?3, ?4) OR (kind = ?5 AND repo IS NOT NULL))
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(
            rusqlite::params![
                rule,
                rule_event::TAKEN,
                rule_event::SAVED,
                rule_event::RESET,
                rule_event::CONFLICT_RESOLVED
            ],
            rule_event_from_row,
        )?;
        rows.next().transpose()
    }

    /// Names of the rules whose hand-backs are currently in conflict.
    pub fn open_rule_conflicts(&self) -> SqlResult<HashSet<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT rule FROM rule_conflicts")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    }

    /// T-000206: every open conflict with its sides as stored — `(rule,
    /// repo_ids)`, the ids still as JSON text: a row that does not parse is the
    /// caller's to report, not a reason to fail the whole read.
    pub fn list_rule_conflicts(&self) -> SqlResult<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT rule, repo_ids FROM rule_conflicts ORDER BY rule")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    /// Mark `rule` as in conflict between the repos `repo_ids`. Returns `true`
    /// when the conflict is new and `false` when it was already open (the row
    /// is then left as it was) — the caller reports only a new one.
    pub fn open_rule_conflict(&self, rule: &str, repo_ids: &BTreeSet<i64>) -> SqlResult<bool> {
        let conn = self.conn.lock().unwrap();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO rule_conflicts (rule, opened_at, repo_ids) VALUES (?1, ?2, ?3)",
            rusqlite::params![rule, utc_now_rfc3339(), conflict_repo_ids_json(repo_ids)],
        )?;
        Ok(inserted == 1)
    }

    /// T-000206: replace the sides of the open conflict on `rule` (a repo
    /// joined it). A no-op when no conflict is open.
    pub fn set_rule_conflict_repos(&self, rule: &str, repo_ids: &BTreeSet<i64>) -> SqlResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE rule_conflicts SET repo_ids = ?2 WHERE rule = ?1",
            rusqlite::params![rule, conflict_repo_ids_json(repo_ids)],
        )?;
        Ok(())
    }

    /// Close the conflict on `rule`. Returns `true` when one was open.
    pub fn close_rule_conflict(&self, rule: &str) -> SqlResult<bool> {
        let conn = self.conn.lock().unwrap();
        let removed = conn.execute("DELETE FROM rule_conflicts WHERE rule = ?1", [rule])?;
        Ok(removed == 1)
    }
}

/// Test support: a rule's whole history, OLDEST first — the order the events
/// happened in, which is what a test asserts against.
#[cfg(test)]
pub(crate) fn rule_history(db: &AppDb, rule: &str) -> Vec<RuleEvent> {
    let mut events = db.list_rule_events(rule).unwrap();
    events.reverse();
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    #[test]
    fn test_rule_history_is_per_rule_and_newest_first() {
        let db = make_db();
        db.insert_rule_event("sdh-retro", rule_event::SAVED, None, None)
            .unwrap();
        db.insert_rule_event("sdh-debugging", rule_event::RESET, None, None)
            .unwrap();
        db.insert_rule_event(
            "sdh-retro",
            rule_event::TAKEN,
            Some("acme/one"),
            Some("approved in chat"),
        )
        .unwrap();

        let events = db.list_rule_events("sdh-retro").unwrap();

        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(kinds, vec!["taken", "saved"]);
        assert_eq!(events[0].repo.as_deref(), Some("acme/one"));
        assert_eq!(events[0].note.as_deref(), Some("approved in chat"));
    }

    /// The origin is the newest event that actually wrote the text — events
    /// that left it alone (lost, superseded, a conflict, a conflict closed
    /// without accepting a side) never count, however recent.
    #[test]
    fn test_latest_origin_event_picks_the_newest_text_writing_event() {
        let db = make_db();
        assert_eq!(db.latest_origin_event("sdh-retro").unwrap(), None);

        db.insert_rule_event("sdh-retro", rule_event::SAVED, None, None)
            .unwrap();
        db.insert_rule_event("sdh-retro", rule_event::TAKEN, Some("acme/one"), None)
            .unwrap();
        for kind in [
            rule_event::LOST,
            rule_event::SUPERSEDED,
            rule_event::CONFLICT,
        ] {
            db.insert_rule_event("sdh-retro", kind, Some("acme/two"), None)
                .unwrap();
        }
        db.insert_rule_event(
            "sdh-retro",
            rule_event::CONFLICT_RESOLVED,
            None,
            Some("both rejected"),
        )
        .unwrap();
        // Another rule's newer event must not leak in.
        db.insert_rule_event("sdh-debugging", rule_event::RESET, None, None)
            .unwrap();

        let origin = db.latest_origin_event("sdh-retro").unwrap().unwrap();
        assert_eq!(origin.kind, "taken");
        assert_eq!(origin.repo.as_deref(), Some("acme/one"));

        // A conflict resolved by accepting one side did write the text.
        db.insert_rule_event(
            "sdh-retro",
            rule_event::CONFLICT_RESOLVED,
            Some("acme/two"),
            Some("accepted the text from acme/two"),
        )
        .unwrap();
        let origin = db.latest_origin_event("sdh-retro").unwrap().unwrap();
        assert_eq!(origin.kind, "conflict_resolved");
        assert_eq!(origin.repo.as_deref(), Some("acme/two"));

        db.insert_rule_event("sdh-retro", rule_event::RESET, None, None)
            .unwrap();
        assert_eq!(
            db.latest_origin_event("sdh-retro").unwrap().unwrap().kind,
            "reset"
        );
    }

    #[test]
    fn test_a_rule_conflict_opens_once_and_closes_once() {
        let db = make_db();
        let sides = BTreeSet::from([3, 1]);
        assert!(
            db.open_rule_conflict("sdh-retro", &sides).unwrap(),
            "new conflict"
        );
        assert!(
            !db.open_rule_conflict("sdh-retro", &BTreeSet::from([9]))
                .unwrap(),
            "already open: not reported as new"
        );
        assert_eq!(
            db.list_rule_conflicts().unwrap(),
            vec![("sdh-retro".to_string(), "[1,3]".to_string())],
            "the sides of the first opening are kept"
        );
        db.set_rule_conflict_repos("sdh-retro", &BTreeSet::from([1, 3, 4]))
            .unwrap();
        let (_, raw) = db.list_rule_conflicts().unwrap().remove(0);
        assert_eq!(
            parse_conflict_repo_ids(&raw).unwrap(),
            BTreeSet::from([1, 3, 4])
        );
        assert!(parse_conflict_repo_ids("not json").is_err());
        assert_eq!(
            db.open_rule_conflicts().unwrap(),
            HashSet::from(["sdh-retro".to_string()])
        );
        assert!(db.close_rule_conflict("sdh-retro").unwrap());
        assert!(
            !db.close_rule_conflict("sdh-retro").unwrap(),
            "nothing open"
        );
        assert!(db.open_rule_conflicts().unwrap().is_empty());
    }
}
