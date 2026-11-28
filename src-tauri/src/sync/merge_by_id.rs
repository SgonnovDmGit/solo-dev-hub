// F-000044 / T-000218: three-way merge of the shared MD files BY ENTITY ID.
//
// In a "shared" repo `docs/todo.md`, `docs/done.md` and `docs/bug-reports.md`
// are committed and edited by two installs of the app. Git's textual merge
// would raise conflicts whenever two people touch neighbouring lines; this
// module merges per entity (task / done row / bug) and then per field, so two
// people editing different rows never conflict, and editing the same row
// resolves by fixed rules (never conflict markers).
//
// Pure: strings in, string out. Used by the git merge driver
// (`crate::merge_driver`) and, in-process, by the sync that commits into a
// non-checked-out branch (T-000220).
//
// Rules (see the T-000218 spec):
//   * added on one side only -> included;
//   * deleted on one side and unchanged on the other -> deleted;
//   * deleted on one side and changed on the other -> the changed one is kept,
//     except in todo.md when the change touched only status and/or section
//     (a deleted todo row normally means "moved to done.md") -> deleted;
//   * a field changed on one side only -> that change; on both sides
//     differently -> status: the further along; bug fix_attempts: the larger;
//     anything else (incl. the entity's section, the checkbox): ours.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedFileKind {
    Todo,
    Done,
    Bugs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// One of the three inputs is not valid UTF-8.
    NonUtf8,
    /// The path is not one of the shared files.
    UnknownKind(String),
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::NonUtf8 => write!(f, "input is not valid UTF-8"),
            MergeError::UnknownKind(p) => write!(f, "not a shared file: {}", p),
        }
    }
}

impl std::error::Error for MergeError {}

/// Which shared file a repo-relative path names. Case-insensitive, accepts `/`
/// and `\` separators; anything but the three exact paths is `None`.
pub fn kind_for_path(p: &str) -> Option<SharedFileKind> {
    let norm = p.trim().replace('\\', "/").to_lowercase();
    let norm = norm.trim_start_matches("./");
    match norm {
        "docs/todo.md" => Some(SharedFileKind::Todo),
        "docs/done.md" => Some(SharedFileKind::Done),
        "docs/bug-reports.md" => Some(SharedFileKind::Bugs),
        _ => None,
    }
}

/// Byte-level entry for callers that hold raw file content.
pub fn merge_shared_bytes(
    kind: SharedFileKind,
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
) -> Result<String, MergeError> {
    let b = std::str::from_utf8(base).map_err(|_| MergeError::NonUtf8)?;
    let o = std::str::from_utf8(ours).map_err(|_| MergeError::NonUtf8)?;
    let t = std::str::from_utf8(theirs).map_err(|_| MergeError::NonUtf8)?;
    merge_shared_file(kind, b, o, t)
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Ent {
    pub(crate) key: String,
    /// Line prefix after the indentation: `- `, `- [ ] `, `- [x] `.
    pub(crate) marker: String,
    /// Raw (escape-preserving, trimmed) pipe-separated fields.
    pub(crate) fields: Vec<String>,
    /// Key of the nearest preceding `## ` header, `""` before the first one.
    pub(crate) section: String,
    /// The original line text without the line ending.
    pub(crate) raw: String,
}

pub(crate) enum Line {
    Header {
        key: String,
        text: String,
    },
    Ent(usize),
    /// A line that looks like an entity but repeats an id seen earlier in the
    /// same file; kept verbatim where it is, never merged.
    Dup(String),
    Other(String),
}

pub(crate) struct Doc {
    pub(crate) lines: Vec<Line>,
    pub(crate) ents: Vec<Ent>,
    pub(crate) by_key: HashMap<String, usize>,
    /// header key -> full header line
    hdrs: HashMap<String, String>,
    crlf: bool,
    trailing_nl: bool,
    bom: bool,
    empty: bool,
}

fn section_key(rest: &str) -> String {
    let rest = rest.trim();
    let first = rest.split_whitespace().next().unwrap_or("");
    let mut ch = first.chars();
    let c0 = ch.next();
    let c1 = ch.next();
    let versionish = match (c0, c1) {
        (Some(a), _) if a.is_ascii_digit() => true,
        (Some('v'), Some(b)) if b.is_ascii_digit() => true,
        _ => false,
    };
    if versionish {
        first.to_string()
    } else {
        rest.to_string()
    }
}

/// Split by unescaped `|`, keeping `\|` verbatim and trimming each field.
fn split_raw(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' && it.peek() == Some(&'|') {
            cur.push('\\');
            cur.push('|');
            it.next();
        } else if c == '|' {
            parts.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    parts.push(cur.trim().to_string());
    parts
}

fn looks_like_task_id_start(r: &str) -> bool {
    r.chars().next().map_or(false, |c| {
        c == 'T' || c == 'F' || c == 'O' || c == 'M' || c == 'B' || c == 'v' || c.is_ascii_digit()
    })
}

fn detect(kind: SharedFileKind, trimmed: &str) -> Option<(String, Vec<String>)> {
    match kind {
        SharedFileKind::Todo => {
            let (marker, rest) = if let Some(r) = trimmed.strip_prefix("- [ ] ") {
                ("- [ ] ", r)
            } else if let Some(r) = trimmed.strip_prefix("- [x] ") {
                ("- [x] ", r)
            } else if let Some(r) = trimmed.strip_prefix("- [X] ") {
                ("- [X] ", r)
            } else if let Some(r) = trimmed
                .strip_prefix("- ")
                .filter(|r| looks_like_task_id_start(r))
            {
                ("- ", r)
            } else {
                return None;
            };
            let fields = split_raw(rest);
            if fields.len() < 3 || fields[0].is_empty() {
                return None;
            }
            Some((marker.to_string(), fields))
        }
        SharedFileKind::Done => {
            let (marker, rest) = if let Some(r) = trimmed.strip_prefix("- [x] ") {
                ("- [x] ", r)
            } else if let Some(r) = trimmed.strip_prefix("- [X] ") {
                ("- [X] ", r)
            } else if let Some(r) = trimmed.strip_prefix("- ") {
                ("- ", r)
            } else {
                return None;
            };
            let fields = split_raw(rest);
            if fields.len() != 3 && fields.len() != 4 {
                return None;
            }
            Some((marker.to_string(), fields))
        }
        SharedFileKind::Bugs => {
            let (marker, rest) = if let Some(r) = trimmed.strip_prefix("- [ ] ") {
                ("- [ ] ", r)
            } else if let Some(r) = trimmed
                .strip_prefix("- ")
                .filter(|r| r.starts_with("B-") || r.starts_with("VB-"))
            {
                ("- ", r)
            } else {
                return None;
            };
            let fields = split_raw(rest);
            if fields.len() != 8 || fields[0].is_empty() {
                return None;
            }
            Some((marker.to_string(), fields))
        }
    }
}

pub(crate) fn parse(kind: SharedFileKind, input: &str) -> Doc {
    let (bom, s) = match input.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, input),
    };
    let crlf = s.contains("\r\n");
    let trailing_nl = s.ends_with('\n');
    let raw_lines: Vec<String> = if s.is_empty() {
        Vec::new()
    } else {
        let mut v: Vec<&str> = s.split('\n').collect();
        if trailing_nl {
            v.pop();
        }
        v.into_iter()
            .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
            .collect()
    };

    let mut doc = Doc {
        lines: Vec::new(),
        ents: Vec::new(),
        by_key: HashMap::new(),
        hdrs: HashMap::new(),
        crlf,
        trailing_nl,
        bom,
        empty: s.is_empty(),
    };
    let mut section = String::new();
    for raw in raw_lines {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            let key = section_key(rest);
            section = key.clone();
            doc.hdrs.entry(key.clone()).or_insert_with(|| raw.clone());
            doc.lines.push(Line::Header { key, text: raw });
            continue;
        }
        if let Some((marker, fields)) = detect(kind, trimmed) {
            let key = if fields[0].is_empty() {
                format!("\u{0}text:{}", trimmed)
            } else {
                fields[0].clone()
            };
            if doc.by_key.contains_key(&key) {
                doc.lines.push(Line::Dup(raw));
                continue;
            }
            let idx = doc.ents.len();
            doc.by_key.insert(key.clone(), idx);
            doc.ents.push(Ent {
                key,
                marker,
                fields,
                section: section.clone(),
                raw,
            });
            doc.lines.push(Line::Ent(idx));
            continue;
        }
        doc.lines.push(Line::Other(raw));
    }
    doc
}

// ---------------------------------------------------------------------------
// Field rules
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// UTC-chronology resolution (T-000267 / spec 2026-11-27-shared-sync-utc-chronology)
// ---------------------------------------------------------------------------

use crate::sync::edit_clocks::{field_key, row_key, value_hash, ClockEntry, ClockLedger};
use std::cell::RefCell;

/// The clocked-resolution context: the union ledger of both sides plus the
/// diagnostics the merge collects for the caller to surface as sync issues.
/// Post-activation there is no positional fallback — when neither side holds
/// a valid stamp the merge still terminates (a total function must), but it
/// ties-breaks deterministically on content and SAYS SO in the diagnostics,
/// never silently by machine position.
pub struct Clocks<'a> {
    pub ledger: &'a ClockLedger,
    pub diags: RefCell<Vec<String>>,
    /// UTC seconds at the merge start — the reference for far-future
    /// quarantine (T-000267): stamps beyond now+skew never decide resolution.
    pub now: i64,
}

impl<'a> Clocks<'a> {
    pub fn new(ledger: &'a ClockLedger) -> Self {
        Clocks {
            ledger,
            diags: RefCell::new(Vec::new()),
            now: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        }
    }
    fn diag(&self, msg: String) {
        self.diags.borrow_mut().push(msg);
    }
}

/// The entry of `key` when it validly stamps `value` — the hash must match,
/// or the stamp belongs to a different value and conservatively counts as
/// absent (spec D1: separate convergence of content and ledger can never pair
/// a new stamp with an old value).
fn valid_stamp<'a>(
    ledger: &'a ClockLedger,
    key: &str,
    value: &str,
    now: i64,
) -> Option<&'a ClockEntry> {
    let e = ledger.get(key)?;
    if e.is_quarantined(now) {
        return None;
    }
    if !e.del && e.h == value_hash(value.as_bytes()) {
        Some(e)
    } else {
        None
    }
}

