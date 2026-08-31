// T-000192: the InOutBox channel between equals (client<->client,
// server<->server, microservice<->microservice).
//
// Layout, per repo:
//
//   docs/InOutBox/outbox/<peer>/   what I sent this peer      — I write
//   docs/InOutBox/inbox/<peer>/    what this peer sent me     — the app writes
//   docs/InOutBox/pact/<peer>/     two-sided agreements       — both write, one
//                                                               file per writer
//   docs/InOutBox/registry.md      append-only number ledger  — nobody but the
//                                                               agent ever touches
//
// The folder names state a DIRECTION, not a role: equals have no roles, but an
// exchange always has a direction. That is what distinguishes this channel from
// the hierarchical ones, whose folder names encode who owes whom.
//
// Copying semantics differ per folder, deliberately:
//
//   outbox -> peer's inbox   MIRROR WITH PRUNE. The inbox is a read-only replica
//                            and must follow the author's copy, deletions included.
//   pact  <-> peer's pact    UNION WITHOUT PRUNE. Both sides put files in the same
//                            folder, so a mirror would eat the other side's files.
//
// `registry.md` sits in the channel ROOT, outside every mirrored subfolder, so no
// prune can reach it. That is the whole point: tearing an exchange down removes
// the files but never the numbers, and a number is therefore never reissued.

use std::path::{Path, PathBuf};

use crate::db::AppDb;
use crate::models::Repository;
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
    pub errors: Vec<String>,
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

pub fn outbox_dir(base: &Path, peer: &str) -> PathBuf {
    channel_root(base).join("outbox").join(peer)
}

pub fn inbox_dir(base: &Path, peer: &str) -> PathBuf {
    channel_root(base).join("inbox").join(peer)
}

pub fn pact_dir(base: &Path, peer: &str) -> PathBuf {
    channel_root(base).join("pact").join(peer)
}

