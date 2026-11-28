// T-000265 / UTC-chronology sync: the committed edit-clock ledger.
//
// `.sdh/edit-clocks/<file>.json` is a CRDT LWW-register keyed by the elements
// of one shared file (rows, fields, section headers, prose, structure) or by
// `file` for a file-level artifact. Every entry binds a stamp to ONE value:
// `h` is the sha256 of the value the stamp belongs to — a stamp is valid only
// together with its value. Tombstones (`del: true`) are never removed
// automatically: an offline clone returning months later must still lose to
// the deletion it never saw.
//
// The ledger merges per key by the total order over `(t, src, h, del)` —
// commutative, associative and idempotent by construction, so no positional
// "who merges" rule can ever leak into it. Far-future stamps are quarantined
// (reported as sync issues, key frozen) rather than silently corrected.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const CLOCKS_DIR: &str = ".sdh/edit-clocks";
pub const FORMAT_VERSION: u32 = 1;
/// Hard caps — exceeding them is a sync-blocking error, never a silent trim.
/// A todo.md of 2_000 rows x ~8 fields lands near 20k keys; the caps leave
/// an order of magnitude of headroom.
pub const MAX_ENTRIES: usize = 200_000;
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Stamps further than this into the future are quarantined, not trusted.
pub const MAX_FUTURE_SKEW_SECS: i64 = 24 * 3600;

/// sha256 hex of a value — the `h` field of every entry.
pub fn value_hash(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Encode a repo-relative shared path into a ledger file name:
/// `docs/todo.md` -> `docs--todo.md.json`. Slashes are the only separator
/// in repo paths; `--` cannot collide with real names since `/` never
/// appears inside a single path component.
pub fn ledger_file_name(repo_path: &str) -> String {
    format!("{}.json", repo_path.replace(['/', '\\'], "--"))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockEntry {
    /// sha256 of the value this stamp belongs to. Empty when unknown
    /// (legacy bootstrap edges); a mismatched hash invalidates the stamp.
    pub h: String,
    /// UTC seconds of the edit decision.
    pub t: i64,
    /// Commit (or authoring marker) the decision is provenanced by — the
    /// deterministic tie-breaker when two stamps share `t`.
    pub src: String,
    /// Tombstone: the element was deleted at `t`.
    #[serde(default)]
    pub del: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClockLedger {
    pub format: u32,
    pub file: String,
    pub entries: BTreeMap<String, ClockEntry>,
}

impl ClockLedger {
    pub fn new(file: &str) -> Self {
        ClockLedger {
            format: FORMAT_VERSION,
            file: file.to_string(),
            entries: BTreeMap::new(),
        }
    }

    /// Canonical serialization: sorted keys (BTreeMap), compact separators —
    /// byte-stable across machines so identical content never dirties git.
    pub fn serialize(&self) -> Result<String, String> {
        let out = serde_json::to_string(self).map_err(|e| format!("serialize clocks: {}", e))?;
        if out.len() > MAX_BYTES {
            return Err(format!(
                "edit-clock ledger for {} exceeds {} bytes — refusing to silently drop entries",
                self.file, MAX_BYTES
            ));
        }
        Ok(out)
    }

    pub fn parse(bytes: &[u8], file_hint: &str) -> Result<Self, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!(
                "edit-clock ledger for {} exceeds {} bytes",
                file_hint, MAX_BYTES
            ));
        }
        let text = std::str::from_utf8(bytes).map_err(|e| format!("clocks utf8: {}", e))?;
        let ledger: ClockLedger = serde_json::from_str(text)
            .map_err(|e| format!("parse clocks for {}: {}", file_hint, e))?;
        if ledger.entries.len() > MAX_ENTRIES {
            return Err(format!(
                "edit-clock ledger for {} exceeds {} entries",
                file_hint, MAX_ENTRIES
            ));
        }
        Ok(ledger)
    }

    /// CRDT merge: per key, the winner is the maximum entry in the total
    /// order over `(t, src, h, del)` — deterministic on both machines.
    /// Tombstones participate exactly like live entries — a deletion at 12:00
    /// beats a write at 11:00 and loses to a re-add at 13:00.
    pub fn merge(&self, other: &ClockLedger) -> ClockLedger {
        let mut entries = self.entries.clone();
        for (key, theirs) in &other.entries {
            let winner = match entries.get(key) {
                None => theirs.clone(),
                Some(ours) => pick_winner(ours, theirs).clone(),
            };
            entries.insert(key.clone(), winner);
        }
        ClockLedger {
            format: FORMAT_VERSION,
            file: self.file.clone(),
            entries,
        }
    }

    /// Record (or refresh) one key. Same LWW rule as `merge`: a stale stamp
    /// never overwrites a newer one. Returns whether the entry changed.
    pub fn stamp(&mut self, key: &str, entry: ClockEntry) -> bool {
        let winner = match self.entries.get(key) {
            None => entry,
            Some(existing) => pick_winner(existing, &entry).clone(),
        };
        let changed = self.entries.get(key) != Some(&winner);
        self.entries.insert(key.to_string(), winner);
        changed
    }

    /// Keys whose stamps lie beyond the trustable future — wrong-clock
    /// machines. The caller surfaces them as sync issues and freezes the key;
    /// nothing here silently rewrites the time.
    pub fn quarantined_keys(&self, now_utc: i64) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, e)| e.t > now_utc + MAX_FUTURE_SKEW_SECS)
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// The entry that decides `key`, if any — tombstones included.
    pub fn get(&self, key: &str) -> Option<&ClockEntry> {
        self.entries.get(key)
    }
}

