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

use std::path::{Path, PathBuf};

use crate::db::AppDb;
use crate::models::{lookup, Repository, SyncIssue, SyncIssueCode as Code};
use crate::sync;

/// Channel root under `docs/`.
pub const INOUTBOX_DIR: &str = "InOutBox";

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

/// One direction of a pair: `a_out` (author) -> `b_in` (replica + receipts).
/// Teardowns first — re-seeding a file that was just removed is the classic
/// resurrection bug. Then the receipt backflow, then the mirror.
fn sync_one_direction(a_out: &Path, b_in: &Path, out: &mut PeerChannelOutcome) {
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
    copy_responses_back(b_in, a_out, out);
    mirror_outbox(a_out, b_in, out);
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
/// it would re-seed a file that was just removed.
pub fn sync_peer_pair(a: &Repository, b: &Repository) -> PeerChannelOutcome {
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
    //    HANDOFF, an outbox `.impl.md` only a REQ triple; responses flow back;
    //    then the outbox mirrors into the inbox without touching receipts.
    sync_one_direction(&a_out, &b_in, &mut out);
    sync_one_direction(&b_out, &a_in, &mut out);
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
            let o = sync_peer_pair(repo, &peer);
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
        // sender's mirror.
        fs::write(b_in.join("REQ-001_x.response.md"), "receipt").unwrap();
        let ack = b_in.join("ANNOUNCE-002_y.md.ack");
        fs::create_dir_all(&ack).unwrap();
        fs::write(ack.join("beta"), "").unwrap();

        let out = sync_peer_pair(&a, &b);
        assert_eq!(out.pruned, 0, "recipient-authored entries are not pruned");
        assert!(b_in.join("REQ-001_x.response.md").exists());
        assert!(ack.join("beta").exists());
        // And an .ack dir sitting in the outbox never mirrors into the inbox.
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
}
