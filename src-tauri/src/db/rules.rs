// T-000174 (v1.19.0): history and conflict state of the workflow rules — the
// `sdh-*` skills and the global CLAUDE.md section (migration v33).
//
// A rule is keyed by its NAME (skill name, or `claude.md.global`), never by a
// templates row: the same name identifies it in the bundle, in every repo copy
// and in `~/.claude/skills/`.

use super::*;
use std::collections::HashSet;

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

    /// Mark `rule` as in conflict. Returns `true` when the conflict is new and
    /// `false` when it was already open — the caller reports only a new one.
    pub fn open_rule_conflict(&self, rule: &str) -> SqlResult<bool> {
        let conn = self.conn.lock().unwrap();
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO rule_conflicts (rule, opened_at) VALUES (?1, ?2)",
            rusqlite::params![rule, utc_now_rfc3339()],
        )?;
        Ok(inserted == 1)
    }

    /// Close the conflict on `rule`. Returns `true` when one was open.
    pub fn close_rule_conflict(&self, rule: &str) -> SqlResult<bool> {
        let conn = self.conn.lock().unwrap();
        let removed = conn.execute("DELETE FROM rule_conflicts WHERE rule = ?1", [rule])?;
        Ok(removed == 1)
    }
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
        assert!(db.open_rule_conflict("sdh-retro").unwrap(), "new conflict");
        assert!(
            !db.open_rule_conflict("sdh-retro").unwrap(),
            "already open: not reported as new"
        );
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