impl ClockEntry {
    /// True when the stamp lies beyond the trustable future — a wrong-clock
    /// machine wrote it. A quarantined entry never decides resolution: the
    /// key freezes out of every winner/tombstone read (degraded fallback,
    /// reported as a sync issue), but the stamp itself is never rewritten —
    /// when the machine's clock is repaired its honest entries merge back in.
    pub fn is_quarantined(&self, now_utc: i64) -> bool {
        self.t > now_utc + MAX_FUTURE_SKEW_SECS
    }
}

/// Pick the maximum entry in the total order over `(t, src, h, del)`:
/// larger `t` wins, then larger `src`, then larger `h`; if all three tie,
/// a tombstone (`del: true`) wins over a live entry. The final direction is
/// arbitrary but fixed — it makes same-second/same-source union results
/// commutative even when the two stamps bind different values. Identical
/// entries compare equal, so returning the left side is correct there.
fn pick_winner<'a>(a: &'a ClockEntry, b: &'a ClockEntry) -> &'a ClockEntry {
    let a_key = (a.t, a.src.as_str(), a.h.as_str(), a.del);
    let b_key = (b.t, b.src.as_str(), b.h.as_str(), b.del);
    if b_key > a_key {
        b
    } else {
        a
    }
}

// ---------------------------------------------------------------------------
// T-000266: the activation protocol file.
// ---------------------------------------------------------------------------

/// `.sdh/protocol.json` — written once by the owner's activation click,
/// travels the ordinary owner-only structure transport. From the moment it
/// appears in the fetched refs the clocked semantics are unconditional and
/// the positional fallback is abolished; an outdated snapshot of observed
/// versions can never roll activation back, because the flag rides the refs,
/// not the observation (spec D1a).
pub const PROTOCOL_PATH: &str = ".sdh/protocol.json";
pub const CHRONOLOGY_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolFile {
    pub chronology: u32,
    pub activated_at: String,
    pub baseline_commit: String,
}

/// Parse the protocol file — `None` when absent, malformed, or not yet at
/// the chronology version this binary understands. A corrupt protocol file
/// post-activation is a sync issue for the caller to raise, not a silent
/// legacy fallback: the caller distinguishes absent (None, pre-activation)
/// from corrupt (Err) exactly for that reason.
pub fn parse_protocol(bytes: &[u8]) -> Result<Option<ProtocolFile>, String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) if !t.trim().is_empty() => t,
        _ => return Ok(None),
    };
    let pf: ProtocolFile =
        serde_json::from_str(text).map_err(|e| format!("parse protocol.json: {}", e))?;
    if pf.chronology < CHRONOLOGY_VERSION {
        return Ok(None);
    }
    Ok(Some(pf))
}

pub fn render_protocol(activated_at: &str, baseline_commit: &str) -> String {
    serde_json::to_string_pretty(&ProtocolFile {
        chronology: CHRONOLOGY_VERSION,
        activated_at: activated_at.to_string(),
        baseline_commit: baseline_commit.to_string(),
    })
    .unwrap_or_else(|_| "{}".to_string())
}

