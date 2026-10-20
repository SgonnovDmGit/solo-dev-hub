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
struct Ent {
    key: String,
    /// Line prefix after the indentation: `- `, `- [ ] `, `- [x] `.
    marker: String,
    /// Raw (escape-preserving, trimmed) pipe-separated fields.
    fields: Vec<String>,
    /// Key of the nearest preceding `## ` header, `""` before the first one.
    section: String,
    /// The original line text without the line ending.
    raw: String,
}

enum Line {
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

struct Doc {
    lines: Vec<Line>,
    ents: Vec<Ent>,
    by_key: HashMap<String, usize>,
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

fn parse(kind: SharedFileKind, input: &str) -> Doc {
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
fn pick(role: Role, b: Option<&str>, o: &str, t: &str) -> String {
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
    match role {
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
    }
}

struct Merged {
    section: String,
    line: String,
}

fn same(a: &Ent, b: &Ent) -> bool {
    a.marker == b.marker && a.fields == b.fields && a.section == b.section
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

fn merge_both(kind: SharedFileKind, b: Option<&Ent>, o: &Ent, t: &Ent) -> Merged {
    // Fields of different arity (legacy lines): no positional merge possible;
    // the side that changed wins, ours on a tie.
    if o.fields.len() != t.fields.len() {
        let pick_theirs = b.map_or(false, |b| same(b, o));
        let w = if pick_theirs { t } else { o };
        return Merged {
            section: w.section.clone(),
            line: w.raw.clone(),
        };
    }
    let n = o.fields.len();
    let bf = b.filter(|b| b.fields.len() == n);
    let marker = pick(
        Role::Other,
        b.map(|b| b.marker.as_str()),
        &o.marker,
        &t.marker,
    );
    let section = pick(
        Role::Other,
        b.map(|b| b.section.as_str()),
        &o.section,
        &t.section,
    );
    let fields: Vec<String> = (0..n)
        .map(|i| {
            pick(
                role_at(kind, n, i),
                bf.map(|b| b.fields[i].as_str()),
                &o.fields[i],
                &t.fields[i],
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
) -> Option<Merged> {
    match (o, t) {
        (Some(o), Some(t)) => Some(merge_both(kind, b, o, t)),
        (Some(x), None) | (None, Some(x)) => {
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
    merge_with_bases(kind, base, ours, theirs, &HashMap::new())
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
/// entity the histories never both held keeps the whole-file base.
pub fn merge_shared_file_per_entity(
    kind: SharedFileKind,
    base: &str,
    versions: &[String],
    ours_seq: &[usize],
    theirs_seq: &[usize],
    ours: &str,
    theirs: &str,
) -> Result<String, MergeError> {
    let overrides = entity_bases(kind, base, versions, ours_seq, theirs_seq, ours, theirs);
    merge_with_bases(kind, base, ours, theirs, &overrides)
}

/// Do two states of one row (`None` = absent) agree?
fn same_state(a: Option<&Ent>, b: Option<&Ent>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => same(x, y),
        _ => false,
    }
}

/// The state sequence of one row over a history, consecutive repeats dropped.
fn state_seq<'a>(
    seq: &[usize],
    states: &'a [HashMap<String, Ent>],
    key: &str,
) -> Vec<Option<&'a Ent>> {
    let mut out: Vec<Option<&Ent>> = Vec::new();
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
fn lcs_last<'a>(a: &[Option<&'a Ent>], b: &[Option<&'a Ent>]) -> Option<Option<&'a Ent>> {
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
    versions: &[String],
    ours_seq: &[usize],
    theirs_seq: &[usize],
    ours: &str,
    theirs: &str,
) -> HashMap<String, Option<Ent>> {
    let mut overrides = HashMap::new();
    let (b, o, t) = (parse(kind, base), parse(kind, ours), parse(kind, theirs));
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
            let whole = b.by_key.get(key).map(|&i| &b.ents[i]);
            if !same_state(found, whole) {
                overrides.insert(key.clone(), found.cloned());
            }
        }
    }
    overrides
}

fn merge_with_bases(
    kind: SharedFileKind,
    base: &str,
    ours: &str,
    theirs: &str,
    overrides: &HashMap<String, Option<Ent>>,
) -> Result<String, MergeError> {
    // Trivial merges: exactly what git itself would do. The base ones stand
    // for the whole file only while no row has a base of its own.
    if ours == theirs {
        return Ok(ours.to_string());
    }
    if overrides.is_empty() {
        if base == ours {
            return Ok(theirs.to_string());
        }
        if base == theirs {
            return Ok(ours.to_string());
        }
    }

    let b = parse(kind, base);
    let o = parse(kind, ours);
    let t = parse(kind, theirs);

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
        merged.insert(e.key.clone(), decide(kind, eb, eo, et));
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
    for (i, l) in t.lines.iter().enumerate() {
        match l {
            Line::Header { key, text } => {
                if let Some(otext) = o_hdr.get(key) {
                    // Only theirs renamed the header -> take the new text.
                    if b_hdr.get(key) == Some(otext) && text != otext {
                        if let Some(p) = find_header(&out, key) {
                            out[p].text = text.clone();
                        }
                    }
                } else if !b_hdr.contains_key(key) && find_header(&out, key).is_none() {
                    let anchor = anchor_for(&t, i, &out);
                    insert_header(&mut out, anchor, key, text);
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
            &v,
            &[0, 1, 0, 1],
            &[0, 1, 0],
            &without,
            &with,
        )
        .unwrap();
        assert!(!deleted.contains("T-000002"));
        // ours: without, with, without, with; theirs is its prefix.
        let readded = merge_shared_file_per_entity(
            Todo,
            &with,
            &v,
            &[1, 0, 1, 0],
            &[1, 0, 1],
            &with,
            &without,
        )
        .unwrap();
        assert!(readded.contains("T-000002"));
    }
}
