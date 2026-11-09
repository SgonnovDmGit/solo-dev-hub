// T-000192: the InOutBox channel between equals (client<->client,
// server<->server, microservice<->microservice).
// T-000190: the same layout now carries the hierarchical pairs too — a REQ
// exchange (client->server, server->microservice, consumer->codex) is just
// another kind of file in the same outbox/inbox lanes.
//
// Layout, per repo:
//
//   docs/InOutBox/outbox/<peer>/   what I sent this peer      — I write
//   docs/InOutBox/inbox/<peer>/    what this peer sent me     — the app writes
//   docs/InOutBox/pact/<peer>/     two-sided agreements       — both write, one
//                                                               file per writer
//   docs/InOutBox/registry/<peer>.md  per-pair append-only number ledger
//                                   (split of the legacy single registry.md —
//                                   nobody but the agent ever touches)
//
// The folder names state a DIRECTION, not a role: equals have no roles, but an
// exchange always has a direction.
//
// Copying semantics differ per folder AND per file kind, deliberately:
//
//   outbox -> peer's inbox   MIRROR WITH PRUNE, kind-aware. The inbox is a
//                            replica of the outbox EXCEPT the files the
//                            recipient itself authors there:
//                            `*.response.md` (the REQ answer) and the
//                            `*.ack/` marker dirs (T-000245) — those are never
//                            copied INTO the inbox and never pruned from it.
//                            Responses flow the other way: inbox -> outbox.
//   pact  <-> peer's pact    UNION WITHOUT PRUNE. Both sides put files in the same
//                            folder, so a mirror would eat the other side's files.
//
// Closing markers are kind-guarded: `.done.md` closes only NOTE-*/HANDOFF-*
// bodies, `.impl.md` in outbox/inbox closes only REQ-* triples (in pact/ it
// closes a PACT) — a stray marker must never tear down a different kind.
//
// The registry sits in the channel ROOT, outside every mirrored subfolder, so
// no prune can reach it. That is the whole point: tearing an exchange down
// removes the files but never the numbers, and a number is never reissued.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::db::AppDb;
use crate::git_ops::{self, GitBinary};
use crate::models::{lookup, Repository, SyncIssue, SyncIssueCode as Code};
use crate::sync;
use crate::sync::shared_structure as ss;

/// Channel root under `docs/`.
pub const INOUTBOX_DIR: &str = "InOutBox";

/// T-000245: transition release gate. While true, ANNOUNCE delivery into a
/// shared repo also writes the legacy role-named projection
/// (`server-announcements/<sender>/`, `microservice-announcements/<ms-project>/`)
/// so an install on an older binary still finds it where its agent looks. The
/// gate is compile-time, not a user setting: it switches off in the release
/// after v1.23.0 and never governs ack markers, version files or archives.
pub const ANNOUNCE_LEGACY_DUAL_WRITE: bool = true;

/// The append-only number ledger. Named here only so the rest of the engine can
/// assert it is never written — no code path in this module touches it.
pub const REGISTRY_FILE: &str = "registry.md";

/// Aggregated result of syncing one peer pair.
#[derive(Debug, Default)]
pub struct PeerChannelOutcome {
    /// Files written (new or changed content).
    pub copied: usize,
    /// Files removed — mirror prunes plus marker-driven teardowns.
    pub pruned: usize,
    /// Base filenames of exchanges closed by a marker this pass.
    pub closed: Vec<String>,
    /// Non-fatal per-pair failures surfaced to the user.
    pub errors: Vec<SyncIssue>,
}

pub fn channel_root(base: &Path) -> PathBuf {
    base.join("docs").join(INOUTBOX_DIR)
}

/// Path of the channel's number ledger. Exposed so callers can *report* it —
/// nothing in this module ever writes or deletes it. That abstinence is the
/// whole guarantee: tearing an exchange down removes its files, never its number.
pub fn registry_path(base: &Path) -> PathBuf {
    channel_root(base).join(REGISTRY_FILE)
}

/// T-000190: the new canonical per-pair ledger `registry/<peer>.md`.
pub fn registry_peer_path(base: &Path, peer: &str) -> PathBuf {
    channel_root(base)
        .join("registry")
        .join(format!("{}.md", peer))
}

/// T-000190: does this repo hold ANY ledger material for `peer`? Presence —
/// not content — is all the app reports: the agent owns the bytes and does the
/// split. Counts the new `registry/<peer>.md`, the legacy unified
/// `registry.md`, and the frozen `*-requirements/**/registry.md` archives
/// (flat and per-peer variants; the agent keeps them byte-identical).
pub fn registry_present(base: &Path, peer: &str) -> bool {
    if registry_peer_path(base, peer).is_file() || registry_path(base).is_file() {
        return true;
    }
    for parent in [
        "backend-requirements",
        "client-requirements",
        "microservice-requirements",
        "server-requirements",
    ] {
        let dir = base.join("docs").join(parent);
        if dir.join(REGISTRY_FILE).is_file() || dir.join(peer).join(REGISTRY_FILE).is_file() {
            return true;
        }
    }
    false
}

pub fn outbox_dir(base: &Path, peer: &str) -> PathBuf {
    channel_root(base).join("outbox").join(peer)
}

pub fn inbox_dir(base: &Path, peer: &str) -> PathBuf {
    channel_root(base).join("inbox").join(peer)
}

pub fn pact_dir(base: &Path, peer: &str) -> PathBuf {
    channel_root(base).join("pact").join(peer)
}

/// Top-level `*.md` files in `dir` that are not markers, acknowledgements or
/// receipts — i.e. the exchange bodies. Subdirectories are ignored: the
/// channel is flat by design, one folder per counterpart. Used by the sync
/// screen to count what is open in a folder. T-000190: a REQ `.response.md`
/// is the recipient's receipt, not a new message — same for `.ack.md`.
pub fn scan_bodies(dir: &Path) -> Vec<String> {
    scan_md(dir, |name| {
        !name.ends_with(".ack.md")
            && !name.ends_with(".impl.md")
            && !name.ends_with(".done.md")
            && !name.ends_with(".response.md")
    })
}

/// Top-level `*.md` files in `dir` whose name ends with `suffix`.
fn scan_with_suffix(dir: &Path, suffix: &str) -> Vec<String> {
    scan_md(dir, |name| name.ends_with(suffix))
}

fn scan_md(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".md") && keep(name))
        .collect();
    out.sort();
    out
}

/// True when two directories hold exactly the same relative files with the same
/// bytes. Deliberately NOT `sync::dirs_in_sync`, which reports an empty source
/// as "not synced" — correct for a codex svod (nothing published yet), wrong for
/// a channel folder, where empty on both sides is the normal resting state.
pub fn dirs_equal(a: &Path, b: &Path) -> bool {
    let (Ok(ea), Ok(eb)) = (std::fs::read_dir(a), std::fs::read_dir(b)) else {
        // A missing folder on either side means the channel is not laid out yet.
        return false;
    };
    let names = |entries: std::fs::ReadDir| -> Vec<String> {
        let mut v: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        v.sort();
        v
    };
    let (na, nb) = (names(ea), names(eb));
    if na != nb {
        return false;
    }
    na.iter().all(|n| {
        matches!(
            (std::fs::read(a.join(n)), std::fs::read(b.join(n))),
            (Ok(x), Ok(y)) if x == y
        )
    })
}

/// Create the three per-peer folders on one side. `registry.md` is deliberately
/// NOT created: the ledger is the agent's file, and the app not writing it is
/// the guarantee that tearing an exchange down cannot erase a number.
fn ensure_skeleton(base: &Path, peer: &str) -> Result<(), String> {
    for dir in [
        outbox_dir(base, peer),
        inbox_dir(base, peer),
        pact_dir(base, peer),
    ] {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {}", dir.display(), e))?;
    }
    Ok(())
}