/// The clocked both-changed arbiter: the side whose value carries the later
/// valid stamp wins. The ledger holds at most one winner per key, so at most
/// one side can match in practice; a neither-match is degraded mode — content
/// order decides (deterministic on every machine, keeping commutativity) and
/// the key is reported.
fn pick_clocked(clocks: &Clocks, key: &str, o: &str, t: &str) -> String {
    let so = valid_stamp(clocks.ledger, key, o, clocks.now);
    let st = valid_stamp(clocks.ledger, key, t, clocks.now);
    match (so, st) {
        (Some(a), Some(b)) => {
            if b.t > a.t || (b.t == a.t && b.src > a.src) {
                t.to_string()
            } else {
                o.to_string()
            }
        }
        (Some(_), None) => o.to_string(),
        (None, Some(_)) => t.to_string(),
        (None, None) => {
            clocks.diag(format!(
                "no valid stamp on either side for {} — content order decided",
                key
            ));
            // Deterministic degraded tie-break: never positional. The larger
            // value wins identically on both machines.
            if t > o {
                t.to_string()
            } else {
                o.to_string()
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Other,
    TaskStatus,
    BugStatus,
    Attempts,
}

fn role_at(kind: SharedFileKind, n: usize, i: usize) -> Role {
    match kind {
        SharedFileKind::Todo => {
            let status = if n >= 7 {
                n - 2
            } else if n == 5 || n == 6 {
                4
            } else {
                n - 1
            };
            if i == status {
                Role::TaskStatus
            } else {
                Role::Other
            }
        }
        SharedFileKind::Done => Role::Other,
        SharedFileKind::Bugs => {
            if n == 8 && i == 5 {
                Role::BugStatus
            } else if n == 8 && i == 6 {
                Role::Attempts
            } else {
                Role::Other
            }
        }
    }
}

fn status_rank(role: Role, v: &str) -> i32 {
    let v = v.trim();
    let order: &[&str] = match role {
        Role::TaskStatus => &["open", "in-progress", "review"],
        Role::BugStatus => &["created", "in-progress", "testing", "rejected", "confirmed"],
        _ => return -1,
    };
    order.iter().position(|s| *s == v).map_or(-1, |p| p as i32)
}

/// Three-way pick of one field. `b` is `None` when the base has no such field.
/// `clocks`/`ckey` carry the UTC-resolution context; the base-match rules above
/// stay genuine three-way in both modes.
fn pick(
    role: Role,
    b: Option<&str>,
    o: &str,
    t: &str,
    clocks: Option<&Clocks>,
    ckey: Option<String>,
) -> String {
    if o == t {
        return o.to_string();
    }
    if let Some(b) = b {
        if b == o {
            return t.to_string();
        }
        if b == t {
            return o.to_string();
        }
    }
    match clocks {
        // Post-activation: monotonic rules (status rank, max attempts) are
        // abolished — the later UTC decision wins, whatever direction it moves.
        Some(c) => pick_clocked(c, &ckey.unwrap_or_default(), o, t),
        None => match role {
            Role::TaskStatus | Role::BugStatus => {
                if status_rank(role, t) > status_rank(role, o) {
                    t.to_string()
                } else {
                    o.to_string()
                }
            }
            Role::Attempts => match (o.trim().parse::<i64>(), t.trim().parse::<i64>()) {
                (Ok(a), Ok(b)) if b > a => t.to_string(),
                _ => o.to_string(),
            },
            Role::Other => o.to_string(),
        },
    }
}

struct Merged {
    section: String,
    line: String,
}

fn same(a: &Ent, b: &Ent) -> bool {
    a.marker == b.marker && a.fields == b.fields && a.section == b.section
}

impl PartialEq for Ent {
    /// History-state equality (see `same`): marker, fields and section — the
    /// raw text and the key do not distinguish states.
    fn eq(&self, other: &Self) -> bool {
        same(self, other)
    }
}

fn only_status_or_section_changed(kind: SharedFileKind, base: &Ent, cur: &Ent) -> bool {
    if base.marker != cur.marker || base.fields.len() != cur.fields.len() {
        return false;
    }
    let n = cur.fields.len();
    base.fields
        .iter()
        .zip(cur.fields.iter())
        .enumerate()
        .all(|(i, (x, y))| x == y || role_at(kind, n, i) == Role::TaskStatus)
}

fn build_line(indent_from: &Ent, marker: &str, fields: &[String]) -> String {
    let indent: String = indent_from
        .raw
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    format!("{}{}{}", indent, marker, fields.join(" | "))
}

fn merge_both(
    kind: SharedFileKind,
    b: Option<&Ent>,
    o: &Ent,
    t: &Ent,
    clocks: Option<&Clocks>,
) -> Merged {
    // Fields of different arity (legacy lines): no positional merge possible.
    // Clocked mode: the whole row is one unit keyed by `r:<id>` (spec D2 —
    // legacy rows fall back to row-level stamps). Positional mode unchanged:
    // the side that changed wins, ours on a tie.
    if o.fields.len() != t.fields.len() {
        if let Some(c) = clocks {
            let w = pick_clocked(c, &row_key(&o.key), &o.raw, &t.raw);
            let src_side = if w == t.raw { t } else { o };
            return Merged {
                section: src_side.section.clone(),
                line: w,
            };
        }
        let pick_theirs = b.map_or(false, |b| same(b, o));
        let w = if pick_theirs { t } else { o };
        return Merged {
            section: w.section.clone(),
            line: w.raw.clone(),
        };
    }
    let n = o.fields.len();
    let bf = b.filter(|b| b.fields.len() == n);
    // Marker and section carry no field index of their own — in clocked mode
    // they resolve by the row-level stamp (granularity caveat: the row stamp
    // is the row's latest edit of any kind, documented in the spec's D2).
    let rowk = clocks.map(|_| row_key(&o.key));
    let marker = pick(
        Role::Other,
        b.map(|b| b.marker.as_str()),
        &o.marker,
        &t.marker,
        clocks,
        rowk.clone(),
    );
    let section = pick(
        Role::Other,
        b.map(|b| b.section.as_str()),
        &o.section,
        &t.section,
        clocks,
        rowk,
    );
    let fields: Vec<String> = (0..n)
        .map(|i| {
            pick(
                role_at(kind, n, i),
                bf.map(|b| b.fields[i].as_str()),
                &o.fields[i],
                &t.fields[i],
                clocks,
                clocks.map(|_| field_key(&o.key, i)),
            )
        })
        .collect();
    // Keep a side's original text when nothing differs from it (no reformat).
    let line = if marker == o.marker && fields == o.fields {
        o.raw.clone()
    } else if marker == t.marker && fields == t.fields {
        t.raw.clone()
    } else {
        build_line(o, &marker, &fields)
    };
    Merged { section, line }
}

fn decide(
    kind: SharedFileKind,
    b: Option<&Ent>,
    o: Option<&Ent>,
    t: Option<&Ent>,
    clocks: Option<&Clocks>,
) -> Option<Merged> {
    match (o, t) {
        (Some(o), Some(t)) => Some(merge_both(kind, b, o, t, clocks)),
        (Some(x), None) | (None, Some(x)) => {
            if let Some(c) = clocks {
                // Clocked existence (spec D2): the deletion's tombstone time
                // against the survivor's row-edit time — the later UTC
                // decision wins, in EITHER direction (a re-add/reopen after
                // the deletion legitimately resurrects). The positional
                // "status/section-only" exception is abolished.
                let key = row_key(&x.key);
                let tomb = c
                    .ledger
                    .get(&key)
                    .filter(|e| e.del && !e.is_quarantined(c.now));
                let edit = valid_stamp(c.ledger, &key, &x.raw, c.now);
                let keep = match (tomb, edit) {
                    (Some(d), Some(e)) => e.t > d.t || (e.t == d.t && e.src > d.src),
                    // A changed survivor with no stamps either way: genuine
                    // three-way still applies — an UNCHANGED row loses to the
                    // deletion, a changed one conservatively stays (deletion
                    // is destructive, resurrection is visible) and reports.
                    (None, None) => {
                        if b.map_or(false, |b| same(b, x)) {
                            return None;
                        }
                        c.diag(format!(
                            "no valid stamp for existence of {} — kept conservatively",
                            key
                        ));
                        true
                    }
                    // Only one side stamped: the stamped decision is the
                    // newer knowledge by construction of the union ledger.
                    (Some(_), None) => false,
                    (None, Some(_)) => true,
                };
                if !keep {
                    return None;
                }
                return Some(Merged {
                    section: x.section.clone(),
                    line: x.raw.clone(),
                });
            }
            if let Some(b) = b {
                if same(b, x) {
                    return None;
                }
                if kind == SharedFileKind::Todo && only_status_or_section_changed(kind, b, x) {
                    return None;
                }
            }
            Some(Merged {
                section: x.section.clone(),
                line: x.raw.clone(),
            })
        }
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------
// Output assembly
// ---------------------------------------------------------------------------

enum OKind {
    Header(String),
    Ent(String, String),
    Other,
}

struct OLine {
    text: String,
    kind: OKind,
}

impl OLine {
    fn blank(&self) -> bool {
        matches!(self.kind, OKind::Other) && self.text.trim().is_empty()
    }
    fn is_header(&self) -> bool {
        matches!(self.kind, OKind::Header(_))
    }
    fn other(text: &str) -> OLine {
        OLine {
            text: text.to_string(),
            kind: OKind::Other,
        }
    }
}

fn find_header(out: &[OLine], key: &str) -> Option<usize> {
    out.iter()
        .position(|l| matches!(&l.kind, OKind::Header(k) if k == key))
}

fn find_ent(out: &[OLine], key: &str) -> Option<usize> {
    out.iter()
        .position(|l| matches!(&l.kind, OKind::Ent(k, _) if k == key))
}

/// Insert a header after the END of the section the anchor sits in (so entities
/// that follow the anchor in the output keep their section), or at the end.
fn insert_header(out: &mut Vec<OLine>, anchor: Option<usize>, key: &str, text: &str) {
    let mut pos = match anchor {
        Some(a) => {
            let mut p = a + 1;
            while p < out.len() && !out[p].is_header() {
                p += 1;
            }
            p
        }
        None => out.len(),
    };
    let mut backed = 0;
    while pos > 0 && out[pos - 1].blank() {
        pos -= 1;
        backed += 1;
    }
    let mut ins = Vec::new();
    if backed > 0 && pos > 0 {
        ins.push(OLine::other(""));
    }
    ins.push(OLine {
        text: text.to_string(),
        kind: OKind::Header(key.to_string()),
    });
    out.splice(pos..pos, ins);
}

/// Insert an entity line as the first line of the section under header `h`.
fn insert_under_header(out: &mut Vec<OLine>, h: usize, line: OLine) {
    let p = h + 1;
    let mut q = p;
    while q < out.len() && out[q].blank() {
        q += 1;
    }
    let at = if q < out.len() && !out[q].is_header() {
        q
    } else {
        p
    };
    out.insert(at, line);
}

/// Anchor in the output for the theirs-line at `i`: the nearest preceding line
/// of theirs that is also in the output.
fn anchor_for(t: &Doc, i: usize, out: &[OLine]) -> Option<usize> {
    for j in (0..i).rev() {
        let found = match &t.lines[j] {
            Line::Header { key, .. } => find_header(out, key),
            Line::Ent(idx) => find_ent(out, &t.ents[*idx].key),
            Line::Other(s) if !s.trim().is_empty() => out
                .iter()
                .position(|l| matches!(l.kind, OKind::Other) && l.text.trim() == s.trim()),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

pub fn merge_shared_file(
    kind: SharedFileKind,
    base: &str,
    ours: &str,
    theirs: &str,
) -> Result<String, MergeError> {
    merge_with_bases(
        kind,
        base,
        ours,
        theirs,
        &HashMap::new(),
        &HashMap::new(),
        None,
    )
}

/// Per-entity base (T-000226). `versions` are the distinct texts of the file
/// that the two histories held; `ours_seq` / `theirs_seq` index into it, OLDEST
/// first, one entry per commit. The whole-file `base` still serves the
/// structure (headers, prose); the base of every entity whose row differs
/// between `ours` and `theirs` is the last state of THAT row (a version of the
/// row, or its absence) common to both histories, found as the last element of
/// the longest common subsequence of the two histories projected on the row.
/// Judging a whole file by when its bytes first appeared goes wrong when the
/// same file content comes back three or more times; aligned as sequences, the
/// branch that is a prefix of the other ends exactly at the shared state. An
/// entity the histories never both held keeps the whole-file base. A row
/// whose LCS ends in absence is ambiguous — its state in `ancestry` (the
/// file at the real merge-base, `None` when there is none) becomes the base
/// instead (T-000255): presence there makes a deletion newer knowledge.
pub fn merge_shared_file_per_entity(
    kind: SharedFileKind,
    base: &str,
    ancestry: Option<&str>,
    versions: &[String],
    ours_seq: &[usize],
    theirs_seq: &[usize],
    ours: &str,
    theirs: &str,
) -> Result<String, MergeError> {
    merge_shared_file_with_ancestors(
        kind,
        base,
        ancestry,
        versions,
        ours_seq,
        theirs_seq,
        &[],
        ours,
        theirs,
    )
}

/// The per-entity merge plus the ancestor-history stale check (T-000260).
/// `ancestors` is the file's ordered content timeline behind AND AT the
/// real merge-base, oldest first, `None` where the file was absent at that
/// commit — empty when there is no merge-base or it could not be read. On
/// top of the per-entity bases this adds: per-header bases (D1, the same
/// LCS keyed on `Doc::hdrs`) and the stale override (D5b) — where the
/// merge-base and `theirs` both hold a PRESENT but different state, theirs'
/// state is stale when the timeline superseded it before the merge-base.
pub fn merge_shared_file_with_ancestors(
    kind: SharedFileKind,
    base: &str,
    ancestry: Option<&str>,
    versions: &[String],
    ours_seq: &[usize],
    theirs_seq: &[usize],
    ancestors: &[Option<String>],
    ours: &str,
    theirs: &str,
) -> Result<String, MergeError> {
    let mut ent_overrides = entity_bases(
        kind, base, ancestry, versions, ours_seq, theirs_seq, ours, theirs,
    );
    let mut hdr_overrides = header_bases(
        kind, base, ancestry, versions, ours_seq, theirs_seq, ours, theirs,
    );
    apply_stale_overrides(
        kind,
        base,
        theirs,
        ancestors,
        &mut ent_overrides,
        &mut hdr_overrides,
    );
    merge_with_bases_clocked(
        kind,
        base,
        ours,
        theirs,
        &ent_overrides,
        &hdr_overrides,
        None,
    )
    .map(|(text, _)| text)
}

/// The clocked entry point (T-000267): the same pipeline, but every
/// both-changed tie and every deletion-vs-edit decision resolves by the union
/// ledger's UTC stamps instead of positional rules. Returns the merged text
/// plus the diagnostics the caller surfaces as sync issues — a merge that had
/// to decide without valid stamps is degraded, never silent.
///
/// Slice 1 scope: entity fields, marker/section (row-level stamp) and
/// existence. Structure keying (`__structure__`) and prose clocks land in the
/// next slice — until then ordering and prose follow the positional assembly.
pub fn merge_shared_file_clocked(
    kind: SharedFileKind,
    base: &str,
    ancestry: Option<&str>,
    versions: &[String],
    ours_seq: &[usize],
    theirs_seq: &[usize],
    ancestors: &[Option<String>],
    ours: &str,
    theirs: &str,
    ledger: &ClockLedger,
) -> Result<(String, Vec<String>), MergeError> {
    let mut ent_overrides = entity_bases(
        kind, base, ancestry, versions, ours_seq, theirs_seq, ours, theirs,
    );
    let mut hdr_overrides = header_bases(
        kind, base, ancestry, versions, ours_seq, theirs_seq, ours, theirs,
    );
    apply_stale_overrides(
        kind,
        base,
        theirs,
        ancestors,
        &mut ent_overrides,
        &mut hdr_overrides,
    );
    let clocks = Clocks::new(ledger);
    let quarantined = ledger.quarantined_keys(clocks.now);
    if !quarantined.is_empty() {
        let shown: Vec<&str> = quarantined.iter().take(3).map(|s| s.as_str()).collect();
        clocks.diag(format!(
            "{} key(s) frozen by far-future stamps (a wrong-clock machine wrote them) — they decide nothing here: {}{}",
            quarantined.len(),
            shown.join(", "),
            if quarantined.len() > 3 { ", ..." } else { "" }
        ));
    }
    let text = merge_with_bases(
        kind,
        base,
        ours,
        theirs,
        &ent_overrides,
        &hdr_overrides,
        Some(&clocks),
    )?;
    let diags = clocks.diags.into_inner();
    Ok((text, diags))
}

/// Thin wrapper over `merge_with_bases` that returns the diagnostics alongside
/// — the clocked public entry needs both, the positional ones discard them.
fn merge_with_bases_clocked(
    kind: SharedFileKind,
    base: &str,
    ours: &str,
    theirs: &str,
    overrides: &HashMap<String, Option<Ent>>,
    header_overrides: &HashMap<String, Option<String>>,
    clocks: Option<&Clocks>,
) -> Result<(String, Vec<String>), MergeError> {
    let text = merge_with_bases(
        kind,
        base,
        ours,
        theirs,
        overrides,
        header_overrides,
        clocks,
    )?;
    let diags = clocks.map(|c| c.diags.borrow().clone()).unwrap_or_default();
    Ok((text, diags))
}

/// True when `theirs` would overwrite `base` with entity or header content
/// the shared timeline superseded before the merge-base — the check the
/// `ours == base` merge shortcut needs before adopting `theirs` (T-000260
/// D5b-gate). The full merge is unnecessary: in the `ours == base` case no
/// LCS override can fire (`lcs_last` lands on the base state itself), so
/// the stale sets alone decide.
pub fn has_stale_remote_state(
    kind: SharedFileKind,
    base: &str,
    theirs: &str,
    ancestors: &[Option<String>],
) -> bool {
    let mut ent = HashMap::new();
    let mut hdr = HashMap::new();
    apply_stale_overrides(kind, base, theirs, ancestors, &mut ent, &mut hdr);
    !ent.is_empty() || !hdr.is_empty()
}

/// The stale-content check (T-000260/D5b). For every entity key and header
/// key where the BASE and THEIRS both hold a present but different state:
/// `p_b` = last ancestor position whose state equals base's, `p_t` = last
/// position equal to theirs'. The state is STALE when `p_b` and `p_t` are
/// both defined and `p_t < p_b` — theirs' state sat on the shared timeline
/// before the merge-base's state superseded it. `p_t` undefined means the
/// state is new (not stale); `p_b` undefined means the bound proves nothing
/// (not stale — never refuse on unproven age). The override the check
/// installs is THEIRS' CURRENT STATE as the effective base, so the ordinary
/// three-way rules keep ours. It fills only keys the LCS rules left
/// untouched (`entry().or_insert`) — exactly the case `found == base`
/// cannot see. Presence/absence stays with T-000255: a row absent at the
/// merge-base is a re-add, never stale.
fn apply_stale_overrides(
    kind: SharedFileKind,
    base: &str,
    theirs: &str,
    ancestors: &[Option<String>],
    entity_overrides: &mut HashMap<String, Option<Ent>>,
    header_overrides: &mut HashMap<String, Option<String>>,
) {
    if ancestors.is_empty() {
        return;
    }
    let (b, t) = (parse(kind, base), parse(kind, theirs));
    let ent_keys: Vec<String> = b
        .by_key
        .keys()
        .filter(|k| {
            t.by_key
                .get(*k)
                .is_some_and(|&i| !same(&b.ents[b.by_key[*k]], &t.ents[i]))
        })
        .cloned()
        .collect();
    let hdr_keys: Vec<String> = b
        .hdrs
        .keys()
        .filter(|k| t.hdrs.get(*k).is_some_and(|x| *x != b.hdrs[*k]))
        .cloned()
        .collect();
    if ent_keys.is_empty() && hdr_keys.is_empty() {
        return;
    }
    // One parse per ancestor version, oldest first.
    let docs: Vec<Option<Doc>> = ancestors
        .iter()
        .map(|s| s.as_deref().map(|s| parse(kind, s)))
        .collect();
    for k in &ent_keys {
        let (Some(&bs), Some(&ts)) = (b.by_key.get(k), t.by_key.get(k)) else {
            continue;
        };
        let (bs, ts) = (&b.ents[bs], &t.ents[ts]);
        let p_b = docs.iter().rposition(|d| match d {
            Some(doc) => doc.by_key.get(k).is_some_and(|&i| same(&doc.ents[i], bs)),
            None => false,
        });
        let p_t = docs.iter().rposition(|d| match d {
            Some(doc) => doc.by_key.get(k).is_some_and(|&i| same(&doc.ents[i], ts)),
            None => false,
        });
        if matches!((p_b, p_t), (Some(pb), Some(pt)) if pt < pb) {
            entity_overrides
                .entry(k.clone())
                .or_insert_with(|| Some(ts.clone()));
        }
    }
    for k in &hdr_keys {
        let (Some(bs), Some(ts)) = (b.hdrs.get(k), t.hdrs.get(k)) else {
            continue;
        };
        let p_b = docs
            .iter()
            .rposition(|d| d.as_ref().and_then(|d| d.hdrs.get(k)) == Some(bs));
        let p_t = docs
            .iter()
            .rposition(|d| d.as_ref().and_then(|d| d.hdrs.get(k)) == Some(ts));
        if matches!((p_b, p_t), (Some(pb), Some(pt)) if pt < pb) {
            header_overrides
                .entry(k.clone())
                .or_insert_with(|| Some(ts.clone()));
        }
    }
}

/// Do two states of one row / header (`None` = absent) agree?
fn same_state<T: PartialEq>(a: Option<&T>, b: Option<&T>) -> bool {
    a == b
}

/// The state sequence of one row / header over a history, consecutive
/// repeats dropped.
fn state_seq<'a, T: PartialEq>(
    seq: &[usize],
    states: &'a [HashMap<String, T>],
    key: &str,
) -> Vec<Option<&'a T>> {
    let mut out: Vec<Option<&T>> = Vec::new();
    for &i in seq {
        let st = states.get(i).and_then(|m| m.get(key));
        if out.last().map_or(true, |p| !same_state(*p, st)) {
            out.push(st);
        }
    }
    out
}

/// Last element of the longest common subsequence (latest on ours, then on
/// theirs, among equals); `None` when the sequences share no state.
fn lcs_last<'a, T: PartialEq>(a: &[Option<&'a T>], b: &[Option<&'a T>]) -> Option<Option<&'a T>> {
    let (n, m) = (a.len(), b.len());
    let mut f = vec![vec![0u32; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            f[i][j] = if same_state(a[i - 1], b[j - 1]) {
                f[i - 1][j - 1] + 1
            } else {
                f[i - 1][j].max(f[i][j - 1])
            };
        }
    }
    let total = f[n][m];
    if total == 0 {
        return None;
    }
    for i in (1..=n).rev() {
        for j in (1..=m).rev() {
            if same_state(a[i - 1], b[j - 1]) && f[i][j] == total && f[i - 1][j - 1] + 1 == total {
                return Some(a[i - 1]);
            }
        }
    }
    None
}

fn entity_bases(
    kind: SharedFileKind,
    base: &str,
    ancestry: Option<&str>,
    versions: &[String],
    ours_seq: &[usize],
    theirs_seq: &[usize],
    ours: &str,
    theirs: &str,
) -> HashMap<String, Option<Ent>> {
    let mut overrides = HashMap::new();
    let (b, o, t) = (parse(kind, base), parse(kind, ours), parse(kind, theirs));
    let anc = ancestry.map(|s| parse(kind, s));
    // Only a row that differs between the sides needs a base.
    let mut diff: HashSet<String> = HashSet::new();
    for e in o.ents.iter().chain(t.ents.iter()) {
        let eo = o.by_key.get(&e.key).map(|&i| &o.ents[i]);
        let et = t.by_key.get(&e.key).map(|&i| &t.ents[i]);
        if !same_state(eo, et) {
            diff.insert(e.key.clone());
        }
    }
    if diff.is_empty() {
        return overrides;
    }
    // One parse per version, keeping only the rows in question.
    let used: HashSet<usize> = ours_seq.iter().chain(theirs_seq.iter()).copied().collect();
    let mut states: Vec<HashMap<String, Ent>> = vec![HashMap::new(); versions.len()];
    for &i in &used {
        let Some(text) = versions.get(i) else {
            continue;
        };
        let mut m = HashMap::new();
        for e in parse(kind, text).ents {
            if diff.contains(&e.key) {
                m.insert(e.key.clone(), e);
            }
        }
        states[i] = m;
    }
    for key in &diff {
        let a = state_seq(ours_seq, &states, key);
        let bb = state_seq(theirs_seq, &states, key);
        if let Some(found) = lcs_last(&a, &bb) {
            // An absent tail of the LCS is never a usable base (T-000255):
            // absence is fungible, so the LCS may have aligned unrelated
            // gaps — a stale copy on our side vs a fresh deletion on
            // theirs, or a re-add ours made after a deletion both sides
            // shared. Row-only history cannot tell those apart; the
            // merge-base's file can — a base file holding the row makes
            // the deletion newer knowledge (it wins), a base file without
            // the row (or no merge-base at all) makes ours' row newer
            // (a re-add survives).
            if found.is_none() {
                let st = anc
                    .as_ref()
                    .and_then(|p| p.by_key.get(key).map(|&i| &p.ents[i]));
                overrides.insert(key.clone(), st.cloned());
                continue;
            }
            let whole = b.by_key.get(key).map(|&i| &b.ents[i]);
            if !same_state(found, whole) {
                overrides.insert(key.clone(), found.cloned());
            }
        }
    }
    overrides
}

/// Per-header-key base (T-000260/D1): the same provenance `entity_bases`
/// gives rows, applied to `## ` header lines — keyed on the FIRST
/// occurrence of each key (`Doc::hdrs`). The override maps a contested key
/// (ours' text != theirs' text) to the effective base text; `None` = the
/// header was absent in that state. An override equal to the whole-file
/// base entry is not stored, same convention as entities.
fn header_bases(
    kind: SharedFileKind,
    base: &str,
    ancestry: Option<&str>,
    versions: &[String],
    ours_seq: &[usize],
    theirs_seq: &[usize],
    ours: &str,
    theirs: &str,
) -> HashMap<String, Option<String>> {
    let mut overrides = HashMap::new();
    let (b, o, t) = (parse(kind, base), parse(kind, ours), parse(kind, theirs));
    let anc = ancestry.map(|s| parse(kind, s));
    let mut diff: HashSet<String> = HashSet::new();
    for key in o.hdrs.keys().chain(t.hdrs.keys()) {
        if o.hdrs.get(key) != t.hdrs.get(key) {
            diff.insert(key.clone());
        }
    }
    if diff.is_empty() {
        return overrides;
    }
    let used: HashSet<usize> = ours_seq.iter().chain(theirs_seq.iter()).copied().collect();
    let mut states: Vec<HashMap<String, String>> = vec![HashMap::new(); versions.len()];
    for &i in &used {
        let Some(text) = versions.get(i) else {
            continue;
        };
        let mut m = HashMap::new();
        for (key, text) in parse(kind, text).hdrs {
            if diff.contains(&key) {
                m.insert(key, text);
            }
        }
        states[i] = m;
    }
    for key in &diff {
        let a = state_seq(ours_seq, &states, key);
        let bb = state_seq(theirs_seq, &states, key);
        if let Some(found) = lcs_last(&a, &bb) {
            // The T-000255 rule applies to headers too: an LCS ending in
            // absence is ambiguous, so the real merge-base's copy (its
            // `Doc::hdrs` entry) becomes the base.
            if found.is_none() {
                let st = anc.as_ref().and_then(|p| p.hdrs.get(key)).cloned();
                overrides.insert(key.clone(), st);
                continue;
            }
            if found != b.hdrs.get(key) {
                overrides.insert(key.clone(), found.cloned());
            }
        }
    }
    overrides
}

/// The verbatim `## ` occurrences of every key in file order — what the
/// trivial `base == ours → theirs` shortcut compares before adopting
/// `theirs` wholesale (T-000260/D5c).
fn header_occurrences(input: &str) -> HashMap<String, Vec<String>> {
    let s = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut m: HashMap<String, Vec<String>> = HashMap::new();
    for l in s.split('\n') {
        let l = l.strip_suffix('\r').unwrap_or(l);
        if let Some(rest) = l.trim().strip_prefix("## ") {
            m.entry(section_key(rest)).or_default().push(l.to_string());
        }
    }
    m
}

/// The structural fingerprint of a parsed document: the ordered sequence of
/// section-header keys and entity keys — exactly what `__structure__` stamps
/// bind to. Prose lines are deliberately outside the fingerprint: prose carries
/// its own content-hash keys and never reorders entities.
pub(crate) fn skeleton_fingerprint(doc: &Doc) -> String {
    let mut s = String::new();
    for l in &doc.lines {
        match l {
            Line::Header { key, .. } => {
                s.push_str("H:");
                s.push_str(key);
                s.push('\n');
            }
            Line::Ent(i) => {
                s.push_str("E:");
                s.push_str(&doc.ents[*i].key);
                s.push('\n');
            }
            _ => {}
        }
    }
    crate::sync::edit_clocks::value_hash(s.as_bytes())
}

/// Which side's skeleton the assembly follows in clocked mode: the side whose
/// fingerprint the `__structure__` stamp validly binds to. The ledger holds at
/// most one structure entry, so at most one side can match; a neither-match is
/// degraded — ours keeps the skeleton (positional assembly is unchanged) and
/// the key is reported.
fn structure_prefers_theirs(clocks: &Clocks, o: &Doc, t: &Doc) -> bool {
    use crate::sync::edit_clocks::STRUCTURE_KEY;
    let ho = skeleton_fingerprint(o);
    let ht = skeleton_fingerprint(t);
    if ho == ht {
        return false;
    }
    let e = match clocks.ledger.get(STRUCTURE_KEY) {
        Some(e) if !e.del && !e.is_quarantined(clocks.now) => e,
        _ => {
            // Diverging skeletons with no structure stamp at all is degraded
            // mode: the positional fallback below is ours-relative, so it
            // must be loud — the caller turns this into a sync issue.
            clocks.diag(format!(
                "missing {} stamp for diverging skeletons — ordering is positional",
                STRUCTURE_KEY
            ));
            return false;
        }
    };
    match (e.h == ho, e.h == ht) {
        (true, _) => false,
        (false, true) => true,
        (false, false) => {
            clocks.diag(format!(
                "no valid {} stamp for either skeleton — ours keeps the ordering",
                STRUCTURE_KEY
            ));
            false
        }
    }
}

fn merge_with_bases(
    kind: SharedFileKind,
    base: &str,
    ours: &str,
    theirs: &str,
    overrides: &HashMap<String, Option<Ent>>,
    header_overrides: &HashMap<String, Option<String>>,
    clocks: Option<&Clocks>,
) -> Result<String, MergeError> {
    // Trivial merges: exactly what git itself would do. The base ones stand
    // for the whole file only while no row or header has a base of its own
    // (T-000260/D3 — a header override is exactly the evidence that flat
    // `base == ours` would adopt stale bytes). And adopting `theirs`
    // verbatim must honour the first-occurrence header policy (D5c): it is
    // allowed only when theirs' header occurrences per key match ours'
    // exactly, or the full path runs and keeps ours' later duplicates.
    if ours == theirs {
        return Ok(ours.to_string());
    }
    if overrides.is_empty() && header_overrides.is_empty() {
        let ours_headers = header_occurrences(ours);
        if base == ours && (ours_headers.is_empty() || ours_headers == header_occurrences(theirs)) {
            return Ok(theirs.to_string());
        }
        if base == theirs {
            return Ok(ours.to_string());
        }
    }

    let b = parse(kind, base);
    let o = parse(kind, ours);
    let t = parse(kind, theirs);

    // Clocked structure (spec D2/T-000267 slice 2): the assembly skeleton is
    // the side with the newer valid `__structure__` stamp — entity resolution
    // is direction-independent under clocks, so swapping the parses for the
    // whole pipeline below cannot skew content, only ordering and prose.
    let (o, t) = match clocks {
        Some(c) if structure_prefers_theirs(c, &o, &t) => (t, o),
        _ => (o, t),
    };

    // Decide every entity.
    let mut merged: HashMap<String, Option<Merged>> = HashMap::new();
    for e in o.ents.iter().chain(t.ents.iter()) {
        if merged.contains_key(&e.key) {
            continue;
        }
        let eb = match overrides.get(&e.key) {
            Some(x) => x.as_ref(),
            None => b.by_key.get(&e.key).map(|&i| &b.ents[i]),
        };
        let eo = o.by_key.get(&e.key).map(|&i| &o.ents[i]);
        let et = t.by_key.get(&e.key).map(|&i| &t.ents[i]);
        merged.insert(e.key.clone(), decide(kind, eb, eo, et, clocks));
    }

    // Step A: ours skeleton.
    let mut out: Vec<OLine> = Vec::new();
    let mut placed: HashSet<String> = HashSet::new();
    for l in &o.lines {
        match l {
            Line::Header { key, text } => out.push(OLine {
                text: text.clone(),
                kind: OKind::Header(key.clone()),
            }),
            Line::Other(s) | Line::Dup(s) => out.push(OLine::other(s)),
            Line::Ent(i) => {
                let e = &o.ents[*i];
                if let Some(Some(m)) = merged.get(&e.key) {
                    if m.section == e.section {
                        placed.insert(e.key.clone());
                        out.push(OLine {
                            text: m.line.clone(),
                            kind: OKind::Ent(e.key.clone(), m.section.clone()),
                        });
                    }
                }
            }
        }
    }

    // Step B: headers / prose that exist only in theirs.
    let mut known: HashSet<String> = HashSet::new();
    let mut o_hdr: HashMap<String, String> = HashMap::new();
    for l in &o.lines {
        match l {
            Line::Other(s) | Line::Dup(s) => {
                known.insert(s.trim().to_string());
            }
            Line::Header { key, text } => {
                o_hdr.entry(key.clone()).or_insert_with(|| text.clone());
            }
            _ => {}
        }
    }
    let mut b_hdr: HashMap<String, String> = HashMap::new();
    for l in &b.lines {
        match l {
            Line::Other(s) | Line::Dup(s) => {
                known.insert(s.trim().to_string());
            }
            Line::Header { key, text } => {
                b_hdr.entry(key.clone()).or_insert_with(|| text.clone());
            }
            _ => {}
        }
    }
    // T-000260/D5c: the FIRST occurrence of each theirs header key decides;
    // later duplicates of theirs never rename ours' first nor insert again.
    let mut seen_hdr: HashSet<String> = HashSet::new();
    for (i, l) in t.lines.iter().enumerate() {
        match l {
            Line::Header { key, text } => {
                if !seen_hdr.insert(key.clone()) {
                    continue;
                }
                if let Some(otext) = o_hdr.get(key) {
                    // T-000260/D2: the per-header base (LCS or stale override)
                    // stands in for the whole-file one where it exists. Only
                    // theirs renaming the effective base's text wins — a base
                    // equal to theirs' current text marks ours' text the
                    // newer knowledge, so the stale rename is refused.
                    let hb = match header_overrides.get(key) {
                        Some(h) => h.as_ref(),
                        None => b_hdr.get(key),
                    };
                    let adopt = if hb == Some(otext) && text != otext {
                        true
                    } else if let (Some(c), true) = (clocks, text != otext) {
                        // Clocked rename (T-000267): both sides hold the key
                        // with different texts and the base rules did not
                        // decide — the later valid stamp wins, either way.
                        use crate::sync::edit_clocks::header_key;
                        pick_clocked(c, &header_key(key), otext, text) == text.as_str()
                    } else {
                        false
                    };
                    if adopt {
                        if let Some(p) = find_header(&out, key) {
                            out[p].text = text.clone();
                        }
                    }
                } else {
                    let base_had = match header_overrides.get(key) {
                        Some(h) => h.is_some(),
                        None => b_hdr.contains_key(key),
                    };
                    if !base_had && find_header(&out, key).is_none() {
                        let anchor = anchor_for(&t, i, &out);
                        insert_header(&mut out, anchor, key, text);
                    }
                }
            }
            Line::Other(s) => {
                let tr = s.trim();
                if !tr.is_empty() && !known.contains(tr) {
                    let anchor = anchor_for(&t, i, &out);
                    let at = anchor.map_or(out.len(), |a| a + 1);
                    out.insert(at, OLine::other(s));
                    known.insert(tr.to_string());
                }
            }
            _ => {}
        }
    }

    // Step C: entities that are not in place yet (theirs-only or moved).
    for (ti, te) in t.ents.iter().enumerate() {
        let m = match merged.get(&te.key) {
            Some(Some(m)) => m,
            _ => continue,
        };
        if placed.contains(&te.key) {
            continue;
        }
        let line = OLine {
            text: m.line.clone(),
            kind: OKind::Ent(te.key.clone(), m.section.clone()),
        };
        // Right after its predecessor in theirs, when that sits in the same section.
        if ti > 0 {
            let pk = &t.ents[ti - 1].key;
            if let Some(p) = out
                .iter()
                .position(|l| matches!(&l.kind, OKind::Ent(k, s) if k == pk && *s == m.section))
            {
                out.insert(p + 1, line);
                continue;
            }
        }
        if m.section.is_empty() {
            let mut pos = out.iter().position(|l| l.is_header()).unwrap_or(out.len());
            while pos > 0 && out[pos - 1].blank() {
                pos -= 1;
            }
            out.insert(pos, line);
            continue;
        }
        if find_header(&out, &m.section).is_none() {
            let text = t
                .hdrs
                .get(&m.section)
                .or_else(|| o.hdrs.get(&m.section))
                .or_else(|| b.hdrs.get(&m.section))
                .cloned()
                .unwrap_or_else(|| format!("## {}", m.section));
            let anchor = t
                .lines
                .iter()
                .position(|l| matches!(l, Line::Header { key, .. } if *key == m.section))
                .and_then(|hi| anchor_for(&t, hi, &out));
            insert_header(&mut out, anchor, &m.section, &text);
        }
        let h = find_header(&out, &m.section).expect("header just ensured");
        insert_under_header(&mut out, h, line);
    }

    // Render with ours' conventions.
    let eol = if o.crlf { "\r\n" } else { "\n" };
    let trailing = if o.empty {
        t.trailing_nl
    } else {
        o.trailing_nl
    };
    let mut s = String::new();
    if o.bom {
        s.push('\u{feff}');
    }
    for (i, l) in out.iter().enumerate() {
        if i > 0 {
            s.push_str(eol);
        }
        s.push_str(&l.text);
    }
    if trailing && !out.is_empty() {
        s.push_str(eol);
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use SharedFileKind::*;

    fn m(kind: SharedFileKind, b: &str, o: &str, t: &str) -> String {
        merge_shared_file(kind, b, o, t).unwrap()
    }

    fn lines(s: &str) -> Vec<&str> {
        s.lines().collect()
    }

    const TODO_BASE: &str = "# Todo\n\n## v1.20.0 — contributor mode\n\n- [ ] T-000001 | first task | 2 | high | open | 2026-10-01\n- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n- [ ] T-000003 | third task | 3 | medium | open | 2026-10-03\n";

    #[test]
    fn kind_for_path_variants() {
        assert_eq!(kind_for_path("docs/todo.md"), Some(Todo));
        assert_eq!(kind_for_path("docs\\done.md"), Some(Done));
        assert_eq!(kind_for_path("Docs/Bug-Reports.MD"), Some(Bugs));
        assert_eq!(kind_for_path("./docs/todo.md"), Some(Todo));
        assert_eq!(kind_for_path("docs/other.md"), None);
        assert_eq!(kind_for_path("README.md"), None);
        assert_eq!(kind_for_path("x/docs/todo.md"), None);
    }

    #[test]
    fn bytes_entry_rejects_non_utf8() {
        let r = merge_shared_bytes(Todo, b"", &[0xff, 0xfe, 0x41], b"x");
        assert_eq!(r, Err(MergeError::NonUtf8));
    }

    #[test]
    fn different_tasks_edited_on_both_sides() {
        let ours = TODO_BASE.replace("first task", "first task EDITED");
        let theirs = TODO_BASE.replace(
            "third task | 3 | medium | open",
            "third task | 5 | medium | open",
        );
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(r.contains("T-000001 | first task EDITED | 2"));
        assert!(r.contains("T-000003 | third task | 5 | medium"));
        assert_eq!(lines(&r).len(), lines(TODO_BASE).len());
    }

    #[test]
    fn same_task_status_both_sides_further_wins() {
        let ours = TODO_BASE.replace(
            "T-000002 | second task | 1 | low | open",
            "T-000002 | second task | 1 | low | in-progress",
        );
        let theirs = TODO_BASE.replace(
            "T-000002 | second task | 1 | low | open",
            "T-000002 | second task | 1 | low | review",
        );
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(r.contains("second task | 1 | low | review | 2026-10-02"));
        // and the other way round
        let r2 = m(Todo, TODO_BASE, &theirs, &ours);
        assert!(r2.contains("second task | 1 | low | review | 2026-10-02"));
    }

    #[test]
    fn unknown_status_ranks_lowest() {
        let ours = TODO_BASE.replace(
            "T-000002 | second task | 1 | low | open",
            "T-000002 | second task | 1 | low | weird",
        );
        let theirs = TODO_BASE.replace(
            "T-000002 | second task | 1 | low | open",
            "T-000002 | second task | 1 | low | open!",
        );
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        // both unknown -> tie -> ours
        assert!(r.contains("| low | weird |"));
        let theirs2 = TODO_BASE.replace(
            "T-000002 | second task | 1 | low | open",
            "T-000002 | second task | 1 | low | in-progress",
        );
        let r2 = m(Todo, TODO_BASE, &ours, &theirs2);
        assert!(r2.contains("| low | in-progress |"));
    }

    #[test]
    fn description_both_sides_ours_wins_other_fields_merge() {
        let ours = TODO_BASE.replace("second task | 1 | low", "наша правка | 1 | low");
        let theirs = TODO_BASE.replace(
            "second task | 1 | low | open",
            "чужая правка | 4 | low | open",
        );
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        // description: ours; effort changed only by theirs: theirs
        assert!(
            r.contains("T-000002 | наша правка | 4 | low | open | 2026-10-02"),
            "{}",
            r
        );
    }

    #[test]
    fn delete_vs_unchanged_is_deleted() {
        let ours = TODO_BASE.replace(
            "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
            "",
        );
        let theirs = TODO_BASE.replace("first task", "first task v2");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(!r.contains("T-000002"));
        assert!(r.contains("first task v2"));
        // symmetric
        let r2 = m(Todo, TODO_BASE, &theirs, &ours);
        assert!(!r2.contains("T-000002"));
        assert!(r2.contains("first task v2"));
    }

    #[test]
    fn delete_vs_description_edit_is_kept() {
        let ours = TODO_BASE.replace(
            "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
            "",
        );
        let theirs = TODO_BASE.replace("second task", "second task reworded");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(r.contains("T-000002 | second task reworded"));
        let r2 = m(Todo, TODO_BASE, &theirs, &ours);
        assert!(r2.contains("T-000002 | second task reworded"));
    }

    #[test]
    fn todo_delete_vs_status_only_edit_is_deleted() {
        let ours = TODO_BASE.replace(
            "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
            "",
        );
        let theirs = TODO_BASE.replace(
            "second task | 1 | low | open",
            "second task | 1 | low | in-progress",
        );
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(!r.contains("T-000002"));
        let r2 = m(Todo, TODO_BASE, &theirs, &ours);
        assert!(!r2.contains("T-000002"));
    }

    #[test]
    fn todo_delete_vs_section_move_only_is_deleted() {
        let base = format!(
            "{}\n## v1.21.0\n\n- [ ] T-000009 | later | 1 | low | open | 2026-10-09\n",
            TODO_BASE
        );
        let ours = base.replace(
            "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
            "",
        );
        // theirs moves T-000002 into v1.21.0
        let theirs = base
            .replace(
                "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
                "",
            )
            .replace(
                "- [ ] T-000009",
                "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n- [ ] T-000009",
            );
        let r = m(Todo, &base, &ours, &theirs);
        assert!(!r.contains("T-000002"), "{}", r);
    }

    #[test]
    fn bugs_delete_vs_status_edit_is_kept() {
        // the status-only exception is todo.md only
        let base = "# Bug reports\n\n## Open bugs\n\n- B-000001 | 2026-10-01 | broken | major | logic | created | 0 | \n";
        let ours = "# Bug reports\n\n## Open bugs\n\n";
        let theirs = base.replace("| created |", "| in-progress |");
        let r = m(Bugs, base, ours, &theirs);
        assert!(r.contains("B-000001") && r.contains("in-progress"));
    }

    #[test]
    fn added_on_both_sides_different_sections() {
        let base = format!(
            "{}\n## v1.21.0\n\n- [ ] T-000009 | later | 1 | low | open | 2026-10-09\n",
            TODO_BASE
        );
        let ours = base.replace(
            "- [ ] T-000003",
            "- [ ] T-000010 | ours new | 1 | low | open | 2026-10-10\n- [ ] T-000003",
        );
        let theirs = base.replace(
            "- [ ] T-000009",
            "- [ ] T-000011 | theirs new | 1 | low | open | 2026-10-11\n- [ ] T-000009",
        );
        let r = m(Todo, &base, &ours, &theirs);
        let l = lines(&r);
        let pos = |needle: &str| l.iter().position(|x| x.contains(needle)).unwrap();
        let h2 = pos("## v1.21.0");
        assert!(pos("T-000010") < h2, "ours new stays in v1.20.0");
        assert!(pos("T-000011") > h2, "theirs new lands in v1.21.0");
        assert_eq!(
            pos("T-000011"),
            pos("T-000009") - 1,
            "inserted after predecessor... or before? {}",
            r
        );
    }

    #[test]
    fn added_after_predecessor_in_same_section() {
        let theirs = TODO_BASE.replace(
            "- [ ] T-000003",
            "- [ ] T-000010 | between | 1 | low | open | 2026-10-10\n- [ ] T-000003",
        );
        let ours = TODO_BASE.replace("third task", "third task v2");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        let l = lines(&r);
        assert!(l[5].contains("T-000002"));
        assert!(l[6].contains("T-000010"));
        assert!(l[7].contains("T-000003") && l[7].contains("third task v2"));
    }

    #[test]
    fn new_section_only_in_theirs_is_created() {
        let theirs = format!(
            "{}\n## v1.21.0 — next\n\n- [ ] T-000020 | next thing | 1 | low | open | 2026-10-20\n",
            TODO_BASE
        );
        let ours = TODO_BASE.replace("first task", "first task v2");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(r.contains("first task v2"));
        let l = lines(&r);
        let h = l.iter().position(|x| x.starts_with("## v1.21.0")).unwrap();
        assert!(l[h + 1..].iter().any(|x| x.contains("T-000020")));
        assert!(l.iter().position(|x| x.contains("T-000003")).unwrap() < h);
    }

    #[test]
    fn task_moved_between_versions_follows_the_move() {
        let base = format!(
            "{}\n## v1.21.0\n\n- [ ] T-000009 | later | 1 | low | open | 2026-10-09\n",
            TODO_BASE
        );
        let ours = base.replace("first task", "first task edited");
        // theirs moves T-000001 to v1.21.0
        let theirs = base
            .replace(
                "- [ ] T-000001 | first task | 2 | high | open | 2026-10-01\n",
                "",
            )
            .replace(
                "- [ ] T-000009",
                "- [ ] T-000001 | first task | 2 | high | open | 2026-10-01\n- [ ] T-000009",
            );
        let r = m(Todo, &base, &ours, &theirs);
        let l = lines(&r);
        let h2 = l.iter().position(|x| x.starts_with("## v1.21.0")).unwrap();
        let p1 = l.iter().position(|x| x.contains("T-000001")).unwrap();
        assert!(p1 > h2, "{}", r);
        assert!(l[p1].contains("first task edited"));
        assert_eq!(l.iter().filter(|x| x.contains("T-000001")).count(), 1);
    }

    #[test]
    fn checkbox_both_changed_ours_wins() {
        let ours = TODO_BASE.replace("- [ ] T-000001", "- [x] T-000001");
        let theirs = TODO_BASE.replace(
            "- [ ] T-000001 | first task | 2 | high | open",
            "- [X] T-000001 | first task | 2 | high | review",
        );
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(
            r.contains("- [x] T-000001 | first task | 2 | high | review | 2026-10-01"),
            "{}",
            r
        );
    }

    #[test]
    fn escaped_pipe_survives() {
        let base = "## v1\n\n- [ ] T-000001 | regex a\\|b here | 1 | low | open | 2026-10-01\n- [ ] T-000002 | other | 1 | low | open | 2026-10-02\n";
        let ours = base.replace(
            "| 1 | low | open | 2026-10-01",
            "| 1 | low | review | 2026-10-01",
        );
        let theirs = base.replace("other", "other v2");
        let r = m(Todo, base, &ours, &theirs);
        assert!(
            r.contains("T-000001 | regex a\\|b here | 1 | low | review | 2026-10-01"),
            "{}",
            r
        );
        assert!(r.contains("other v2"));
        // an escaped pipe in a field that has to be re-joined
        let theirs2 = base.replace(
            "| 1 | low | open | 2026-10-01",
            "| 3 | low | open | 2026-10-01",
        );
        let r2 = m(Todo, base, &ours, &theirs2);
        assert!(
            r2.contains("T-000001 | regex a\\|b here | 3 | low | review | 2026-10-01"),
            "{}",
            r2
        );
    }

    #[test]
    fn legacy_five_field_todo_line() {
        let base =
            "## v1\n\n- T-001 | old style | M | open\n- [ ] T-002 | five | L | high | open\n";
        let ours = base.replace(
            "T-002 | five | L | high | open",
            "T-002 | five | L | high | in-progress",
        );
        let theirs = base.replace(
            "T-002 | five | L | high | open",
            "T-002 | five | L | high | review",
        );
        let r = m(Todo, base, &ours, &theirs);
        assert!(r.contains("T-002 | five | L | high | review"), "{}", r);
        assert!(r.contains("- T-001 | old style | M | open"));
        // 4-field line: status is the last field
        let o4 = base.replace("old style | M | open", "old style | M | in-progress");
        let t4 = base.replace("old style | M | open", "old style | M | review");
        let r4 = m(Todo, base, &o4, &t4);
        assert!(r4.contains("- T-001 | old style | M | review"), "{}", r4);
    }

    #[test]
    fn prose_and_blank_lines_come_from_ours() {
        let ours = format!(
            "{}\nsome note of ours\n",
            TODO_BASE.replace("# Todo", "# Todo (ours)")
        );
        let theirs = TODO_BASE.replace("third task", "third task v2");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(r.starts_with("# Todo (ours)\n"));
        assert!(r.contains("some note of ours"));
        assert!(r.contains("third task v2"));
    }

    #[test]
    fn theirs_only_prose_line_is_added() {
        let theirs = format!("{}\nnote from theirs\n", TODO_BASE);
        let ours = TODO_BASE.replace("first task", "first task v2");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(r.contains("note from theirs"));
        assert!(r.contains("first task v2"));
        assert_eq!(r.matches("note from theirs").count(), 1);
    }

    // ---- done.md ----

    const DONE_BASE: &str = "# Done\n\n## 2026-10-01\n- T-000001 | did a thing | v1.19.0\n-  | legacy without id | v1.19.0\n\n## 2026-10-02\n- T-000002 | did another | v1.19.0\n";

    #[test]
    fn done_union_with_new_date_header_and_empty_id() {
        let ours = DONE_BASE.replace(
            "## 2026-10-02\n",
            "## 2026-10-02\n- T-000005 | ours done | v1.20.0\n",
        );
        let theirs = format!(
            "{}\n## 2026-10-20\n- T-000007 | theirs done | v1.20.0\n-  | theirs legacy | v1.20.0\n",
            DONE_BASE
        );
        let r = m(Done, DONE_BASE, &ours, &theirs);
        let l = lines(&r);
        let h = l.iter().position(|x| *x == "## 2026-10-20").expect(&r);
        assert!(l[h + 1..].iter().any(|x| x.contains("T-000007")));
        assert!(l[h + 1..].iter().any(|x| x.contains("theirs legacy")));
        assert!(r.contains("T-000005 | ours done"));
        assert!(r.contains("-  | legacy without id | v1.19.0"));
        assert_eq!(r.matches("legacy without id").count(), 1);
        // old date sections are untouched in front
        assert!(l.iter().position(|x| x.contains("T-000002")).unwrap() < h);
    }

    #[test]
    fn done_empty_id_deleted_on_one_side() {
        let ours = DONE_BASE.replace("-  | legacy without id | v1.19.0\n", "");
        let theirs = DONE_BASE.replace("did another", "did another v2");
        let r = m(Done, DONE_BASE, &ours, &theirs);
        assert!(!r.contains("legacy without id"));
        assert!(r.contains("did another v2"));
    }

    #[test]
    fn done_version_conflict_ours_wins() {
        let ours = DONE_BASE.replace("did a thing | v1.19.0", "did a thing | v1.20.0");
        let theirs = DONE_BASE.replace("did a thing | v1.19.0", "did a thing | v1.21.0");
        let r = m(Done, DONE_BASE, &ours, &theirs);
        assert!(r.contains("T-000001 | did a thing | v1.20.0"));
    }

    #[test]
    fn done_same_task_added_on_both_sides_not_duplicated() {
        let base = "## 2026-10-01\n- T-000001 | a | v1\n";
        let ours = "## 2026-10-01\n- T-000001 | a | v1\n- T-000002 | same | v2\n";
        let theirs =
            "## 2026-10-01\n- T-000001 | a | v1\n- T-000002 | same | v2\n- T-000003 | extra | v2\n";
        let r = m(Done, base, ours, theirs);
        assert_eq!(r.matches("T-000002").count(), 1);
        assert!(r.contains("T-000003"));
        // empty base (add/add)
        let r2 = m(Done, "", ours, theirs);
        assert_eq!(r2.matches("T-000002").count(), 1);
        assert!(r2.contains("T-000003"));
    }

    // ---- bug-reports.md ----

    const BUGS_BASE: &str = "# Bug reports\n\n## Open bugs\n\n- B-000001 | 2026-10-01 | first bug | major | logic | created | 0 | \n- B-000002 | 2026-10-02 | second bug | minor | ui_ux | in-progress | 1 | looking\n";

    #[test]
    fn bugs_status_order_and_attempts_max() {
        let ours = BUGS_BASE.replace(
            "B-000002 | 2026-10-02 | second bug | minor | ui_ux | in-progress | 1 | looking",
            "B-000002 | 2026-10-02 | second bug | minor | ui_ux | testing | 2 | fixed it",
        );
        let theirs = BUGS_BASE.replace(
            "B-000002 | 2026-10-02 | second bug | minor | ui_ux | in-progress | 1 | looking",
            "B-000002 | 2026-10-02 | second bug | minor | ui_ux | rejected | 3 | no, still broken",
        );
        let r = m(Bugs, BUGS_BASE, &ours, &theirs);
        // status: rejected > testing; attempts: max(2,3)=3; comment both changed -> ours
        assert!(
            r.contains(
                "B-000002 | 2026-10-02 | second bug | minor | ui_ux | rejected | 3 | fixed it"
            ),
            "{}",
            r
        );
        // confirmed beats rejected
        let theirs2 = BUGS_BASE.replace("in-progress | 1 | looking", "confirmed | 1 | looking");
        let r2 = m(Bugs, BUGS_BASE, &theirs, &theirs2);
        assert!(r2.contains("| confirmed |"), "{}", r2);
        // created < in-progress
        let o3 = BUGS_BASE.replace(
            "first bug | major | logic | created",
            "first bug | major | logic | in-progress",
        );
        let t3 = BUGS_BASE.replace(
            "first bug | major | logic | created | 0 | ",
            "first bug | major | logic | created | 0 | note",
        );
        let r3 = m(Bugs, BUGS_BASE, &o3, &t3);
        assert!(
            r3.contains(
                "B-000001 | 2026-10-01 | first bug | major | logic | in-progress | 0 | note"
            ),
            "{}",
            r3
        );
    }

    #[test]
    fn bugs_added_on_both_sides() {
        let ours = format!(
            "{}- B-000003 | 2026-10-03 | ours bug | minor | other | created | 0 | \n",
            BUGS_BASE
        );
        let theirs = format!(
            "{}- B-000004 | 2026-10-04 | theirs bug | minor | other | created | 0 | \n",
            BUGS_BASE
        );
        let r = m(Bugs, BUGS_BASE, &ours, &theirs);
        assert!(r.contains("B-000003") && r.contains("B-000004"));
        let l = lines(&r);
        assert!(
            l.iter().position(|x| x.contains("B-000004")).unwrap()
                > l.iter().position(|x| x.contains("B-000002")).unwrap()
        );
    }

    // ---- line endings ----

    #[test]
    fn crlf_preserved_from_ours() {
        let crlf = |s: &str| s.replace('\n', "\r\n");
        let base = crlf(TODO_BASE);
        let ours = base.replace("first task", "first task v2");
        let theirs = crlf(&TODO_BASE.replace(
            "- [ ] T-000003",
            "- [ ] T-000010 | new | 1 | low | open | 2026-10-10\n- [ ] T-000003",
        ));
        let r = m(Todo, &base, &ours, &theirs);
        assert!(r.contains("\r\n"));
        assert!(!r.replace("\r\n", "").contains('\n'), "bare LF in output");
        assert!(r.contains("T-000010"));
        assert!(r.ends_with("\r\n"));
        // ours LF, theirs CRLF -> LF output
        let r2 = m(
            Todo,
            TODO_BASE,
            &TODO_BASE.replace("first task", "first task v2"),
            &crlf(&TODO_BASE.replace("third task", "third task v2")),
        );
        assert!(!r2.contains('\r'));
        assert!(r2.contains("first task v2") && r2.contains("third task v2"));
    }

    #[test]
    fn trailing_newline_state_follows_ours() {
        let ours = TODO_BASE
            .trim_end_matches('\n')
            .replace("first task", "first task v2");
        let theirs = TODO_BASE.replace("third task", "third task v2");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(!r.ends_with('\n'));
        assert!(r.contains("third task v2"));
        let r2 = m(
            Todo,
            TODO_BASE,
            &(ours.clone() + "\n"),
            &theirs.trim_end_matches('\n'),
        );
        assert!(r2.ends_with('\n'));
    }

    #[test]
    fn bom_kept_and_not_duplicated() {
        let ours = format!(
            "\u{feff}{}",
            TODO_BASE.replace("first task", "first task v2")
        );
        let base = format!("\u{feff}{}", TODO_BASE);
        let theirs = format!(
            "\u{feff}{}",
            TODO_BASE.replace("third task", "third task v2")
        );
        let r = m(Todo, &base, &ours, &theirs);
        assert!(r.starts_with('\u{feff}'));
        assert_eq!(r.matches("# Todo").count(), 1);
    }

    // ---- idempotence ----

    #[test]
    fn idempotence_properties() {
        let ours = TODO_BASE.replace("first task", "first task v2");
        let theirs = TODO_BASE.replace("third task", "third task v2").replace(
            "- [ ] T-000003",
            "- [ ] T-000010 | n | 1 | low | open | 2026-10-10\n- [ ] T-000003",
        );
        for kind_case in [(Todo, TODO_BASE), (Done, DONE_BASE), (Bugs, BUGS_BASE)] {
            let (k, f) = kind_case;
            assert_eq!(m(k, f, f, f), f);
        }
        assert_eq!(m(Todo, TODO_BASE, TODO_BASE, &theirs), theirs);
        assert_eq!(m(Todo, TODO_BASE, &ours, TODO_BASE), ours);
        assert_eq!(m(Todo, TODO_BASE, &ours, &ours), ours);
        assert_eq!(m(Todo, "", "", &theirs), theirs);
        // swapping ours/theirs on disjoint edits yields the same set of lines
        let a = m(Todo, TODO_BASE, &ours, &theirs);
        let b = m(Todo, TODO_BASE, &theirs, &ours);
        let mut la = lines(&a);
        let mut lb = lines(&b);
        la.sort();
        lb.sort();
        assert_eq!(la, lb);
        // merging the result again with either side changes nothing
        assert_eq!(m(Todo, TODO_BASE, &a, &ours), a);
    }

    #[test]
    fn duplicate_ids_in_one_file_do_not_multiply() {
        let base = "## v1\n\n- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n- [ ] T-000001 | a dup | 1 | low | open | 2026-10-01\n";
        let ours = base.replace("| a |", "| a2 |");
        let theirs = base.replace(
            "| 1 | low | open | 2026-10-01\n- [ ] T-000001",
            "| 1 | low | review | 2026-10-01\n- [ ] T-000001",
        );
        let r = m(Todo, base, &ours, &theirs);
        assert_eq!(r.matches("T-000001").count(), 2, "{}", r);
        assert!(r.contains("a2") && r.contains("review"));
    }

    #[test]
    fn never_emits_conflict_markers() {
        let ours = TODO_BASE.replace("first task | 2 | high | open", "A | 1 | low | in-progress");
        let theirs = TODO_BASE.replace("first task | 2 | high | open", "B | 9 | critical | review");
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(!r.contains("<<<<<<<") && !r.contains(">>>>>>>") && !r.contains("======="));
        assert!(r.contains("T-000001 | A | 1 | low | review"));
    }

    #[test]
    fn header_renamed_by_theirs_only_is_applied() {
        let ours = TODO_BASE.replace("first task", "first task v2");
        let theirs = TODO_BASE.replace(
            "## v1.20.0 — contributor mode",
            "## v1.20.0 — contributor mode (renamed)",
        );
        let r = m(Todo, TODO_BASE, &ours, &theirs);
        assert!(r.contains("## v1.20.0 — contributor mode (renamed)"));
        assert_eq!(r.matches("## v1.20.0").count(), 1);
        assert!(r.contains("first task v2"));
    }

    #[test]
    fn diverged_row_texts_with_only_absence_in_common_keep_the_whole_file_base() {
        let with = TODO_BASE.to_string();
        let without = with.replace(
            "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
            "",
        );
        let edited = with.replace("second task", "second task edited");
        let v = vec![without.clone(), with.clone(), edited];
        // ours: absent, then the row; theirs: absent, an edited row, deleted
        // again. The row never had a shared text, so absence is no base: the
        // whole-file base (the row present) makes theirs' deletion stand.
        let merged = merge_shared_file_per_entity(
            Todo,
            &with,
            Some(&with),
            &v,
            &[0, 1],
            &[0, 2, 0],
            &with,
            &without,
        )
        .unwrap();
        assert!(!merged.contains("T-000002"), "resurrected:\n{}", merged);
    }

    #[test]
    fn per_entity_base_is_the_last_state_both_histories_held() {
        let with = TODO_BASE.to_string();
        let without = with.replace(
            "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
            "",
        );
        let v = vec![with.clone(), without.clone()];
        // ours: with, without, with, without; theirs is its prefix.
        let deleted = merge_shared_file_per_entity(
            Todo,
            &with,
            Some(&with),
            &v,
            &[0, 1, 0, 1],
            &[0, 1, 0],
            &without,
            &with,
        )
        .unwrap();
        assert!(!deleted.contains("T-000002"));
        // ours: without, with, without, with; theirs is its prefix. The
        // merge-base is theirs' tip — the shared deletion — so ours'
        // identical re-add is newer knowledge and survives (T-000255).
        let readded = merge_shared_file_per_entity(
            Todo,
            &with,
            Some(&without),
            &v,
            &[1, 0, 1, 0],
            &[1, 0, 1],
            &with,
            &without,
        )
        .unwrap();
        assert!(readded.contains("T-000002"));
        // The same shape with the row still present at the merge-base reads
        // as a stale copy kept past a deletion — the deletion stands.
        let stale = merge_shared_file_per_entity(
            Todo,
            &with,
            Some(&with),
            &v,
            &[1, 0, 1, 0],
            &[1, 0, 1],
            &with,
            &without,
        )
        .unwrap();
        assert!(!stale.contains("T-000002"));
        // A re-add with different text is unmistakable — it survives either
        // way.
        let edited = with.replace("second task", "second task redone");
        let v2 = vec![without.clone(), with.clone(), edited.clone()];
        let redone = merge_shared_file_per_entity(
            Todo,
            &with,
            Some(&without),
            &v2,
            &[0, 1, 0, 2],
            &[0, 1, 0],
            &edited,
            &without,
        )
        .unwrap();
        assert!(redone.contains("second task redone"));
    }

    #[test]
    fn oscillating_histories_do_not_resurrect_a_moved_row() {
        let with = TODO_BASE.to_string();
        let without = with.replace(
            "- [ ] T-000002 | second task | 1 | low | open | 2026-10-02\n",
            "",
        );
        let v = vec![without.clone(), with.clone()];
        // LenSA T-500053 (T-000255): ours' worktree kept the stale row while
        // theirs moved it to done.md; both histories had already flickered
        // under the plumbing, so the LCS lands on "absent". The real
        // merge-base (`d24a60f`) still held the row — the deletion wins.
        let merged = merge_shared_file_per_entity(
            Todo,
            &with,
            Some(&with),
            &v,
            &[0, 1, 0, 1, 0, 1],
            &[0, 1, 0, 1, 0],
            &with,
            &without,
        )
        .unwrap();
        assert!(!merged.contains("T-000002"), "resurrected:\n{}", merged);
    }

    // ---- T-000260: per-header provenance + ancestor-history stale check ----

    const ROW: &str = "- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n";

    fn m_a(
        kind: SharedFileKind,
        base: &str,
        ancestry: Option<&str>,
        versions: &[String],
        os: &[usize],
        ts: &[usize],
        ancestors: &[Option<String>],
        ours: &str,
        theirs: &str,
    ) -> String {
        merge_shared_file_with_ancestors(
            kind, base, ancestry, versions, os, ts, ancestors, ours, theirs,
        )
        .unwrap()
    }

    #[test]
    fn stale_theirs_header_ours_edit_survives() {
        // The remote side republishes the OLD header text as a descendant of
        // the merge-base that already holds our rename — the stale-writer
        // dogfood case. Ours' text is the newer knowledge and must survive.
        let old = format!("## v1.20.0 — OLD\n\n{}", ROW);
        let new = format!("## v1.20.0 — NEW\n\n{}", ROW);
        let v = vec![old.clone(), new.clone()];
        // LCS path: our history held OLD then NEW, theirs' holds OLD; the
        // last shared state is OLD = theirs' current -> ours keeps.
        let merged = m_a(Todo, &new, Some(&new), &v, &[0, 1], &[0], &[], &new, &old);
        assert!(merged.contains("## v1.20.0 — NEW"), "{}", merged);
        assert!(!merged.contains("## v1.20.0 — OLD"));
        // Ancestor path alone (no LCS input): p_t (OLD) predates p_b (NEW).
        let merged = m_a(
            Todo,
            &new,
            Some(&new),
            &[],
            &[],
            &[],
            &[Some(old.clone()), Some(new.clone())],
            &new,
            &old,
        );
        assert!(merged.contains("## v1.20.0 — NEW"), "{}", merged);
    }

    #[test]
    fn genuine_theirs_header_rename_still_wins() {
        // Ours did not edit the header; theirs' rename is newer than the
        // shared state and applies.
        let base = format!("## v1.20.0 — A\n\n{}", ROW);
        let theirs = format!("## v1.20.0 — B\n\n{}", ROW);
        let v = vec![base.clone()];
        let merged = m_a(
            Todo,
            &base,
            Some(&base),
            &v,
            &[0],
            &[0],
            &[Some(base.clone())],
            &base,
            &theirs,
        );
        assert!(merged.contains("## v1.20.0 — B"), "{}", merged);
        assert_eq!(merged.matches("## v1.20.0").count(), 1);
    }

    #[test]
    fn both_renamed_header_ours_wins() {
        let base = format!("## v1.20.0 — BASE\n\n{}", ROW);
        let ours = format!("## v1.20.0 — OURS\n\n{}", ROW);
        let theirs = format!("## v1.20.0 — THEIRS\n\n{}", ROW);
        let v = vec![base.clone(), ours.clone(), theirs.clone()];
        // LCS lands on BASE = whole-file base -> no override -> flat rule:
        // base's text != ours' -> no rename, ours keeps.
        let merged = m_a(
            Todo,
            &base,
            Some(&base),
            &v,
            &[0, 1],
            &[0, 2],
            &[Some(base.clone())],
            &ours,
            &theirs,
        );
        assert!(merged.contains("## v1.20.0 — OURS"), "{}", merged);
        assert!(!merged.contains("## v1.20.0 — THEIRS"));
    }

    #[test]
    fn trivial_merge_guard_runs_when_headers_differ() {
        // ours == base byte for byte; theirs changed only the header text to
        // a state the timeline superseded. The `base == ours -> theirs`
        // shortcut must NOT adopt it.
        let old = format!("## v1.20.0 — OLD\n\n{}", ROW);
        let new = format!("## v1.20.0 — NEW\n\n{}", ROW);
        let merged = m_a(
            Todo,
            &new,
            Some(&new),
            &[],
            &[],
            &[],
            &[Some(old.clone()), Some(new.clone())],
            &new,
            &old,
        );
        assert!(merged.contains("## v1.20.0 — NEW"), "{}", merged);
        // A header-only diff on a NON-stale rename still fast-paths to
        // theirs verbatim (the occurrences match per key).
        let renamed = format!("## v1.20.0 — RENAMED\n\n{}", ROW);
        let merged = m_a(Todo, &new, Some(&new), &[], &[], &[], &[], &new, &renamed);
        assert_eq!(merged, renamed);
    }

    #[test]
    fn duplicate_same_key_headers_first_occurrence_only() {
        // Ours carries the key twice; theirs renamed BOTH occurrences. The
        // first decides (theirs' rename wins as it is a genuine edit), the
        // second stays verbatim ours.
        let base = format!("## v1 — A\n\n{}## v1 — dupA\n\n", ROW);
        let ours = base.clone();
        let theirs = format!("## v1 — B\n\n{}## v1 — dupB\n\n", ROW);
        let merged = m_a(Todo, &base, Some(&base), &[], &[], &[], &[], &ours, &theirs);
        let l = lines(&merged);
        let mut hdrs: Vec<&str> = l
            .iter()
            .filter(|x| x.starts_with("## v1"))
            .copied()
            .collect();
        assert_eq!(hdrs, vec!["## v1 — B", "## v1 — dupA"], "{:?}", hdrs);
        hdrs.clear();
        // And the trivial `base == ours -> theirs` gate: differing
        // occurrences keep the full path, which yields the same policy
        // result instead of verbatim theirs.
        let theirs2 = format!("## v1 — A\n\n{}## v1 — dupB\n\n", ROW);
        let merged = m_a(
            Todo,
            &base,
            Some(&base),
            &[],
            &[],
            &[],
            &[],
            &ours,
            &theirs2,
        );
        assert_ne!(merged, theirs2, "occurrence mismatch must not fast-path");
        assert!(merged.contains("## v1 — dupA"));
    }

    #[test]
    fn no_history_header_fallback_is_the_flat_rule() {
        // No versions, no ancestors: the whole-file base governs exactly as
        // before — theirs' rename of an ours-untouched header wins.
        let base = format!("## v1.20.0 — A\n\n{}", ROW);
        let theirs = format!("## v1.20.0 — B\n\n{}", ROW);
        let merged = m_a(Todo, &base, Some(&base), &[], &[], &[], &[], &base, &theirs);
        assert!(merged.contains("## v1.20.0 — B"));
    }

    #[test]
    fn stale_entity_row_kept_and_legit_remote_edit_passes() {
        // The remote tip holds a row state the shared timeline superseded
        // before the merge-base: stale -> ours' (== base's) state survives.
        let old_row = format!("## v1\n\n- [ ] T-000001 | old text | 1 | low | open | 2026-10-01\n");
        let new_row = format!("## v1\n\n- [ ] T-000001 | new text | 1 | low | open | 2026-10-01\n");
        let ancestors = vec![Some(old_row.clone()), Some(new_row.clone())];
        assert!(has_stale_remote_state(Todo, &new_row, &old_row, &ancestors));
        let merged = m_a(
            Todo,
            &new_row,
            Some(&new_row),
            &[],
            &[],
            &[],
            &ancestors,
            &new_row,
            &old_row,
        );
        assert!(merged.contains("new text"), "{}", merged);
        assert!(!merged.contains("old text"));
        // A genuinely new remote state (never on the timeline) is not stale.
        let fresh = format!("## v1\n\n- [ ] T-000001 | fresh | 1 | low | open | 2026-10-01\n");
        assert!(!has_stale_remote_state(Todo, &new_row, &fresh, &ancestors));
        // A remote re-add of a row absent at the merge-base is not stale
        // either (T-000255), even when its text matches a pre-deletion one.
        let absent_base = "## v1\n\n";
        assert!(!has_stale_remote_state(
            Todo,
            absent_base,
            &old_row,
            &ancestors
        ));
    }

    #[test]
    fn stale_check_needs_proof_not_unseen_history() {
        // base's state is not inside the ancestor bound -> not stale.
        let old_row = format!("## v1\n\n- [ ] T-000001 | old text | 1 | low | open | 2026-10-01\n");
        let new_row = format!("## v1\n\n- [ ] T-000001 | new text | 1 | low | open | 2026-10-01\n");
        let unrelated = format!("## v1\n\n- [ ] T-000009 | other | 1 | low | open | 2026-10-01\n");
        assert!(!has_stale_remote_state(
            Todo,
            &new_row,
            &old_row,
            &[Some(unrelated)]
        ));
        // Empty ancestors never flag.
        assert!(!has_stale_remote_state(Todo, &new_row, &old_row, &[]));
    }
}

#[cfg(test)]
mod clocked_tests {
    use super::*;
    use crate::sync::edit_clocks::{
        field_key, header_key, row_key, value_hash, ClockEntry, ClockLedger, STRUCTURE_KEY,
    };

    fn ent(h_val: &str, t: i64, src: &str, del: bool) -> ClockEntry {
        ClockEntry {
            h: value_hash(h_val.as_bytes()),
            t,
            src: src.to_string(),
            del,
        }
    }

    fn ledger_of(pairs: &[(&str, ClockEntry)]) -> ClockLedger {
        let mut l = ClockLedger::new("docs/todo.md");
        for (k, e) in pairs {
            l.entries.insert(k.to_string(), e.clone());
        }
        l
    }

    fn merge6(base: &str, ours: &str, theirs: &str, ledger: &ClockLedger) -> (String, Vec<String>) {
        merge_shared_file_clocked(
            SharedFileKind::Todo,
            base,
            None,
            &[],
            &[],
            &[],
            &[],
            ours,
            theirs,
            ledger,
        )
        .unwrap()
    }

    const BASE: &str = "## v1

- [ ] T-000001 | task | 1 | low | open | 2026-10-01
";

    #[test]
    fn clocked_field_resolution_later_utc_wins_both_orders() {
        // Both sides moved the status off base "open": ours to in-progress,
        // theirs to review. The stamp on "review" is later — review must win
        // whichever side the merge runs on (positional rules would have kept
        // the merging machine's own value or the higher rank).
        let ours = "## v1

- [ ] T-000001 | task | 1 | low | in-progress | 2026-10-01
";
        let theirs = "## v1

- [ ] T-000001 | task | 1 | low | review | 2026-10-01
";
        let fwd = ledger_of(&[(&field_key("T-000001", 4), ent("review", 200, "s2", false))]);
        let (text, diags) = merge6(BASE, ours, theirs, &fwd);
        assert!(text.contains("| review |"), "{}", text);
        assert!(diags.is_empty(), "{:?}", diags);
    }

    #[test]
    fn clocked_status_moves_backward_when_later() {
        // Monotonic ranks are abolished: a reopen (review -> open) with the
        // later stamp wins even though its rank is LOWER.
        let base = "## v1

- [ ] T-000001 | task | 1 | low | in-progress | 2026-10-01
";
        let ours = "## v1

- [ ] T-000001 | task | 1 | low | review | 2026-10-01
";
        let theirs = "## v1

- [ ] T-000001 | task | 1 | low | open | 2026-10-01
";
        let l = ledger_of(&[(&field_key("T-000001", 4), ent("open", 300, "s3", false))]);
        let (text, _) = merge6(base, ours, theirs, &l);
        assert!(text.contains("| open |"), "{}", text);
    }

    #[test]
    fn clocked_existence_tombstone_beats_stale_row_and_loses_to_reopen() {
        let deleted_side = "## v1

";
        let live = "- [ ] T-000001 | task | 1 | low | in-progress | 2026-10-01";
        let ours_with_row = format!(
            "## v1

{}
",
            live
        );
        // Case 1: the tombstone is the ledger's latest word — the row stays
        // deleted even though one side still carries it (changed vs base!).
        let tomb = ledger_of(&[(&row_key("T-000001"), {
            let mut e = ent("", 400, "sdel", true);
            e.h = String::new();
            e
        })]);
        let (text, _) = merge6(BASE, &ours_with_row, deleted_side, &tomb);
        assert!(!text.contains("T-000001"), "{}", text);
        // Case 2: the reopen edit is later — the ledger entry is live and
        // matches the surviving line, so the row is kept.
        let reopen = ledger_of(&[(&row_key("T-000001"), ent(live, 500, "sre", false))]);
        let (text2, _) = merge6(BASE, &ours_with_row, deleted_side, &reopen);
        assert!(text2.contains("T-000001"), "{}", text2);
    }

    #[test]
    fn clocked_no_stamps_is_deterministic_and_diagnosed() {
        let ours = "## v1

- [ ] T-000001 | alpha | 1 | low | open | 2026-10-01
";
        let theirs = "## v1

- [ ] T-000001 | beta | 1 | low | open | 2026-10-01
";
        let empty = ledger_of(&[]);
        let (t1, d1) = merge6(BASE, ours, theirs, &empty);
        let (t2, d2) = merge6(BASE, theirs, ours, &empty);
        // Degraded mode: same result on both machines (commutativity holds
        // even without stamps) plus a loud diagnostic — never silent.
        assert_eq!(t1, t2);
        assert!(t1.contains("| beta |"), "{}", t1);
        assert!(!d1.is_empty());
        assert!(!d2.is_empty());
    }

    #[test]
    fn clocked_structure_stamp_moves_the_skeleton() {
        // Both sides changed the structure off base: ours reordered v2 before
        // v1, theirs appended a new section v3 at the end. The structure stamp
        // binds to THEIRS' fingerprint (their structural edit is the later
        // decision) — the merged skeleton must follow theirs' ordering even
        // though the assembly is ours-relative without clocks.
        let base = "## v1

- [ ] T-000001 | a | 1 | low | open | 2026-10-01

## v2

- [ ] T-000002 | b | 1 | low | open | 2026-10-01
";
        let reordered = "## v2

- [ ] T-000002 | b | 1 | low | open | 2026-10-01

## v1

- [ ] T-000001 | a | 1 | low | open | 2026-10-01
";
        let appended = "## v1

- [ ] T-000001 | a | 1 | low | open | 2026-10-01

## v2

- [ ] T-000002 | b | 1 | low | open | 2026-10-01

## v3

- [ ] T-000003 | c | 1 | low | open | 2026-10-01
";
        let fp_theirs = {
            let d = parse(SharedFileKind::Todo, appended);
            skeleton_fingerprint(&d)
        };
        let l = ledger_of(&[(
            STRUCTURE_KEY,
            ClockEntry {
                h: fp_theirs,
                t: 500,
                src: "s".to_string(),
                del: false,
            },
        )]);
        let (text, _) = merge6(base, reordered, appended, &l);
        let v1 = text.find("## v1").unwrap();
        let v2 = text.find("## v2").unwrap();
        let v3 = match text.find("## v3") {
            Some(p) => p,
            None => panic!(
                "v3 section missing:
{}",
                text
            ),
        };
        assert!(v1 < v2 && v2 < v3, "{}", text);
        // And commutativity: the same merge with the sides swapped yields the
        // same ordering — the stamp decides, not the machine position.
        let (text2, _) = merge6(base, appended, reordered, &l);
        assert_eq!(text, text2);
    }

    #[test]
    fn clocked_header_rename_later_stamp_wins() {
        // Both renamed the same section header differently off base; the
        // stamp on theirs' text is later — theirs wins regardless of position.
        let base = "## v1 old

- [ ] T-000001 | a | 1 | low | open | 2026-10-01
";
        // Anti-degraded ordering: ours' text is lexicographically LARGER, so
        // the content-order fallback would keep ours — a pass here can only
        // come from the stamp truly deciding. Header text is the full raw
        // line, `## ` included — the hash must bind to exactly that.
        let ours = "## v1 zeta

- [ ] T-000001 | a | 1 | low | open | 2026-10-01
";
        let theirs = "## v1 beta

- [ ] T-000001 | a | 1 | low | open | 2026-10-01
";
        // Section keys derive from the header text; the rename target key is
        // the base key. The stamp binds to theirs' header TEXT.
        let key = {
            let d = parse(SharedFileKind::Todo, base);
            match &d.lines[0] {
                Line::Header { key, .. } => key.clone(),
                _ => panic!("no header"),
            }
        };
        let l = ledger_of(&[(&header_key(&key), ent("## v1 beta", 600, "s", false))]);
        let (text, _) = merge6(base, ours, theirs, &l);
        assert!(text.contains("## v1 beta"), "{}", text);
        assert!(!text.contains("## v1 zeta"), "{}", text);
    }

    fn far_future() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 48 * 3600
    }

    #[test]
    fn quarantined_field_stamp_decides_nothing_and_reports() {
        // Ours moved the status to zulu, theirs to alpha; the ledger stamps
        // "alpha" in the FAR FUTURE — a wrong-clock machine. Without
        // quarantine consumption that poisoned stamp wins every merge
        // forever. Frozen, it decides nothing: the degraded deterministic
        // fallback keeps ours ("zulu" > "alpha" in content order would pick
        // zulu whichever machine merges), and the freeze is reported.
        let ours = "## v1

- [ ] T-000001 | task | 1 | low | zulu | 2026-10-01
";
        let theirs = "## v1

- [ ] T-000001 | task | 1 | low | alpha | 2026-10-01
";
        let l = ledger_of(&[(
            &field_key("T-000001", 4),
            ent("alpha", far_future(), "bad", false),
        )]);
        let (text, diags) = merge6(BASE, ours, theirs, &l);
        assert!(text.contains("| zulu |"), "{}", text);
        assert!(!text.contains("| alpha |"), "{}", text);
        assert!(
            diags.iter().any(|d| d.contains("far-future")),
            "the freeze must be reported: {:?}",
            diags
        );
    }

    #[test]
    fn quarantined_tombstone_cannot_delete_a_honestly_edited_row() {
        // Ours edited the row (honest stamp t=100), theirs deleted it and a
        // wrong-clock machine stamped the tombstone in the far future.
        // Unconsumed, the poisoned tombstone deletes the row on every merge.
        // Frozen, the honest edit stamp stands alone and the row survives.
        let live = "- [ ] T-000001 | task | 1 | low | in-progress | 2026-10-01";
        let ours_with_row = format!(
            "## v1

{}
",
            live
        );
        let deleted_side = "## v1

";
        let tomb = {
            let mut e = ent("", far_future(), "bad", true);
            e.h = String::new();
            e
        };
        // One ledger key holds one winner — model the union result directly:
        // the tombstone is the key's entry (later stamp won the CRDT merge),
        // so the freeze must come from quarantine, not from losing the union.
        let l = ledger_of(&[(&row_key("T-000001"), tomb)]);
        let (text, _) = merge6(BASE, &ours_with_row, deleted_side, &l);
        // With the tombstone frozen there is no deletion evidence at all;
        // the row changed vs base, so the conservative survivor rule keeps it.
        assert!(text.contains("T-000001"), "{}", text);
        // And sanity: a NON-quarantined tombstone still deletes it.
        let honest_tomb = {
            let mut e = ent("", 400, "sdel", true);
            e.h = String::new();
            e
        };
        let l2 = ledger_of(&[(&row_key("T-000001"), honest_tomb)]);
        let (text2, _) = merge6(BASE, &ours_with_row, deleted_side, &l2);
        assert!(!text2.contains("T-000001"), "{}", text2);
    }
}
