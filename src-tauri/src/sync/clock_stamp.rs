// T-000274 / UTC-chronology sync: the write-side stamping pass.
//
// Given the parent version of a shared file on THIS branch and the version
// the new commit carries, diff them key by key and stamp exactly what
// changed. This is the mechanism behind the spec's core rule: only the
// commit where a value changed relative to its branch parent stamps it —
// transport copies and merge/import results produce an empty diff here and
// therefore never restamp (they inherit the source stamp through the
// ledger's own union merge).
//
// Everything is per key through `ClockLedger::stamp`, which applies the LWW
// rule internally: a stale pass can never overwrite a newer decision.

use crate::sync::edit_clocks::{
    field_key, header_key, prose_key, row_key, value_hash, ClockEntry, ClockLedger, STRUCTURE_KEY,
};
use crate::sync::merge_by_id::{parse, skeleton_fingerprint, Doc, Line, SharedFileKind};

fn entry(h: String, t: i64, src: &str, del: bool) -> ClockEntry {
    ClockEntry {
        h,
        t,
        src: src.to_string(),
        del,
    }
}

/// Stamp everything that differs between `old` and `new` of one shared file.
/// Returns whether any key changed. `now` is the commit's UTC seconds, `src`
/// the commit sha (or the authoring marker pre-commit).
pub fn stamp_file_delta(
    kind: SharedFileKind,
    old: &str,
    new: &str,
    ledger: &mut ClockLedger,
    now: i64,
    src: &str,
) -> bool {
    let o = parse(kind, old);
    let n = parse(kind, new);
    let mut changed = false;

    // ── Entities ─────────────────────────────────────────────────────────
    let mut keys: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for e in o.ents.iter().chain(n.ents.iter()) {
        keys.insert(e.key.as_str());
    }
    for key in keys {
        let oe = o.by_key.get(key).map(|&i| &o.ents[i]);
        let ne = n.by_key.get(key).map(|&i| &n.ents[i]);
        let rk = row_key(key);
        match (oe, ne) {
            (Some(_), None) => {
                // Deleted on this branch — the tombstone IS the decision.
                changed |= ledger.stamp(&rk, entry(String::new(), now, src, true));
            }
            (None, Some(x)) => {
                // Appeared: row-level stamp plus every field.
                changed |= ledger.stamp(&rk, entry(value_hash(x.raw.as_bytes()), now, src, false));
                for (i, f) in x.fields.iter().enumerate() {
                    changed |= ledger.stamp(
                        &field_key(key, i),
                        entry(value_hash(f.as_bytes()), now, src, false),
                    );
                }
            }
            (Some(a), Some(x)) => {
                if a.raw != x.raw {
                    changed |=
                        ledger.stamp(&rk, entry(value_hash(x.raw.as_bytes()), now, src, false));
                    if a.fields.len() == x.fields.len() {
                        for (i, (fa, fx)) in a.fields.iter().zip(x.fields.iter()).enumerate() {
                            if fa != fx {
                                changed |= ledger.stamp(
                                    &field_key(key, i),
                                    entry(value_hash(fx.as_bytes()), now, src, false),
                                );
                            }
                        }
                    }
                    // Different arity: row-level stamp alone carries the
                    // decision (spec D2 legacy fallback), no field keys.
                } else if a.section != x.section {
                    // A pure section move leaves the raw line untouched, so
                    // the content diff above cannot see it — but the move IS
                    // a decision about the row. Re-stamp the row key with the
                    // same hash and the newer time: the stamp stays valid for
                    // the value and the section resolution gets its time axis.
                    changed |=
                        ledger.stamp(&rk, entry(value_hash(x.raw.as_bytes()), now, src, false));
                }
            }
            (None, None) => {}
        }
    }

    // ── Section headers ──────────────────────────────────────────────────
    let mut hkeys: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for l in o.lines.iter().chain(n.lines.iter()) {
        if let Line::Header { key, .. } = l {
            hkeys.insert(key.as_str());
        }
    }
    for key in hkeys {
        let ot = header_text(&o, key);
        let nt = header_text(&n, key);
        let hk = header_key(key);
        match (ot, nt) {
            (Some(_), None) => {
                changed |= ledger.stamp(&hk, entry(String::new(), now, src, true));
            }
            (None, Some(t)) => {
                changed |= ledger.stamp(&hk, entry(value_hash(t.as_bytes()), now, src, false));
            }
            (Some(a), Some(x)) if a != x => {
                changed |= ledger.stamp(&hk, entry(value_hash(x.as_bytes()), now, src, false));
            }
            _ => {}
        }
    }

    // ── Prose blocks (content-hash keyed) ────────────────────────────────
    let prose_of = |d: &Doc| -> std::collections::HashSet<String> {
        d.lines
            .iter()
            .filter_map(|l| match l {
                Line::Other(s) | Line::Dup(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => None,
            })
            .collect()
    };
    let po = prose_of(&o);
    let pn = prose_of(&n);
    for gone in po.difference(&pn) {
        let h = value_hash(gone.as_bytes());
        changed |= ledger.stamp(&prose_key(&h), entry(String::new(), now, src, true));
    }
    for added in pn.difference(&po) {
        let h = value_hash(added.as_bytes());
        changed |= ledger.stamp(&prose_key(&h), entry(h.clone(), now, src, false));
    }

    // ── Structure ────────────────────────────────────────────────────────
    let fo = skeleton_fingerprint(&o);
    let fnw = skeleton_fingerprint(&n);
    if fo != fnw {
        changed |= ledger.stamp(STRUCTURE_KEY, entry(fnw, now, src, false));
    }

    changed
}

/// Baseline stamping for the activation commit (T-000266, spec D1a): every
/// live key of `content` that lacks a VALID stamp gets one at the baseline
/// time — but an honest earlier stamp of the same value is never overwritten
/// (the whole point of stamping continuously since the first v1.25.0 pass).
/// "Valid" means same key AND same value hash: a stale entry of a superseded
/// value does not protect the key — the current value is what gets stamped.
/// Tombstones are untouched: nothing here revives or rewrites a deletion.
pub fn stamp_file_baseline(
    kind: SharedFileKind,
    content: &str,
    ledger: &mut ClockLedger,
    now: i64,
    src: &str,
) -> bool {
    let mut fresh = ClockLedger::new(&ledger.file);
    stamp_file_delta(kind, "", content, &mut fresh, now, src);
    let mut changed = false;
    for (k, e) in fresh.entries {
        let honest = ledger.entries.get(&k).is_some_and(|x| x.h == e.h && !x.del);
        if !honest {
            changed |= ledger.stamp(&k, e);
        }
    }
    changed
}

/// T-000273: stamp one whole-file artifact's change. `old`/`new` are the
/// parent-side and the committing content; `None` means absent on that side.
/// Add/edit stamps the content hash, deletion stamps the tombstone — the same
/// LWW grammar as the entity keys, one artifact one key. Transport copies
/// produce no worktree diff and therefore never restamp, exactly like the
/// entity-level writer.
pub fn stamp_whole_file_change(
    old: Option<&[u8]>,
    new: Option<&[u8]>,
    ledger: &mut ClockLedger,
    now: i64,
    src: &str,
) -> bool {
    use crate::sync::edit_clocks::FILE_KEY;
    match (old, new) {
        (None, None) => false,
        (Some(_), None) => ledger.stamp(FILE_KEY, entry(String::new(), now, src, true)),
        (o, Some(n)) if o != Some(n) => {
            // The content is the whole value here — there is no position or
            // section dimension like the entity keys have. Re-stamping the
            // same hash is therefore the SAME event, not a new decision:
            // keep the original (earlier, more honest) time. This is what
            // makes repeated passes over a not-yet-committed delta inert.
            // A live winner with a different hash, or a tombstone winner,
            // never suppresses: a re-add after a deletion must stamp.
            let h = value_hash(n);
            if ledger
                .get(FILE_KEY)
                .is_some_and(|e| !e.del && e.h == h)
            {
                return false;
            }
            ledger.stamp(FILE_KEY, entry(h, now, src, false))
        }
        _ => false,
    }
}

fn header_text<'a>(d: &'a Doc, key: &str) -> Option<&'a str> {
    d.lines.iter().find_map(|l| match l {
        Line::Header { key: k, text } if k == key => Some(text.as_str()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> ClockLedger {
        ClockLedger::new("docs/todo.md")
    }

    const V1: &str = "## v1\n\n- [ ] T-000001 | task | 1 | low | open | 2026-10-01\n";

    #[test]
    fn baseline_stamps_all_live_content_once() {
        let mut l = ledger();
        assert!(stamp_file_baseline(
            SharedFileKind::Todo,
            V1,
            &mut l,
            1000,
            "base"
        ));
        assert!(l.get(&row_key("T-000001")).is_some());
        assert!(l.get(&crate::sync::edit_clocks::STRUCTURE_KEY).is_some());
        // A second baseline is inert — same values, already honest.
        assert!(!stamp_file_baseline(
            SharedFileKind::Todo,
            V1,
            &mut l,
            2000,
            "base2"
        ));
    }

    #[test]
    fn baseline_never_overwrites_an_honest_earlier_stamp() {
        let mut l = ledger();
        // The honest stamp of the same value at t=100.
        stamp_file_delta(SharedFileKind::Todo, "", V1, &mut l, 100, "honest");
        let before: Vec<(String, i64)> = l.entries.iter().map(|(k, e)| (k.clone(), e.t)).collect();
        assert!(stamp_file_baseline(SharedFileKind::Todo, V1, &mut l, 9000, "base") == false);
        let after: Vec<(String, i64)> = l.entries.iter().map(|(k, e)| (k.clone(), e.t)).collect();
        assert_eq!(before, after, "honest times survive the baseline");
    }

    #[test]
    fn baseline_replaces_a_stamp_of_a_superseded_value() {
        let mut l = ledger();
        // v-old stamped the row's old text; the current content differs.
        let old = "## v1

- [ ] T-000001 | old text | 1 | low | open | 2026-10-01
";
        stamp_file_delta(SharedFileKind::Todo, "", old, &mut l, 100, "honest");
        assert!(stamp_file_baseline(
            SharedFileKind::Todo,
            V1,
            &mut l,
            500,
            "base"
        ));
        use crate::sync::edit_clocks::value_hash;
        let row = l.get(&row_key("T-000001")).unwrap();
        assert_eq!(row.t, 500, "the current value gets the baseline time");
        // The row hash is the CURRENT row's hash now.
        let cur = parse(SharedFileKind::Todo, V1);
        let ent = &cur.ents[0];
        assert_eq!(row.h, value_hash(ent.raw.as_bytes()));
    }

    #[test]
    fn no_diff_stamps_nothing() {
        let mut l = ledger();
        assert!(!stamp_file_delta(
            SharedFileKind::Todo,
            V1,
            V1,
            &mut l,
            100,
            "c1"
        ));
        assert!(l.entries.is_empty());
    }

    #[test]
    fn field_edit_stamps_row_and_field_not_other_fields() {
        let new = "## v1\n\n- [ ] T-000001 | task | 1 | low | review | 2026-10-01\n";
        let mut l = ledger();
        assert!(stamp_file_delta(
            SharedFileKind::Todo,
            V1,
            new,
            &mut l,
            100,
            "c1"
        ));
        assert!(l.get(&row_key("T-000001")).is_some());
        assert!(l.get(&field_key("T-000001", 4)).is_some());
        assert!(
            l.get(&field_key("T-000001", 1)).is_none(),
            "untouched fields must not restamp"
        );
        assert!(!l.get(&row_key("T-000001")).unwrap().del);
    }

    #[test]
    fn deletion_stamps_tombstone_and_readd_beats_it() {
        let mut l = ledger();
        assert!(stamp_file_delta(
            SharedFileKind::Todo,
            V1,
            "## v1\n\n",
            &mut l,
            100,
            "c1"
        ));
        let rk = row_key("T-000001");
        assert!(l.get(&rk).unwrap().del);
        // The re-add later: live stamp wins over the tombstone.
        assert!(stamp_file_delta(
            SharedFileKind::Todo,
            "## v1\n\n",
            V1,
            &mut l,
            200,
            "c2"
        ));
        let e = l.get(&rk).unwrap();
        assert!(!e.del);
        assert_eq!(e.t, 200);
        // And a stale deletion pass cannot overwrite the re-add (LWW inside).
        let mut l2 = l.clone();
        stamp_file_delta(SharedFileKind::Todo, V1, "## v1\n\n", &mut l2, 150, "c3");
        // t=150 < 200 — nothing changes even though the pass ran
        assert!(!l2.get(&rk).unwrap().del);
    }

    #[test]
    fn header_rename_restamps_text_and_key_change_tombstones() {
        // Version-ish headers key by their first token (`v1`), so a rename
        // behind the same token is a TEXT change of the same key — one live
        // stamp with the new hash, no tombstone.
        let new = "## v1 renamed

- [ ] T-000001 | task | 1 | low | open | 2026-10-01
";
        let mut l = ledger();
        stamp_file_delta(SharedFileKind::Todo, V1, new, &mut l, 100, "c1");
        let hkeys: Vec<String> = l
            .entries
            .keys()
            .filter(|k| k.starts_with("h:"))
            .cloned()
            .collect();
        assert_eq!(hkeys, vec!["h:v1".to_string()]);
        let e = l.get("h:v1").unwrap();
        assert!(!e.del);
        // Header `text` is the FULL raw line including the `## ` prefix.
        assert_eq!(e.h, value_hash("## v1 renamed".as_bytes()));
        // A key change (the whole section renamed away) tombstones the old
        // key and stamps the new one — the spec's rename-is-a-new-key rule.
        let other = "## backlog

- [ ] T-000001 | task | 1 | low | open | 2026-10-01
";
        let mut l2 = ledger();
        stamp_file_delta(SharedFileKind::Todo, V1, other, &mut l2, 100, "c1");
        let live: Vec<&String> = l2
            .entries
            .keys()
            .filter(|k| k.starts_with("h:") && !l2.get(k).unwrap().del)
            .collect();
        let tomb: Vec<&String> = l2
            .entries
            .keys()
            .filter(|k| k.starts_with("h:") && l2.get(k).unwrap().del)
            .collect();
        assert_eq!(live.len(), 1);
        assert_eq!(tomb.len(), 1);
        assert_eq!(tomb[0].as_str(), "h:v1");
    }

    #[test]
    fn reorder_stamps_structure_only() {
        let base = "## v1\n\n- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n\n## v2\n\n- [ ] T-000002 | b | 1 | low | open | 2026-10-01\n";
        let reordered = "## v2\n\n- [ ] T-000002 | b | 1 | low | open | 2026-10-01\n\n## v1\n\n- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n";
        let mut l = ledger();
        stamp_file_delta(SharedFileKind::Todo, base, reordered, &mut l, 100, "c1");
        assert!(l.get(STRUCTURE_KEY).is_some());
        // No row content changed — there must be no row keys beyond the
        // section-move restamps... rows did NOT move sections here (they
        // moved WITH their sections), so no row keys at all.
        assert!(
            !l.entries.keys().any(|k| k.starts_with("r:")),
            "rows unchanged in content and section must not stamp: {:?}",
            l.entries.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn section_move_restamps_row_with_same_hash() {
        let base = "## v1\n\n- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n\n## v2\n\n";
        let moved = "## v1\n\n\n## v2\n\n- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n";
        let mut l = ledger();
        stamp_file_delta(SharedFileKind::Todo, base, moved, &mut l, 100, "c1");
        let e = l
            .get(&row_key("T-000001"))
            .expect("move must restamp the row");
        assert_eq!(
            e.h,
            value_hash("- [ ] T-000001 | a | 1 | low | open | 2026-10-01".as_bytes())
        );
        assert!(!e.del);
    }

    #[test]
    fn prose_add_and_remove_stamp_by_content_hash() {
        let base = "## v1\n\n- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n";
        let with_prose =
            "## v1\n\na note in prose\n\n- [ ] T-000001 | a | 1 | low | open | 2026-10-01\n";
        let mut l = ledger();
        stamp_file_delta(SharedFileKind::Todo, base, with_prose, &mut l, 100, "c1");
        let live_prose: Vec<String> = l
            .entries
            .keys()
            .filter(|k| k.starts_with("p:"))
            .cloned()
            .collect();
        assert_eq!(live_prose.len(), 1);
        assert!(!l.get(&live_prose[0]).unwrap().del);
        // removing it tombstones the same key
        stamp_file_delta(SharedFileKind::Todo, with_prose, base, &mut l, 200, "c2");
        assert!(l.get(&live_prose[0]).unwrap().del);
    }
    #[test]
    fn whole_file_add_edit_delete_cycle() {
        use crate::sync::edit_clocks::FILE_KEY;
        let mut l = ledger();
        // add
        assert!(stamp_whole_file_change(None, Some(b"v1"), &mut l, 100, "c1"));
        let e = l.get(FILE_KEY).unwrap();
        assert!(!e.del);
        assert_eq!(e.h, value_hash(b"v1"));
        assert_eq!(e.t, 100);
        // byte-identical transport copy: no diff, no restamp
        assert!(!stamp_whole_file_change(Some(b"v1"), Some(b"v1"), &mut l, 150, "cX"));
        assert_eq!(l.get(FILE_KEY).unwrap().t, 100);
        // edit
        assert!(stamp_whole_file_change(Some(b"v1"), Some(b"v2"), &mut l, 200, "c2"));
        assert_eq!(l.get(FILE_KEY).unwrap().h, value_hash(b"v2"));
        assert_eq!(l.get(FILE_KEY).unwrap().t, 200);
        // delete: tombstone
        assert!(stamp_whole_file_change(Some(b"v2"), None, &mut l, 300, "c3"));
        assert!(l.get(FILE_KEY).unwrap().del);
        assert_eq!(l.get(FILE_KEY).unwrap().t, 300);
        // a stale edit pass (t=250) cannot overwrite the tombstone
        assert!(!stamp_whole_file_change(Some(b"v1"), Some(b"v9"), &mut l, 250, "c4"));
        assert!(l.get(FILE_KEY).unwrap().del);
        // a genuine later re-add wins
        assert!(stamp_whole_file_change(None, Some(b"v3"), &mut l, 400, "c5"));
        assert!(!l.get(FILE_KEY).unwrap().del);
        assert_eq!(l.get(FILE_KEY).unwrap().h, value_hash(b"v3"));
    }

}