/// A failed peer-channel step on `subject` (a repo, file or folder).
fn peer_issue(subject: impl Into<String>, e: impl std::fmt::Display) -> SyncIssue {
    SyncIssue::new(Code::PeerChannelFailed, subject).detail(e)
}

/// The channel kind of an exchange body, by filename prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Req,
    Announce,
    Note,
    Handoff,
    Pact,
    /// An `*.md` body without a channel prefix — mirrored like any authored
    /// file but never torn down by a kind-guarded marker.
    Other,
}

fn body_kind(stem: &str) -> BodyKind {
    if stem.starts_with("REQ-") {
        BodyKind::Req
    } else if stem.starts_with("ANNOUNCE-") {
        BodyKind::Announce
    } else if stem.starts_with("NOTE-") {
        BodyKind::Note
    } else if stem.starts_with("HANDOFF-") {
        BodyKind::Handoff
    } else if stem.starts_with("PACT-") {
        BodyKind::Pact
    } else {
        BodyKind::Other
    }
}

/// True when `name` is a file the RECIPIENT of this inbox dir authors — the
/// REQ answer `.response.md` — or a marker directory it authors — `*.ack/`
/// (T-000245). The outbox->inbox mirror neither copies over nor prunes these;
/// responses flow the other way, ack dirs get their own delivery.
fn is_recipient_authored(name: &std::ffi::OsStr) -> bool {
    let s = name.to_string_lossy();
    s.ends_with(".response.md") || s.ends_with(".ack")
}

/// Remove every exchange whose author dropped a closing marker, from ALL the
/// directories that hold a copy of it.
///
/// `marker` is the closing suffix (`.impl.md` for a PACT or a REQ, `.done.md`
/// for a HANDOFF / NOTE); `companions` are the suffixes making up the exchange,
/// marker included; `closable` gates which body kinds this marker may tear
/// down — a `.done.md` must never close a REQ or an ANNOUNCE, an outbox
/// `.impl.md` closes only a REQ triple. Returns the base filenames closed.
///
/// Deletion happens across every side before any copying runs. The pact union
/// has no prune, so removing the triple on one side only would let the peer's
/// surviving replica resurrect it on the next pass — the same failure mode that
/// bit the REQ channel in v1.9.5. Idempotent: a missing file is a no-op, so a
/// pair seen from both project syncs closes twice harmlessly.
fn close_marked_if(
    dirs: &[PathBuf],
    marker: &str,
    companions: &[&str],
    closable: impl Fn(&str) -> bool,
    out: &mut PeerChannelOutcome,
) {
    let mut stems: Vec<String> = Vec::new();
    for dir in dirs {
        for name in scan_with_suffix(dir, marker) {
            if let Some(stem) = name.strip_suffix(marker) {
                if closable(stem) && !stems.iter().any(|s| s == stem) {
                    stems.push(stem.to_string());
                }
            }
        }
    }
    for stem in stems {
        for dir in dirs {
            for suffix in companions {
                let path = dir.join(format!("{stem}{suffix}"));
                if path.exists() {
                    match std::fs::remove_file(&path) {
                        Ok(()) => out.pruned += 1,
                        Err(e) => out.errors.push(peer_issue(path.display().to_string(), e)),
                    }
                }
            }
        }
        out.closed.push(format!("{stem}.md"));
    }
}

/// Mirror an author's outbox onto the peer's inbox, kind-aware (T-000190):
/// copy changed files, prune whatever the author removed — but skip the files
/// and dirs the RECIPIENT authors in its own inbox (`*.response.md`, `*.ack/`).
/// Mirroring over them would overwrite the recipient's receipt with the
/// sender's stale replica; pruning them would delete the recipient's own work.
fn mirror_outbox(src: &Path, dst: &Path, out: &mut PeerChannelOutcome) {
    if !src.exists() {
        return; // no authored outbox — nothing to deliver, nothing to prune
    }
    if let Err(e) = std::fs::create_dir_all(dst) {
        out.errors.push(peer_issue(dst.display().to_string(), e));
        return;
    }
    // Copy pass: every source entry except recipient-authored names.
    if let Err(e) = copy_channel_tree(src, dst, &mut out.copied) {
        out.errors.push(peer_issue(dst.display().to_string(), e));
    }
    // Prune pass: dst entries absent from src, except recipient-authored names.
    if let Err(e) = prune_channel_tree(src, dst, &mut out.pruned) {
        out.errors.push(peer_issue(dst.display().to_string(), e));
    }
}

fn copy_channel_tree(src: &Path, dst: &Path, copied: &mut usize) -> Result<(), String> {
    let entries = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(|e| e.ok()) {
        if is_recipient_authored(&entry.file_name()) {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_channel_tree(&src_path, &dst_path, copied)?;
        } else if src_path.is_file() && sync::copy_file_if_changed(&src_path, &dst_path)? {
            *copied += 1;
        }
    }
    Ok(())
}

fn prune_channel_tree(src: &Path, dst: &Path, pruned: &mut usize) -> Result<(), String> {
    let entries = std::fs::read_dir(dst).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(|e| e.ok()) {
        if is_recipient_authored(&entry.file_name()) {
            continue;
        }
        let dst_path = entry.path();
        let src_path = src.join(entry.file_name());
        if dst_path.is_dir() {
            if src_path.is_dir() {
                prune_channel_tree(&src_path, &dst_path, pruned)?;
            } else {
                std::fs::remove_dir_all(&dst_path).map_err(|e| e.to_string())?;
                *pruned += 1;
            }
        } else if !src_path.exists() {
            std::fs::remove_file(&dst_path).map_err(|e| e.to_string())?;
            *pruned += 1;
        }
    }
    Ok(())
}

/// The recipient's `*.response.md` files flow back to the sender's outbox —
/// the one place where an inbox is the source of truth (T-000190: the response
/// is written by the recipient next to the body it answers).
fn copy_responses_back(inbox: &Path, outbox: &Path, out: &mut PeerChannelOutcome) {
    for name in scan_with_suffix(inbox, ".response.md") {
        let src = inbox.join(&name);
        let dst = outbox.join(&name);
        match sync::copy_file_if_changed(&src, &dst) {
            Ok(true) => out.copied += 1,
            Ok(false) => {}
            Err(e) => out.errors.push(peer_issue(dst.display().to_string(), e)),
        }
    }
}

// ---------------------------------------------------------------------------
// T-000245: per-recipient acknowledgement of announcements
// ---------------------------------------------------------------------------
//
// For a body `ANNOUNCE-NNN_<slug>.md` the reader's agent drops an EMPTY file
// `<body>.ack/<key>` beside it — `key` is the reader's GitHub login in a shared
// repo, else the recipient repo's canonical name. The app never writes a key:
// it transports markers, resets them when the body's bytes change and deletes
// the whole group once every expected reader has acknowledged.

/// The marker dir a body grows beside itself: `ANNOUNCE-015.md` →
/// `ANNOUNCE-015.md.ack/`. The `.md` suffix stays in the dir name, exactly as
/// the spec pins it.
fn ack_dir_for(dir: &Path, body: &str) -> PathBuf {
    dir.join(format!("{body}.ack"))
}

/// Names of `<body>.ack/` marker directories in `dir` (dirs only — a stray
/// `*.ack` FILE is just a file the mirror carries).
fn ack_dir_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".ack"))
        .collect();
    out.sort();
    out
}