// Key constructors — the single place that knows the key grammar.

pub fn row_key(id: &str) -> String {
    format!("r:{}", id)
}

/// T-000268: the edit-clock ledger of one shared file, read from the repo
/// worktree's `.sdh/edit-clocks/` — `None` when absent or unreadable. The
/// absence keeps every consumer in its legacy rule (pre-activation repos);
/// a corrupt ledger reads as absent too — the sync layer already reports
/// ledger corruption as its own sync issue (spec D1a), so double-reporting
/// from a consumer would only spam.
pub(crate) fn load_worktree_ledger(repo: &std::path::Path, rel: &str) -> Option<ClockLedger> {
    let path = repo.join(CLOCKS_DIR).join(ledger_file_name(rel));
    let bytes = std::fs::read(path).ok()?;
    ClockLedger::parse(&bytes, rel).ok()
}

pub fn field_key(id: &str, field_index: usize) -> String {
    format!("r:{}.f{}", id, field_index)
}

pub fn header_key(section_key: &str) -> String {
    format!("h:{}", section_key)
}

pub fn prose_key(content_hash: &str) -> String {
    format!("p:{}", content_hash)
}

pub const STRUCTURE_KEY: &str = "__structure__";

/// The single key of a whole-file artifact's ledger (T-000273): the file is
/// the atom — one entry binds the content hash to its stamp, `del` marks the
/// tombstone. File-level existence/content resolution reads nothing else.
pub const FILE_KEY: &str = "file";

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(h: &str, t: i64, src: &str, del: bool) -> ClockEntry {
        ClockEntry {
            h: h.to_string(),
            t,
            src: src.to_string(),
            del,
        }
    }

    fn ledger_with(pairs: &[(&str, ClockEntry)]) -> ClockLedger {
        let mut l = ClockLedger::new("docs/todo.md");
        for (k, e) in pairs {
            l.entries.insert(k.to_string(), e.clone());
        }
        l
    }

    #[test]
    fn file_name_encoding() {
        assert_eq!(ledger_file_name("docs/todo.md"), "docs--todo.md.json");
        assert_eq!(
            ledger_file_name("docs/backend-requirements/REQ-001_x.md"),
            "docs--backend-requirements--REQ-001_x.md.json"
        );
    }

    #[test]
    fn serialize_is_canonical_and_roundtrips() {
        let l = ledger_with(&[
            ("r:T-000002", entry("h2", 200, "srcb", false)),
            ("r:T-000001", entry("h1", 100, "srca", false)),
        ]);
        let s1 = l.serialize().unwrap();
        // BTreeMap ordering: T-000001 before T-000002 regardless of insertion.
        assert!(s1.find("T-000001").unwrap() < s1.find("T-000002").unwrap());
        let back = ClockLedger::parse(s1.as_bytes(), "docs/todo.md").unwrap();
        assert_eq!(back.entries, l.entries);
        assert_eq!(back.serialize().unwrap(), s1);
    }

    #[test]
    fn merge_picks_higher_t() {
        let a = ledger_with(&[("r:X", entry("ha", 100, "sa", false))]);
        let b = ledger_with(&[("r:X", entry("hb", 200, "sb", false))]);
        let m = a.merge(&b);
        assert_eq!(m.entries["r:X"].h, "hb");
        assert_eq!(m.entries["r:X"].t, 200);
    }

    #[test]
    fn merge_is_commutative_associative_idempotent() {
        let a = ledger_with(&[
            ("r:X", entry("ha", 100, "sa", false)),
            ("r:Y", entry("hy1", 150, "sa", true)),
        ]);
        let b = ledger_with(&[
            ("r:X", entry("hb", 200, "sb", false)),
            ("r:Z", entry("hz", 50, "sb", false)),
        ]);
        let c = ledger_with(&[("r:X", entry("hc", 200, "sc", false))]);
        // commutative
        assert_eq!(a.merge(&b).entries, b.merge(&a).entries);
        // associative
        assert_eq!(a.merge(&b).merge(&c).entries, a.merge(&b.merge(&c)).entries);
        // idempotent
        assert_eq!(a.merge(&a).entries, a.entries);
        // tie broken deterministically by src
        let m = b.merge(&c);
        assert_eq!(m.entries["r:X"].src, "sc");
    }

    #[test]
    fn merge_full_tie_is_commutative_and_uses_hash_then_tombstone() {
        // Same `(t, src)`, different value hashes: the larger hash wins in
        // either merge order, byte-for-byte.
        let low_hash = ledger_with(&[("r:X", entry("aaa", 200, "same-src", false))]);
        let high_hash = ledger_with(&[("r:X", entry("bbb", 200, "same-src", false))]);
        let ab = low_hash.merge(&high_hash).serialize().unwrap();
        let ba = high_hash.merge(&low_hash).serialize().unwrap();
        assert_eq!(ab, ba);
        assert_eq!(low_hash.merge(&high_hash).entries["r:X"].h, "bbb");

        // Same `(t, src, h)`, different tombstone flags: `del: true` is the
        // fixed final tie-breaker in either merge order.
        let live = ledger_with(&[("r:X", entry("same-hash", 200, "same-src", false))]);
        let deleted = ledger_with(&[("r:X", entry("same-hash", 200, "same-src", true))]);
        let live_deleted = live.merge(&deleted).serialize().unwrap();
        let deleted_live = deleted.merge(&live).serialize().unwrap();
        assert_eq!(live_deleted, deleted_live);
        assert!(live.merge(&deleted).entries["r:X"].del);
    }

    #[test]
    fn tombstone_beats_older_write_loses_to_newer_readd() {
        let deletion = ledger_with(&[("r:X", entry("ha", 300, "sd", true))]);
        let stale_write = ledger_with(&[("r:X", entry("ha", 100, "sw", false))]);
        let readd = ledger_with(&[("r:X", entry("ha", 400, "sr", false))]);
        // deletion at 300 beats write at 100
        let m = deletion.merge(&stale_write);
        assert!(m.entries["r:X"].del);
        // re-add at 400 beats deletion at 300
        let m2 = deletion.merge(&readd);
        assert!(!m2.entries["r:X"].del);
        assert_eq!(m2.entries["r:X"].src, "sr");
    }

    #[test]
    fn stamp_never_overwrites_newer() {
        let mut l = ClockLedger::new("docs/todo.md");
        assert!(l.stamp("r:X", entry("h1", 200, "s1", false)));
        assert!(!l.stamp("r:X", entry("h0", 100, "s0", false)));
        assert_eq!(l.entries["r:X"].h, "h1");
        assert!(l.stamp("r:X", entry("h2", 300, "s2", false)));
        assert_eq!(l.entries["r:X"].h, "h2");
    }

    #[test]
    fn quarantine_catches_far_future_only() {
        let now = 1_000_000;
        let l = ledger_with(&[
            ("r:ok", entry("h", now + 3600, "s", false)),
            (
                "r:bad",
                entry("h", now + MAX_FUTURE_SKEW_SECS + 1, "s", false),
            ),
        ]);
        assert_eq!(l.quarantined_keys(now), vec!["r:bad".to_string()]);
    }

    #[test]
    fn value_hash_is_stable() {
        assert_eq!(value_hash(b"abc"), value_hash(b"abc"));
        assert_ne!(value_hash(b"abc"), value_hash(b"abd"));
        assert_eq!(value_hash(b"abc").len(), 64);
    }

    #[test]
    fn key_grammar() {
        assert_eq!(row_key("T-000123"), "r:T-000123");
        assert_eq!(field_key("T-000123", 3), "r:T-000123.f3");
        assert_eq!(header_key("v1.25.0"), "h:v1.25.0");
        assert_eq!(prose_key("abc"), "p:abc");
    }

    #[test]
    fn protocol_roundtrip_and_gate() {
        let text = render_protocol("2026-11-27T10:00:00Z", "deadbeef");
        let pf = parse_protocol(text.as_bytes()).unwrap().unwrap();
        assert_eq!(pf.chronology, CHRONOLOGY_VERSION);
        assert_eq!(pf.baseline_commit, "deadbeef");
        // empty and absent parse as no-protocol
        assert!(parse_protocol(b"").unwrap().is_none());
        assert!(parse_protocol(b"  ").unwrap().is_none());
        // corrupt is an Err, never a silent fallback
        assert!(parse_protocol(b"{not json").is_err());
        // a future/older chronology version below the gate is inactive
        assert!(parse_protocol(
            b"{\"chronology\":0,\"activated_at\":\"x\",\"baseline_commit\":\"y\"}"
        )
        .unwrap()
        .is_none());
    }
}