/// Top-level `*.md` files in `dir` that are not markers or acknowledgements —
/// i.e. the exchange bodies. Subdirectories are ignored: the channel is flat by
/// design, one folder per counterpart. Used by the sync screen to count what is
/// open in a folder.
pub fn scan_bodies(dir: &Path) -> Vec<String> {
    scan_md(dir, |name| {
        !name.ends_with(".ack.md") && !name.ends_with(".impl.md") && !name.ends_with(".done.md")
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

/// Remove every exchange whose author dropped a closing marker, from ALL the
/// directories that hold a copy of it.
///
/// `marker` is the closing suffix (`.impl.md` for a PACT, `.done.md` for a
/// HANDOFF / NOTE); `companions` are the suffixes making up the exchange,
/// marker included. Returns the base filenames closed.
///
/// Deletion happens across every side before any copying runs. The pact union
/// has no prune, so removing the triple on one side only would let the peer's
/// surviving replica resurrect it on the next pass — the same failure mode that
/// bit the REQ channel in v1.9.5. Idempotent: a missing file is a no-op, so a
/// pair seen from both project syncs closes twice harmlessly.
fn close_marked(dirs: &[PathBuf], marker: &str, companions: &[&str], out: &mut PeerChannelOutcome) {
    let mut stems: Vec<String> = Vec::new();
    for dir in dirs {
        for name in scan_with_suffix(dir, marker) {
            if let Some(stem) = name.strip_suffix(marker) {
                if !stems.iter().any(|s| s == stem) {
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
                        Err(e) => {
                            out.errors
                                .push(format!("Peer channel: {}: {}", path.display(), e))
                        }
                    }
                }
            }
        }
        out.closed.push(format!("{stem}.md"));
    }
}

/// Mirror an author's outbox onto the peer's inbox: copy changed files, prune
/// whatever the author removed. The inbox is a replica, never an authored
/// folder, so following the source exactly is correct here.
fn mirror_outbox(src: &Path, dst: &Path, out: &mut PeerChannelOutcome) {
    match sync::mirror_dir(src, dst) {
        Ok(o) => {
            out.copied += o.copied;
            out.pruned += o.pruned;
        }
        Err(e) => out.errors.push(format!("Peer channel outbox->inbox: {e}")),
    }
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
fn union_pacts(a: &Path, b: &Path, out: &mut PeerChannelOutcome) {
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
                        .push(format!("Peer channel pact: cannot read {name}"));
                    continue;
                };
                if ca == cb {
                    continue;
                }
                match (mtime(&pa), mtime(&pb)) {
                    (Some(ta), Some(tb)) if ta > tb => copy_one(&pa, &pb, out),
                    (Some(ta), Some(tb)) if tb > ta => copy_one(&pb, &pa, out),
                    _ => out.errors.push(format!(
                        "Peer channel pact: {name} differs on both sides with the same \
                         timestamp — one file has two writers, resolve by hand"
                    )),
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
        Err(e) => out.errors.push(format!("Peer channel pact copy: {e}")),
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
            out.errors.push(format!(
                "Peer channel: repo {} root missing ({})",
                repo.display_name(),
                base.display()
            ));
            return out;
        }
    }

    let a_name = a.canonical_folder_name();
    let b_name = b.canonical_folder_name();

    if let Err(e) = ensure_skeleton(a_base, &b_name) {
        out.errors.push(format!("Peer channel skeleton: {e}"));
        return out;
    }
    if let Err(e) = ensure_skeleton(b_base, &a_name) {
        out.errors.push(format!("Peer channel skeleton: {e}"));
        return out;
    }

    let a_out = outbox_dir(a_base, &b_name);
    let a_in = inbox_dir(a_base, &b_name);
    let a_pact = pact_dir(a_base, &b_name);
    let b_out = outbox_dir(b_base, &a_name);
    let b_in = inbox_dir(b_base, &a_name);
    let b_pact = pact_dir(b_base, &a_name);

    // 1. Teardown, everywhere, before anything is copied.
    //    A PACT closes on the initiator's `.impl.md` and takes the whole triple.
    close_marked(
        &[a_pact.clone(), b_pact.clone()],
        ".impl.md",
        &[".md", ".ack.md", ".impl.md"],
        &mut out,
    );
    //    A HANDOFF / NOTE closes on the sender's `.done.md`; there is no reply
    //    file, so the pair is just body + marker. Each direction is torn down
    //    across the sender's outbox and the recipient's inbox replica.
    close_marked(
        &[a_out.clone(), b_in.clone()],
        ".done.md",
        &[".md", ".done.md"],
        &mut out,
    );
    close_marked(
        &[b_out.clone(), a_in.clone()],
        ".done.md",
        &[".md", ".done.md"],
        &mut out,
    );

    // 2. Copies.
    mirror_outbox(&a_out, &b_in, &mut out);
    mirror_outbox(&b_out, &a_in, &mut out);
    union_pacts(&a_pact, &b_pact, &mut out);

    out
}

/// Run every peer channel touching `project_id`: for each repo of the project,
/// resolve its peers (same-project role match plus explicit links) and sync the
/// pair. A pair is synced once per pass even when both of its repos belong to
/// the project.
pub fn run_peer_channels(db: &AppDb, project_id: i64) -> PeerChannelOutcome {
    let mut out = PeerChannelOutcome::default();
    let repos = match db.list_repos_by_project(Some(project_id)) {
        Ok(r) => r,
        Err(e) => {
            out.errors.push(format!("Peer channels: {e}"));
            return out;
        }
    };

    let mut done: Vec<(i64, i64)> = Vec::new();
    for repo in &repos {
        let peers = match db.peers_of_repo(repo.id) {
            Ok(p) => p,
            Err(e) => {
                out.errors.push(format!(
                    "Peer channels: peers of {}: {e}",
                    repo.display_name()
                ));
                continue;
            }
        };
        for peer in peers {
            let key = if repo.id < peer.repo.id {
                (repo.id, peer.repo.id)
            } else {
                (peer.repo.id, repo.id)
            };
            if done.contains(&key) {
                continue;
            }
            done.push(key);
            let o = sync_peer_pair(repo, &peer.repo);
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
            "notes.txt",
        ] {
            fs::write(d.join(n), "x").unwrap();
        }
        assert_eq!(scan_bodies(&d), vec!["PACT-001_x.md".to_string()]);
    }
}