/// A marker name is a key when it is either a login-safe basename (the
/// shared-repo form) or literally a member of the recipient's expected set —
/// a non-shared repo whose canonical name carries a `.` or `_` still needs a
/// way to acknowledge, and `file_name` can never hold a separator, so a bare
/// expected-name match is safe.
fn valid_marker_name(name: &str, expected: Option<&BTreeSet<String>>) -> bool {
    ss::is_valid_request_login(name)
        || expected
            .map(|e| e.contains(&name.to_ascii_lowercase()))
            .unwrap_or(false)
}

/// The valid marker keys inside one `.ack/` dir, lowercased — each is an EMPTY
/// regular file named by a safe basename (login charset, no Windows device
/// names, no separators possible inside `file_name`). A non-empty file, a
/// nested dir, a symlink or an invalid name is reported and never counted: a
/// key must be earned by the reader, never guessed or inherited.
fn valid_ack_keys(
    ack_dir: &Path,
    expected: Option<&BTreeSet<String>>,
    out: &mut PeerChannelOutcome,
) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(ack_dir) else {
        return keys;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let valid = match std::fs::symlink_metadata(&path) {
            Ok(m) => valid_marker_name(&name, expected) && m.file_type().is_file() && m.len() == 0,
            Err(_) => false,
        };
        if valid {
            keys.insert(name.to_ascii_lowercase());
        } else {
            out.errors.push(peer_issue(
                path.display().to_string(),
                "invalid ack marker (must be an empty file named by a login-safe key)",
            ));
        }
    }
    keys
}

/// A body the sender rewrote invalidates every marker made against the old
/// bytes: clear `<body>.ack/` on BOTH copies before the new revision is
/// mirrored — a marker must never confirm a revision it did not see.
fn reset_stale_announces(a_out: &Path, b_in: &Path, out: &mut PeerChannelOutcome) {
    let mut names = ack_dir_names(a_out);
    for n in ack_dir_names(b_in) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    for name in names {
        let body = &name[..name.len() - ".ack".len()];
        let bodies_differ = matches!(
            (std::fs::read(a_out.join(body)), std::fs::read(b_in.join(body))),
            (Ok(x), Ok(y)) if x != y
        );
        if !bodies_differ {
            continue;
        }
        for dir in [a_out, b_in] {
            let ad = dir.join(&name);
            if ad.is_dir() {
                match std::fs::remove_dir_all(&ad) {
                    Ok(()) => out.pruned += 1,
                    Err(e) => out.errors.push(peer_issue(ad.display().to_string(), e)),
                }
            }
        }
    }
}

/// A marker dir whose body is gone from BOTH copies is residue — it must not
/// resurrect a closed announcement nor linger as an unattributed claim. Runs
/// after the mirror so an author-side deletion settles the same pass.
fn drop_orphan_acks(a_out: &Path, b_in: &Path, out: &mut PeerChannelOutcome) {
    let mut names = ack_dir_names(a_out);
    for n in ack_dir_names(b_in) {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    for name in names {
        let body = &name[..name.len() - ".ack".len()];
        if a_out.join(body).is_file() || b_in.join(body).is_file() {
            continue;
        }
        for dir in [a_out, b_in] {
            let ad = dir.join(&name);
            if ad.is_dir() {
                match std::fs::remove_dir_all(&ad) {
                    Ok(()) => out.pruned += 1,
                    Err(e) => out.errors.push(peer_issue(ad.display().to_string(), e)),
                }
            }
        }
    }
}

/// The recipient's `.ack/` markers union back into the author's outbox — per
/// key, never pruned by absence: each key is one reader's acknowledgement and
/// an earlier key is never deleted because a later pass saw fewer of them.
/// Only valid markers travel; an invalid entry is reported and stays behind.
fn copy_acks_back(
    b_in: &Path,
    a_out: &Path,
    expected: Option<&BTreeSet<String>>,
    out: &mut PeerChannelOutcome,
) {
    for name in ack_dir_names(b_in) {
        let src_dir = b_in.join(&name);
        let Ok(entries) = std::fs::read_dir(&src_dir) else {
            continue;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let key = entry.file_name().to_string_lossy().to_string();
            let src = entry.path();
            let valid = match std::fs::symlink_metadata(&src) {
                Ok(m) => {
                    valid_marker_name(&key, expected) && m.file_type().is_file() && m.len() == 0
                }
                Err(_) => false,
            };
            if !valid {
                out.errors.push(peer_issue(
                    src.display().to_string(),
                    "invalid ack marker (must be an empty file named by a login-safe key)",
                ));
                continue;
            }
            let dst = a_out.join(&name).join(&key);
            if dst.exists() {
                continue;
            }
            if let Some(parent) = dst.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    out.errors.push(peer_issue(parent.display().to_string(), e));
                    continue;
                }
            }
            match std::fs::write(&dst, b"") {
                Ok(()) => out.copied += 1,
                Err(e) => out.errors.push(peer_issue(dst.display().to_string(), e)),
            }
        }
    }
}

/// The valid marker keys inside `b_in`'s `<body>.ack/` for a SHARED repo,
/// filtered by git ancestry: a marker counts only when the commit that
/// introduced it descends from the commit that set the body's current bytes.
/// A late merge can carry a marker authored against an older revision next to
/// the new body — the spec rules that ambiguity is a conflict, not an ack, so
/// such a marker is reported and never counted. An uncommitted marker is the
/// one case proven fresh: it was authored against this very worktree body.
fn current_ack_keys(
    git: &GitBinary,
    root: &Path,
    b_in: &Path,
    body: &str,
    expected: Option<&BTreeSet<String>>,
    out: &mut PeerChannelOutcome,
) -> BTreeSet<String> {
    let ack_dir = ack_dir_for(b_in, body);
    let mut keys = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(&ack_dir) else {
        return keys;
    };
    let body_rel = fwd_rel(root, &b_in.join(body));
    let body_rev = git_ops::last_commit_touching(git, root, &body_rel);
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let valid = match std::fs::symlink_metadata(&path) {
            Ok(m) => valid_marker_name(&name, expected) && m.file_type().is_file() && m.len() == 0,
            Err(_) => false,
        };
        if !valid {
            out.errors.push(peer_issue(
                path.display().to_string(),
                "invalid ack marker (must be an empty file named by a login-safe key)",
            ));
            continue;
        }
        let marker_rel = fwd_rel(root, &path);
        let stale = match (
            &body_rev,
            git_ops::last_commit_touching(git, root, &marker_rel),
        ) {
            (Ok(Some(b)), Ok(Some(m))) => match git_ops::is_ancestor(git, root, b, &m) {
                Ok(anc) => !anc,
                Err(e) => {
                    out.errors.push(peer_issue(&marker_rel, e));
                    true
                }
            },
            // Either side has no commit yet — the marker cannot be proven
            // stale against this body, and an uncommitted marker was written
            // against the body sitting in this worktree right now.
            _ => false,
        };
        if stale {
            out.errors.push(
                SyncIssue::new(Code::VersionConflict, marker_rel).detail(
                    "ack marker predates the body's current revision — treated as a conflict, not an acknowledgement",
                ),
            );
            continue;
        }
        keys.insert(name.to_ascii_lowercase());
    }
    keys
}

/// Forward-slash repo-relative path for git calls (`git log -- <path>`).
fn fwd_rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Owner-side paired close (T-000245): once every expected reader of the
/// recipient side has a valid marker, the author's outbox body and the whole
/// `<body>.ack/` are removed on BOTH copies — before any copying, so no
/// replica survives to resurrect the announcement. `expected` is the trusted
/// reader set of the RECIPIENT repo (logins lowercased); `None`/empty means
/// the roster is unknown and cleanup must not fire. `shared` carries the
/// recipient repo's git when it is shared — markers there are validated by
/// commit ancestry, not just filename.
fn cleanup_fully_acked(
    a_out: &Path,
    b_in: &Path,
    expected: Option<&BTreeSet<String>>,
    shared: Option<(&GitBinary, &Path)>,
    out: &mut PeerChannelOutcome,
) {
    let Some(expected) = expected else {
        return;
    };
    if expected.is_empty() {
        return;
    }
    for body in announce_bodies(a_out) {
        let mut have = valid_ack_keys(&ack_dir_for(a_out, &body), Some(expected), out);
        have.extend(match shared {
            Some((git, root)) => current_ack_keys(git, root, b_in, &body, Some(expected), out),
            None => valid_ack_keys(&ack_dir_for(b_in, &body), Some(expected), out),
        });
        if !expected.iter().all(|k| have.contains(k)) {
            continue;
        }
        for dir in [a_out, b_in] {
            let p = dir.join(&body);
            if p.is_file() {
                match std::fs::remove_file(&p) {
                    Ok(()) => out.pruned += 1,
                    Err(e) => out.errors.push(peer_issue(p.display().to_string(), e)),
                }
            }
            let ad = ack_dir_for(dir, &body);
            if ad.is_dir() {
                match std::fs::remove_dir_all(&ad) {
                    Ok(()) => out.pruned += 1,
                    Err(e) => out.errors.push(peer_issue(ad.display().to_string(), e)),
                }
            }
        }
        out.closed.push(body);
    }
}

/// Top-level `ANNOUNCE-*.md` bodies in a channel dir.
pub fn announce_bodies(dir: &Path) -> Vec<String> {
    scan_bodies(dir)
        .into_iter()
        .filter(|n| body_kind(n.strip_suffix(".md").unwrap_or(n)) == BodyKind::Announce)
        .collect()
}

/// The set of keys whose `.ack/` markers close an announcement sent to
/// `recipient` — the "all readers confirmed" bar. A non-shared repo is one
/// logical reader: its canonical name. A shared repo's readers are the
/// trusted roster `main.login ∪ participants[].login` taken from its own
/// `.sdh/project.json`; without a trusted roster (no db, no file, untrusted
/// structure) there is NO cleanup — falling back to the repo name would
/// close a multi-reader announcement on one install's word, which the spec
/// forbids.
fn is_shared_repo(db: Option<&AppDb>, repo: &Repository) -> bool {
    db.and_then(|d| d.get_shared_state(repo.id).ok())
        .map(|s| s.shared_mode)
        .unwrap_or(false)
}

fn expected_ack_keys(
    db: Option<&AppDb>,
    recipient: &Repository,
    recipient_base: &Path,
) -> Option<BTreeSet<String>> {
    if !is_shared_repo(db, recipient) {
        return Some(BTreeSet::from([recipient
            .canonical_folder_name()
            .to_ascii_lowercase()]));
    }
    let db = db?;
    let github_name = recipient.github_name.as_deref()?;
    let s = ss::read_structure_file(recipient_base, github_name, &ss::AdminVerified::load(db))
        .ok()??;
    let mut keys = BTreeSet::new();
    keys.insert(s.main.login.to_ascii_lowercase());
    for p in &s.participants {
        keys.insert(p.login.to_ascii_lowercase());
    }
    Some(keys)
}

/// One direction of a pair: `a_out` (author) -> `b_in` (replica + receipts).
/// Teardowns first — re-seeding a file that was just removed is the classic
/// resurrection bug. Then the announce lifecycle (revision reset, all-acked
/// cleanup, marker backflow), the receipt backflow, the mirror, and finally
/// the orphan-marker sweep that follows an author-side deletion.
fn sync_one_direction(
    a_out: &Path,
    b_in: &Path,
    expected: Option<&BTreeSet<String>>,
    shared: Option<(&GitBinary, &Path)>,
    out: &mut PeerChannelOutcome,
) {
    // A `.done.md` closes only NOTE-*/HANDOFF-* bodies — never a REQ or an
    // ANNOUNCE that happens to share the folder.
    close_marked_if(
        &[a_out.to_path_buf(), b_in.to_path_buf()],
        ".done.md",
        &[".md", ".done.md"],
        |stem| matches!(body_kind(stem), BodyKind::Note | BodyKind::Handoff),
        out,
    );
    // A `.impl.md` in outbox/inbox closes only a REQ triple (body + response +
    // marker); the PACT flavour lives in pact/ and is handled separately.
    close_marked_if(
        &[a_out.to_path_buf(), b_in.to_path_buf()],
        ".impl.md",
        &[".md", ".response.md", ".impl.md"],
        |stem| body_kind(stem) == BodyKind::Req,
        out,
    );
    // T-000245: a revised body invalidates old markers before they can travel
    // back, and a fully-acked announcement closes on both copies together.
    reset_stale_announces(a_out, b_in, out);
    cleanup_fully_acked(a_out, b_in, expected, shared, out);
    copy_acks_back(b_in, a_out, expected, out);
    copy_responses_back(b_in, a_out, out);
    mirror_outbox(a_out, b_in, out);
    drop_orphan_acks(a_out, b_in, out);
}

/// Union two `pact/` folders without pruning either.
///
/// Both sides author files into the same folder, so a mirror in either
/// direction would delete the other side's work. A file present on one side
/// only is copied across; a file present on both with differing content is
/// resolved by modification time — the newer copy wins, which is exactly
/// "the author's copy is the source of truth" once the author has edited it.
///
/// Equal timestamps with differing content cannot be attributed to either
/// author, so nothing is written and the pair is reported. Under the
/// "one file, one writer" rule that state means someone edited a replica.
///
/// `sides` names the two repos, for the report.
fn union_pacts(a: &Path, b: &Path, sides: &str, out: &mut PeerChannelOutcome) {
    let mut names = scan_md(a, |_| true);
    for name in scan_md(b, |_| true) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.sort();

    for name in names {
        let pa = a.join(&name);
        let pb = b.join(&name);
        match (pa.exists(), pb.exists()) {
            (true, false) => copy_one(&pa, &pb, out),
            (false, true) => copy_one(&pb, &pa, out),
            (true, true) => {
                let (Ok(ca), Ok(cb)) = (std::fs::read(&pa), std::fs::read(&pb)) else {
                    out.errors
                        .push(SyncIssue::new(Code::PeerChannelFailed, name.clone()).target(sides));
                    continue;
                };
                if ca == cb {
                    continue;
                }
                match (mtime(&pa), mtime(&pb)) {
                    (Some(ta), Some(tb)) if ta > tb => copy_one(&pa, &pb, out),
                    (Some(ta), Some(tb)) if tb > ta => copy_one(&pb, &pa, out),
                    _ => out
                        .errors
                        .push(SyncIssue::new(Code::VersionConflict, name.clone()).target(sides)),
                }
            }
            (false, false) => {}
        }
    }
}

fn copy_one(src: &Path, dst: &Path, out: &mut PeerChannelOutcome) {
    match sync::copy_file_if_changed(src, dst) {
        Ok(true) => out.copied += 1,
        Ok(false) => {}
        Err(e) => out.errors.push(peer_issue(dst.display().to_string(), e)),
    }
}

fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Sync one peer pair, both directions, in one pass. Argument order is
/// irrelevant — everything below is symmetric.
///
/// Order is not: teardown runs first on every side, then the copies. Reversing
/// it would re-seed a file that was just removed. Test-side entry point: no
/// db, so a shared recipient counts as roster-unknown (transport runs,
/// all-acked cleanup just never fires — the same safe fallback the spec asks
/// for). Production goes through `sync_peer_pair_ctx`.
#[cfg(test)]
pub fn sync_peer_pair(a: &Repository, b: &Repository) -> PeerChannelOutcome {
    sync_peer_pair_ctx(None, a, b)
}

/// The production entry point: `db` resolves the trusted participant roster of
/// a shared recipient for the all-acked bar. Without it a shared repo is
/// treated as roster-unknown — markers still travel, cleanup just never fires.
pub fn sync_peer_pair_ctx(
    db: Option<&AppDb>,
    a: &Repository,
    b: &Repository,
) -> PeerChannelOutcome {
    let mut out = PeerChannelOutcome::default();

    let (Some(a_local), Some(b_local)) = (a.local_path.as_deref(), b.local_path.as_deref()) else {
        // No local path on one side — nothing to sync, and not an error worth
        // repeating on every pass: the repo simply isn't on this disk.
        return out;
    };
    let a_base = Path::new(a_local);
    let b_base = Path::new(b_local);
    for (base, repo) in [(a_base, a), (b_base, b)] {
        if sync::ensure_root_exists(base).is_err() {
            out.errors.push(
                SyncIssue::new(Code::RootMissing, repo.display_name()).detail(base.display()),
            );
            return out;
        }
    }

    let a_name = a.canonical_folder_name();
    let b_name = b.canonical_folder_name();

    if let Err(e) = ensure_skeleton(a_base, &b_name) {
        out.errors.push(peer_issue(a.display_name(), e));
        return out;
    }
    if let Err(e) = ensure_skeleton(b_base, &a_name) {
        out.errors.push(peer_issue(b.display_name(), e));
        return out;
    }

    let a_out = outbox_dir(a_base, &b_name);
    let a_in = inbox_dir(a_base, &b_name);
    let a_pact = pact_dir(a_base, &b_name);
    let b_out = outbox_dir(b_base, &a_name);
    let b_in = inbox_dir(b_base, &a_name);
    let b_pact = pact_dir(b_base, &a_name);

    // 1. Teardown, everywhere, before anything is copied.
    //    A PACT closes on the initiator's `.impl.md` and takes the whole
    //    triple — and ONLY a PACT: a stray `.impl.md` must not tear down a
    //    different kind that ended up in pact/.
    close_marked_if(
        &[a_pact.clone(), b_pact.clone()],
        ".impl.md",
        &[".md", ".ack.md", ".impl.md"],
        |stem| body_kind(stem) == BodyKind::Pact,
        &mut out,
    );
    // 2. Both directions, each kind-aware: a `.done.md` closes only NOTE /
    //    HANDOFF, an outbox `.impl.md` only a REQ triple; responses and
    //    `.ack/<key>` markers flow back; then the outbox mirrors into the
    //    inbox without touching the receipts.
    let b_readers = expected_ack_keys(db, b, b_base);
    let a_readers = expected_ack_keys(db, a, a_base);
    // A shared repo's markers are judged by commit ancestry: only the install
    // holding both checkouts evaluates them, so the recipient's git is read
    // on this machine.
    let git = git_ops::cached_git();
    let b_shared = (is_shared_repo(db, b))
        .then(|| git.as_ref().map(|g| (g, b_base)))
        .flatten();
    let a_shared = (is_shared_repo(db, a))
        .then(|| git.as_ref().map(|g| (g, a_base)))
        .flatten();
    sync_one_direction(&a_out, &b_in, b_readers.as_ref(), b_shared, &mut out);
    sync_one_direction(&b_out, &a_in, a_readers.as_ref(), a_shared, &mut out);
    let sides = format!("{}, {}", a.display_name(), b.display_name());
    union_pacts(&a_pact, &b_pact, &sides, &mut out);

    out
}

/// T-000190: replay `renamed`'s repo renames on `host`'s peer-named dirs —
/// `outbox/<peer>/`, `inbox/<peer>/`, `pact/<peer>/` all follow the
/// counterparty's canonical name. Registry files are agent-owned and never
/// moved by the app; a failed lookup leaves the dirs alone rather than guess.
fn replay_peer_renames(
    db: &AppDb,
    host: &Repository,
    renamed: &Repository,
    out: &mut PeerChannelOutcome,
) {
    let (Some(hp), Ok(renames)) = (
        host.local_path.as_deref(),
        db.list_renames_for_repo(renamed.id),
    ) else {
        return;
    };
    let root = Path::new(hp).join("docs").join(INOUTBOX_DIR);
    for lane in ["outbox", "inbox", "pact"] {
        let parent = root.join(lane);
        for r in &renames {
            match sync::replay_rename_in_dir(&parent, &r.old_canonical, &r.new_canonical) {
                Ok(sync::RenameOutcome::Renamed) => {}
                Ok(sync::RenameOutcome::Collision) => out.errors.push(peer_issue(
                    &r.old_canonical,
                    format!("rename to {} collides", r.new_canonical),
                )),
                Ok(sync::RenameOutcome::NoOp) => {}
                Err(e) => out.errors.push(peer_issue(&r.old_canonical, e)),
            }
        }
    }
}

/// Run every peer channel touching `project_id`: for each repo of the project,
/// walk its explicitly linked peers and sync the pair. A pair is synced once per
/// pass even when both of its repos belong to the project.
pub fn run_peer_channels(db: &AppDb, project_id: i64) -> PeerChannelOutcome {
    let mut out = PeerChannelOutcome::default();
    let repos = match db.list_repos_by_project(Some(project_id)) {
        Ok(r) => r,
        Err(e) => {
            out.errors
                .push(SyncIssue::lookup(lookup::REPOS, format!("#{}", project_id)).detail(e));
            return out;
        }
    };

    let mut done: Vec<(i64, i64)> = Vec::new();
    for repo in &repos {
        let peers = match db.peers_of_repo(repo.id) {
            Ok(p) => p,
            Err(e) => {
                out.errors
                    .push(SyncIssue::lookup(lookup::PEERS, repo.display_name()).detail(e));
                continue;
            }
        };
        for peer in peers {
            let key = if repo.id < peer.id {
                (repo.id, peer.id)
            } else {
                (peer.id, repo.id)
            };
            if done.contains(&key) {
                continue;
            }
            done.push(key);
            // T-000190: replay renames on the peer-named dirs first — each
            // side's outbox/inbox/pact/<peer> follows the counterparty's
            // canonical name, so a rename of one repo lands on the other's fs.
            replay_peer_renames(db, repo, &peer, &mut out);
            replay_peer_renames(db, &peer, repo, &mut out);
            let o = sync_peer_pair_ctx(Some(db), repo, &peer);
            out.copied += o.copied;
            out.pruned += o.pruned;
            out.closed.extend(o.closed);
            out.errors.extend(o.errors);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn repo(id: i64, name: &str, path: &Path) -> Repository {
        Repository {
            id,
            project_id: Some(1),
            github_name: Some(format!("owner/{name}")),
            github_url: None,
            role: Some("client".into()),
            description: None,
            language: None,
            last_pushed_at: None,
            added_at: String::new(),
            updated_at: String::new(),
            local_path: Some(path.to_string_lossy().to_string()),
            github_id: None,
            deploy_target: None,
        }
    }

    /// Two repo roots on disk plus the two Repository rows pointing at them.
    fn pair() -> (TempDir, Repository, Repository) {
        let td = TempDir::new().unwrap();
        let a = td.path().join("alpha");
        let b = td.path().join("beta");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let ra = repo(1, "alpha", &a);
        let rb = repo(2, "beta", &b);
        (td, ra, rb)
    }

    fn base(r: &Repository) -> PathBuf {
        PathBuf::from(r.local_path.as_deref().unwrap())
    }

    #[test]
    fn skeleton_is_created_on_both_sides() {
        let (_td, a, b) = pair();
        let out = sync_peer_pair(&a, &b);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        for (me, peer) in [(&a, &b), (&b, &a)] {
            let n = peer.canonical_folder_name();
            assert!(outbox_dir(&base(me), &n).is_dir());
            assert!(inbox_dir(&base(me), &n).is_dir());
            assert!(pact_dir(&base(me), &n).is_dir());
        }
    }

    #[test]
    fn registry_is_never_created_or_removed_by_the_app() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let reg = registry_path(&base(&a));
        assert!(!reg.exists(), "the app must not seed the ledger");

        // The agent writes it; teardown of an exchange must leave it alone.
        fs::write(
            &reg,
            "- PACT-001 | out | beta | subject | 2026-08-31 | | | \n",
        )
        .unwrap();
        let pa = pact_dir(&base(&a), &b.canonical_folder_name());
        fs::write(pa.join("PACT-001_x.md"), "body").unwrap();
        fs::write(pa.join("PACT-001_x.impl.md"), "done").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["PACT-001_x.md".to_string()]);
        assert!(reg.exists(), "teardown must not touch the ledger");
        assert!(fs::read_to_string(&reg).unwrap().contains("PACT-001"));
    }

    #[test]
    fn outbox_mirrors_into_the_peer_inbox_including_deletions() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("NOTE-002_x.md"), "hello").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.copied, 1);
        assert_eq!(
            fs::read_to_string(b_in.join("NOTE-002_x.md")).unwrap(),
            "hello"
        );

        // The replica must follow the author, deletions included.
        fs::remove_file(a_out.join("NOTE-002_x.md")).unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.pruned, 1);
        assert!(!b_in.join("NOTE-002_x.md").exists());
    }

    #[test]
    fn pact_union_keeps_both_sides_files() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_pact = pact_dir(&base(&a), &b.canonical_folder_name());
        let b_pact = pact_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_pact.join("PACT-001_x.md"), "proposal").unwrap();
        fs::write(b_pact.join("PACT-001_x.ack.md"), "agreed").unwrap();

        let out = sync_peer_pair(&a, &b);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(out.pruned, 0, "the pact folder must never be pruned");
        // Both files now on both sides — a mirror would have eaten one of them.
        for dir in [&a_pact, &b_pact] {
            assert!(dir.join("PACT-001_x.md").exists());
            assert!(dir.join("PACT-001_x.ack.md").exists());
        }
    }

    #[test]
    fn pact_edit_by_the_author_propagates_to_the_replica() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_pact = pact_dir(&base(&a), &b.canonical_folder_name());
        let b_pact = pact_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_pact.join("PACT-001_x.md"), "v1").unwrap();
        sync_peer_pair(&a, &b);
        assert_eq!(
            fs::read_to_string(b_pact.join("PACT-001_x.md")).unwrap(),
            "v1"
        );

        // Author edits; the newer copy must win over its own replica.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(a_pact.join("PACT-001_x.md"), "v2").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(
            fs::read_to_string(b_pact.join("PACT-001_x.md")).unwrap(),
            "v2"
        );
    }

    #[test]
    fn impl_marker_closes_the_pact_triple_on_both_sides() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_pact = pact_dir(&base(&a), &b.canonical_folder_name());
        let b_pact = pact_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_pact.join("PACT-001_x.md"), "proposal").unwrap();
        fs::write(b_pact.join("PACT-001_x.ack.md"), "agreed").unwrap();
        sync_peer_pair(&a, &b);

        fs::write(a_pact.join("PACT-001_x.impl.md"), "closed").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["PACT-001_x.md".to_string()]);
        for dir in [&a_pact, &b_pact] {
            assert!(!dir.join("PACT-001_x.md").exists());
            assert!(!dir.join("PACT-001_x.ack.md").exists());
            assert!(!dir.join("PACT-001_x.impl.md").exists());
        }

        // A second pass must not resurrect anything, and must not fail.
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(!a_pact.join("PACT-001_x.md").exists());
        assert!(!b_pact.join("PACT-001_x.md").exists());
    }

    #[test]
    fn done_marker_closes_a_handoff_on_both_sides() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("HANDOFF-002_area.md"), "yours now").unwrap();
        sync_peer_pair(&a, &b);
        assert!(b_in.join("HANDOFF-002_area.md").exists());

        fs::write(a_out.join("HANDOFF-002_area.done.md"), "delivered").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["HANDOFF-002_area.md".to_string()]);
        assert!(!a_out.join("HANDOFF-002_area.md").exists());
        assert!(!a_out.join("HANDOFF-002_area.done.md").exists());
        assert!(!b_in.join("HANDOFF-002_area.md").exists());

        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty(), "teardown must be idempotent");
        assert!(out.errors.is_empty(), "{:?}", out.errors);
    }

    #[test]
    fn missing_local_path_is_a_silent_skip() {
        let (_td, a, mut b) = pair();
        b.local_path = None;
        let out = sync_peer_pair(&a, &b);
        assert!(out.errors.is_empty());
        assert_eq!(out.copied, 0);
    }

    #[test]
    fn bodies_scan_excludes_markers() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let d = pact_dir(&base(&a), &b.canonical_folder_name());
        for n in [
            "PACT-001_x.md",
            "PACT-001_x.ack.md",
            "PACT-001_x.impl.md",
            "NOTE-003_y.done.md",
            "REQ-004_x.response.md",
            "notes.txt",
        ] {
            fs::write(d.join(n), "x").unwrap();
        }
        assert_eq!(scan_bodies(&d), vec!["PACT-001_x.md".to_string()]);
    }

    // ── T-000190: kind-aware channel semantics ───────────────────────────────

    #[test]
    fn req_triple_flows_through_the_unified_channel() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        // Sender publishes the body; it mirrors into the recipient's inbox.
        fs::write(a_out.join("REQ-001_x.md"), "please add a field").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(b_in.join("REQ-001_x.md").exists());
        assert_eq!(out.copied, 1);

        // The recipient answers in place — the response flows BACK to the
        // sender's outbox and is never treated as a new body.
        fs::write(b_in.join("REQ-001_x.response.md"), "done — see REQ receipt").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(a_out.join("REQ-001_x.response.md").exists());
        assert_eq!(out.copied, 1);
        assert!(scan_bodies(&b_in).contains(&"REQ-001_x.md".to_string()));
        assert!(!scan_bodies(&b_in).iter().any(|n| n.contains("response")));

        // The sender acknowledges with .impl.md — the whole triple (body,
        // response, marker) closes on BOTH sides.
        fs::write(a_out.join("REQ-001_x.impl.md"), "confirmed").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["REQ-001_x.md".to_string()]);
        for d in [&a_out, &b_in] {
            assert!(!d.join("REQ-001_x.md").exists(), "{}", d.display());
            assert!(!d.join("REQ-001_x.response.md").exists(), "{}", d.display());
            assert!(!d.join("REQ-001_x.impl.md").exists(), "{}", d.display());
        }
    }

    #[test]
    fn a_done_marker_never_closes_a_req_or_an_announce() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("REQ-001_x.md"), "body").unwrap();
        fs::write(a_out.join("ANNOUNCE-002_y.md"), "news").unwrap();
        // A stray .done.md next to them — it mirrors like any file but must
        // tear nothing down: it is not their kind's marker.
        fs::write(a_out.join("REQ-001_x.done.md"), "stray").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty(), "{:?}", out.closed);
        assert!(a_out.join("REQ-001_x.md").exists());
        assert!(b_in.join("REQ-001_x.md").exists());
        assert!(b_in.join("ANNOUNCE-002_y.md").exists());
    }

    #[test]
    fn an_outbox_impl_marker_never_closes_a_note() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        // .impl.md is the REQ/PACT marker; a NOTE must survive it.
        fs::write(a_out.join("NOTE-003_z.md"), "fyi").unwrap();
        fs::write(a_out.join("NOTE-003_z.impl.md"), "stray").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty(), "{:?}", out.closed);
        assert!(a_out.join("NOTE-003_z.md").exists());
        assert!(b_in.join("NOTE-003_z.md").exists());
    }

    #[test]
    fn recipient_authored_files_survive_mirror_and_prune() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        // The recipient's own writings in its inbox — a REQ response and a
        // T-000245 ack marker dir — are never overwritten nor pruned by the
        // sender's mirror. The marker needs its body, else the orphan sweep
        // (correctly) treats it as residue.
        fs::write(a_out.join("ANNOUNCE-002_y.md"), "news").unwrap();
        fs::write(b_in.join("REQ-001_x.response.md"), "receipt").unwrap();
        let ack = b_in.join("ANNOUNCE-002_y.md.ack");
        fs::create_dir_all(&ack).unwrap();
        // A key that is NOT the recipient's canonical name — a pending marker
        // from a third reader; "beta" itself would close the announcement.
        fs::write(ack.join("other-reader"), "").unwrap();

        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.pruned, 0, "recipient-authored entries are not pruned");
        assert!(b_in.join("REQ-001_x.response.md").exists());
        assert!(ack.join("other-reader").exists());
        // And an .ack dir sitting in the outbox never mirrors into the inbox.
        fs::write(a_out.join("REQ-009_z.md"), "req").unwrap();
        let out_ack = a_out.join("REQ-009_z.md.ack");
        fs::create_dir_all(&out_ack).unwrap();
        fs::write(out_ack.join("alpha"), "").unwrap();
        sync_peer_pair(&a, &b);
        assert!(!b_in.join("REQ-009_z.md.ack").exists());
    }

    #[test]
    fn registry_present_sees_new_and_legacy_ledgers() {
        let (_td, a, b) = pair();
        let a_base = base(&a);
        let peer = b.canonical_folder_name();
        assert!(!registry_present(&a_base, &peer));

        // The new per-pair ledger.
        let reg_dir = a_base.join("docs/InOutBox/registry");
        fs::create_dir_all(&reg_dir).unwrap();
        fs::write(reg_dir.join(format!("{}.md", peer)), "- REQ-001 | out\n").unwrap();
        assert!(registry_present(&a_base, &peer));
        fs::remove_dir_all(&reg_dir).unwrap();
        assert!(!registry_present(&a_base, &peer));

        // The legacy unified ledger.
        fs::write(registry_path(&a_base), "- REQ-001 | out\n").unwrap();
        assert!(registry_present(&a_base, &peer));
        fs::remove_file(registry_path(&a_base)).unwrap();

        // Frozen archives — flat and per-peer.
        let flat = a_base.join("docs/backend-requirements");
        fs::create_dir_all(&flat).unwrap();
        fs::write(flat.join("registry.md"), "- REQ-001 | out\n").unwrap();
        assert!(registry_present(&a_base, &peer));
        fs::remove_file(flat.join("registry.md")).unwrap();

        let nested = a_base.join("docs/client-requirements").join(&peer);
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("registry.md"), "- REQ-001 | out\n").unwrap();
        assert!(registry_present(&a_base, &peer));
    }

    // ── T-000245: per-recipient announcement acknowledgements ───────────────

    #[test]
    fn announce_ack_closes_body_and_marker_group_on_both_sides() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("ANNOUNCE-010_change.md"), "news").unwrap();
        sync_peer_pair(&a, &b);
        assert!(b_in.join("ANNOUNCE-010_change.md").exists());

        // The recipient acknowledges — key = its canonical name in a
        // non-shared pair (db=None here → "beta").
        let ack = b_in.join("ANNOUNCE-010_change.md.ack");
        fs::create_dir_all(&ack).unwrap();
        fs::write(ack.join("beta"), "").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["ANNOUNCE-010_change.md".to_string()]);
        for d in [&a_out, &b_in] {
            assert!(
                !d.join("ANNOUNCE-010_change.md").exists(),
                "{}",
                d.display()
            );
            assert!(
                !d.join("ANNOUNCE-010_change.md.ack").exists(),
                "{}",
                d.display()
            );
        }

        // Second pass: nothing resurrects — the spec's exact failure mode.
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty());
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert!(!b_in.join("ANNOUNCE-010_change.md").exists());
    }

    #[test]
    fn cleanup_waits_for_every_expected_reader() {
        let td = TempDir::new().unwrap();
        let a_out = td.path().join("out");
        let b_in = td.path().join("in");
        fs::create_dir_all(&a_out).unwrap();
        fs::create_dir_all(&b_in).unwrap();
        for d in [&a_out, &b_in] {
            fs::write(d.join("ANNOUNCE-001.md"), "v1").unwrap();
        }
        let adir = ack_dir_for(&b_in, "ANNOUNCE-001.md");
        fs::create_dir_all(&adir).unwrap();
        fs::write(adir.join("alice"), "").unwrap();

        let expected: BTreeSet<String> = ["alice", "bob"].iter().map(|s| s.to_string()).collect();
        let mut out = PeerChannelOutcome::default();
        cleanup_fully_acked(&a_out, &b_in, Some(&expected), None, &mut out);
        assert!(
            a_out.join("ANNOUNCE-001.md").exists(),
            "one of two is not all"
        );

        fs::write(adir.join("bob"), "").unwrap();
        let mut out = PeerChannelOutcome::default();
        cleanup_fully_acked(&a_out, &b_in, Some(&expected), None, &mut out);
        assert!(!a_out.join("ANNOUNCE-001.md").exists());
        assert!(!b_in.join("ANNOUNCE-001.md").exists());
        assert!(!adir.exists(), "the whole .ack/ group goes with the body");
    }

    #[test]
    fn an_unknown_roster_never_closes_the_announcement() {
        let td = TempDir::new().unwrap();
        let a_out = td.path().join("out");
        let b_in = td.path().join("in");
        fs::create_dir_all(&a_out).unwrap();
        fs::create_dir_all(&b_in).unwrap();
        fs::write(a_out.join("ANNOUNCE-001.md"), "v1").unwrap();
        let adir = ack_dir_for(&b_in, "ANNOUNCE-001.md");
        fs::create_dir_all(&adir).unwrap();
        fs::write(adir.join("whoever"), "").unwrap();

        let mut out = PeerChannelOutcome::default();
        cleanup_fully_acked(&a_out, &b_in, None, None, &mut out);
        assert!(a_out.join("ANNOUNCE-001.md").exists());
        assert!(adir.join("whoever").exists());
    }

    #[test]
    fn a_body_revision_resets_the_old_markers() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("ANNOUNCE-003_news.md"), "v1").unwrap();
        sync_peer_pair(&a, &b);
        let ack = b_in.join("ANNOUNCE-003_news.md.ack");
        fs::create_dir_all(&ack).unwrap();
        fs::write(ack.join("beta"), "").unwrap();

        // The author edits the body: the v1 marker must not confirm v2.
        fs::write(a_out.join("ANNOUNCE-003_news.md"), "v2").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty(), "revised body stays open");
        assert!(!b_in.join("ANNOUNCE-003_news.md.ack").exists());
        assert!(!a_out.join("ANNOUNCE-003_news.md.ack").exists());
        assert_eq!(
            fs::read_to_string(b_in.join("ANNOUNCE-003_news.md")).unwrap(),
            "v2"
        );

        // A fresh marker against v2 closes it.
        fs::create_dir_all(&ack).unwrap();
        fs::write(ack.join("beta"), "").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["ANNOUNCE-003_news.md".to_string()]);
    }

    #[test]
    fn a_new_announce_number_inherits_no_markers() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("ANNOUNCE-001.md"), "old").unwrap();
        sync_peer_pair(&a, &b);
        let ack1 = b_in.join("ANNOUNCE-001.md.ack");
        fs::create_dir_all(&ack1).unwrap();
        fs::write(ack1.join("beta"), "").unwrap();
        sync_peer_pair(&a, &b);
        assert!(!a_out.join("ANNOUNCE-001.md").exists());

        fs::write(a_out.join("ANNOUNCE-002.md"), "new").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty());
        assert!(b_in.join("ANNOUNCE-002.md").exists());
        assert!(!b_in.join("ANNOUNCE-002.md.ack").exists());
        assert!(!a_out.join("ANNOUNCE-002.md.ack").exists());
    }

    #[test]
    fn invalid_ack_markers_are_reported_and_never_counted() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("ANNOUNCE-005.md"), "v1").unwrap();
        sync_peer_pair(&a, &b);
        let ack = b_in.join("ANNOUNCE-005.md.ack");
        fs::create_dir_all(&ack).unwrap();
        // Not empty, a reserved name, an illegal name, a nested dir — none of
        // them is a key a reader earned.
        fs::write(ack.join("beta"), "i read it, honest").unwrap();
        fs::write(ack.join("con"), "").unwrap();
        fs::write(ack.join("bad key"), "").unwrap();
        fs::create_dir_all(ack.join("nested")).unwrap();

        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty());
        assert!(
            out.errors.len() >= 4,
            "every invalid marker must be reported: {:?}",
            out.errors
        );
        assert!(b_in.join("ANNOUNCE-005.md").exists());

        // Replace the junk with a proper marker — now it closes.
        fs::remove_dir_all(&ack).unwrap();
        fs::create_dir_all(&ack).unwrap();
        fs::write(ack.join("beta"), "").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["ANNOUNCE-005.md".to_string()]);
    }

    #[test]
    fn an_orphan_ack_group_is_swept_and_resurrects_nothing() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        // A marker dir whose body is gone — a late-arriving receipt for a
        // closed announcement must not bring the body back.
        let ack = b_in.join("ANNOUNCE-007.md.ack");
        fs::create_dir_all(&ack).unwrap();
        fs::write(ack.join("beta"), "").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(!ack.exists(), "orphan marker group is residue");
        assert!(out.pruned >= 1);
        assert!(!b_in.join("ANNOUNCE-007.md").exists());
        let out = sync_peer_pair(&a, &b);
        assert!(!b_in.join("ANNOUNCE-007.md").exists(), "no resurrection");
        assert!(out.errors.is_empty(), "{:?}", out.errors);
    }

    #[test]
    fn done_and_impl_markers_never_close_an_announce() {
        let (_td, a, b) = pair();
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&base(&a), &b.canonical_folder_name());
        let b_in = inbox_dir(&base(&b), &a.canonical_folder_name());

        fs::write(a_out.join("ANNOUNCE-008.md"), "v1").unwrap();
        fs::write(a_out.join("ANNOUNCE-008.done.md"), "stray").unwrap();
        fs::write(a_out.join("ANNOUNCE-008.impl.md"), "stray").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty(), "{:?}", out.closed);
        assert!(b_in.join("ANNOUNCE-008.md").exists());

        // Markers landing on the recipient side must not tear it down either.
        fs::write(b_in.join("ANNOUNCE-008.done.md"), "stray").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert!(out.closed.is_empty());
        assert!(b_in.join("ANNOUNCE-008.md").exists());
    }

    #[test]
    fn a_dot_in_the_recipient_name_still_lets_it_acknowledge() {
        // GitHub allows `.`/`_` in repo names; the canonical name is the
        // non-shared ack key, so the marker named by it must count even
        // though it is not a GitHub-login-safe string.
        let td = TempDir::new().unwrap();
        let a_dir = td.path().join("alpha");
        let b_dir = td.path().join("my.app");
        fs::create_dir_all(&a_dir).unwrap();
        fs::create_dir_all(&b_dir).unwrap();
        let a = repo(1, "alpha", &a_dir);
        let b = repo(2, "my.app", &b_dir);
        sync_peer_pair(&a, &b);
        let a_out = outbox_dir(&a_dir, "my.app");
        let b_in = inbox_dir(&b_dir, "alpha");

        fs::write(a_out.join("ANNOUNCE-009.md"), "v1").unwrap();
        sync_peer_pair(&a, &b);
        let ack = b_in.join("ANNOUNCE-009.md.ack");
        fs::create_dir_all(&ack).unwrap();
        fs::write(ack.join("my.app"), "").unwrap();
        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.closed, vec!["ANNOUNCE-009.md".to_string()]);
        assert!(!a_out.join("ANNOUNCE-009.md").exists());
    }

    // ── T-000245 shared-side revision guard (needs git) ─────────────────────

    fn run_git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git invocation failed");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn a_marker_from_an_old_revision_is_a_conflict_not_an_ack() {
        use crate::git_ops::check_git_available;
        let git = check_git_available().expect("git available");

        let td = TempDir::new().unwrap();
        let root = td.path();
        run_git(root, &["init", "-q", "-b", "dev"]);
        run_git(root, &["config", "user.email", "t@e.c"]);
        run_git(root, &["config", "user.name", "T"]);

        let b_in = root.join("docs/InOutBox/inbox/alpha");
        fs::create_dir_all(&b_in).unwrap();
        fs::write(b_in.join("ANNOUNCE-001.md"), "v1").unwrap();
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-q", "-m", "v1"]);

        // A marker authored against v1 on a side branch — the late-merge case.
        run_git(root, &["switch", "-q", "-c", "stale"]);
        fs::create_dir_all(b_in.join("ANNOUNCE-001.md.ack")).unwrap();
        fs::write(b_in.join("ANNOUNCE-001.md.ack/bob"), "").unwrap();
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-q", "-m", "bob acked v1"]);

        // v2 lands on dev; the stale marker merges in afterwards.
        run_git(root, &["switch", "-q", "dev"]);
        fs::write(b_in.join("ANNOUNCE-001.md"), "v2").unwrap();
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-q", "-m", "v2"]);
        run_git(root, &["merge", "-q", "--no-edit", "stale"]);

        // A fresh marker committed on top of v2 — must count.
        fs::write(b_in.join("ANNOUNCE-001.md.ack/carol"), "").unwrap();
        run_git(root, &["add", "-A"]);
        run_git(root, &["commit", "-q", "-m", "carol acked v2"]);

        let mut out = PeerChannelOutcome::default();
        let keys = current_ack_keys(&git, root, &b_in, "ANNOUNCE-001.md", None, &mut out);
        assert!(keys.contains("carol"), "fresh marker counts: {:?}", keys);
        assert!(
            !keys.contains("bob"),
            "a marker merged in from the old revision is not an ack"
        );
        assert!(
            out.errors.iter().any(|i| i.subject.contains("bob")),
            "the stale marker must be reported: {:?}",
            out.errors
        );

        // An uncommitted marker was written against this very worktree body —
        // it cannot be stale.
        fs::write(b_in.join("ANNOUNCE-001.md.ack/dave"), "").unwrap();
        let mut out = PeerChannelOutcome::default();
        let keys = current_ack_keys(&git, root, &b_in, "ANNOUNCE-001.md", None, &mut out);
        assert!(keys.contains("dave"), "uncommitted marker is current");
    }
}
