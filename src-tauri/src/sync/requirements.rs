// REQ-pair-specific operations: rename-replay, nested-folder migration,
// the T-000190 legacy->InOutBox pair migration, and bilateral pair-deletion
// (confirm). Called from `sync_project` and `confirm_requirement` in lib.rs.

use std::fs;
use std::path::{Path, PathBuf};

use crate::models::{Repository, SyncIssue, SyncIssueCode};
use crate::sync::fs::{remove_file_if_exists, scan_impl_files};

/// B-000021: bilateral delete of a REQ pair across sender and recipient sides.
/// Resolves filesystem paths from the sender and recipient `Repository`
/// records directly — no dependency on "current project" — so the confirm
/// flow works symmetrically from either project's SyncScreen (server's
/// own view OR microservice's reverse-lookup view).
///
/// Path layout per direction (source_repo.role drives selection). T-000190:
/// the REQ triple lives on the InOutBox layout — sender `outbox/<peer>`,
/// recipient `inbox/<peer>` — and every candidate dir ALSO covers the frozen
/// legacy path so a pair that survived as migration residue still closes.
/// - `client | admin_client | test_client` → sender (client)
///   `outbox/<server>` (+ `docs/backend-requirements/`) and recipient (server)
///   `inbox/<client>` (+ `docs/client-requirements/<client-canonical>/`).
/// - `server` → sender (server) `outbox/<ms>` (+ `microservice-requirements/<ms>/`)
///   and recipient (ms-server) `inbox/<server>` (+ `server-requirements/<server-canonical>/`).
///
/// Returns `Err` for unsupported source roles (`tool` / `landing` / null) so
/// callers don't silently no-op on bad input — same contract as the previous
/// in-place implementation.
///
/// F-000039: the pair may have a third member — a `REQ-NNN_slug.impl.md`
/// sender acknowledgement — which is removed on both sides too, so the manual
/// ✓ path tears down the whole triple just like the auto-close path.
pub fn confirm_pair(
    source_repo: &Repository,
    target_repo: &Repository,
    filename: &str,
) -> Result<(), String> {
    let (Some(source_path), Some(target_path)) = (
        source_repo.local_path.as_ref(),
        target_repo.local_path.as_ref(),
    ) else {
        return Ok(());
    };
    let source_base = Path::new(source_path);
    let target_base = Path::new(target_path);
    let response_name = filename.replace(".md", ".response.md");
    let impl_name = filename.replace(".md", ".impl.md");
    let source_role = source_repo.role.as_deref().unwrap_or("");
    let source_inout = source_base.join("docs").join("InOutBox");
    let target_inout = target_base.join("docs").join("InOutBox");

    let (sender_dirs, recipient_dirs) = match source_role {
        "client" | "admin_client" | "test_client" => {
            let client_canonical = source_repo.canonical_folder_name();
            (
                vec![
                    source_inout
                        .join("outbox")
                        .join(target_repo.canonical_folder_name()),
                    source_base.join("docs").join("backend-requirements"),
                ],
                vec![
                    target_inout.join("inbox").join(&client_canonical),
                    target_base
                        .join("docs")
                        .join("client-requirements")
                        .join(&client_canonical),
                ],
            )
        }
        "server" => {
            let ms_canonical = target_repo.canonical_folder_name();
            let srv_canonical = source_repo.canonical_folder_name();
            (
                vec![
                    source_inout.join("outbox").join(&ms_canonical),
                    source_base
                        .join("docs")
                        .join("microservice-requirements")
                        .join(&ms_canonical),
                ],
                vec![
                    target_inout.join("inbox").join(&srv_canonical),
                    target_base
                        .join("docs")
                        .join("server-requirements")
                        .join(&srv_canonical),
                ],
            )
        }
        _ => {
            return Err(format!(
                "Unexpected source repo role for REQ confirm: {:?}",
                source_repo.role
            ));
        }
    };

    for dir in sender_dirs.iter().chain(recipient_dirs.iter()) {
        remove_file_if_exists(&dir.join(filename))?;
        remove_file_if_exists(&dir.join(&response_name))?;
        // F-000039: tear down the sender's `.impl.md` acknowledgement too.
        remove_file_if_exists(&dir.join(&impl_name))?;
    }
    Ok(())
}

/// F-000039: auto-close REQ pairs whose sender dropped a `.impl.md`. Scans
/// `sender_dir` for `REQ-*.impl.md`; for each, removes the whole triple
/// (`REQ-NNN_slug.md`, `.response.md`, `.impl.md`) in BOTH `sender_dir` and
/// `recipient_dir`. Idempotent (remove_file_if_exists is a no-op when absent),
/// so a pair visible from two sync directions closing twice is harmless.
/// Returns the base REQ filenames (`REQ-NNN_slug.md`) that were closed.
pub fn auto_close_impl_pairs(
    sender_dir: &Path,
    recipient_dir: &Path,
) -> Result<Vec<String>, String> {
    let mut closed = Vec::new();
    for impl_name in scan_impl_files(sender_dir) {
        let Some(stem) = impl_name.strip_suffix(".impl.md") else {
            continue;
        };
        let base = format!("{stem}.md");
        let response = format!("{stem}.response.md");
        remove_file_if_exists(&sender_dir.join(&base))?;
        remove_file_if_exists(&sender_dir.join(&response))?;
        remove_file_if_exists(&sender_dir.join(&impl_name))?;
        remove_file_if_exists(&recipient_dir.join(&base))?;
        remove_file_if_exists(&recipient_dir.join(&response))?;
        remove_file_if_exists(&recipient_dir.join(&impl_name))?;
        closed.push(base);
    }
    Ok(closed)
}

/// F-033 Stage 1e: replay a pending repo rename on the recipient side.
/// Idempotent: returns Ok(false) when there's nothing to do (old dir missing,
/// or new dir already exists — the latter counts as "migrated earlier, or
/// operator created the new folder manually"). Returns Ok(true) when the
/// filesystem was actually mutated.
///
/// Callers pass the **parent** directory (e.g. `srv/docs/client-requirements/`)
/// and the `old`/`new` canonical folder names. No DB state is updated — idempotency
/// comes from fs checks, not from a persistent "applied" flag.
/// M7 review-fix: distinguishable outcomes so callers can surface
/// "collision" as a warning instead of swallowing it as no-op. Previously
/// the function returned `Ok(false)` for both "nothing to do" and "new dir
/// already exists alongside old" — the latter is a manual-intervention
/// signal, the former is normal.
#[derive(Debug, PartialEq, Eq)]
pub enum RenameOutcome {
    /// Successfully renamed `old` → `new`.
    Renamed,
    /// No rename needed (old missing, identical names, or empty inputs).
    NoOp,
    /// Both `old` and `new` directories exist — ambiguous state, left as-is.
    Collision,
}

pub fn replay_rename_in_dir(parent: &Path, old: &str, new: &str) -> Result<RenameOutcome, String> {
    if old == new || old.is_empty() || new.is_empty() {
        return Ok(RenameOutcome::NoOp);
    }
    let old_dir = parent.join(old);
    let new_dir = parent.join(new);
    if !old_dir.exists() {
        return Ok(RenameOutcome::NoOp);
    }
    if new_dir.exists() {
        let is_empty = fs::read_dir(&new_dir)
            .map_err(|e| format!("fs::read_dir {}: {}", new_dir.display(), e))?
            .next()
            .is_none();
        if is_empty {
            // Auto-heal: an empty destination dir has nothing to lose — remove it and merge.
            fs::remove_dir(&new_dir)
                .map_err(|e| format!("fs::remove_dir {}: {}", new_dir.display(), e))?;
        } else {
            return Ok(RenameOutcome::Collision);
        }
    }
    fs::rename(&old_dir, &new_dir).map_err(|e| {
        format!(
            "fs::rename {} -> {}: {}",
            old_dir.display(),
            new_dir.display(),
            e
        )
    })?;
    Ok(RenameOutcome::Renamed)
}

/// F-033 Stage 1f Case B: one-time migration of a subfolder under a parent dir,
/// renaming `old_name/` to `new_name/` if and only if:
///   - `old_name` != `new_name`
///   - `parent/old_name/` exists
///   - `parent/new_name/` does NOT exist
/// Returns Ok(true) on actual rename, Ok(false) on no-op, and pushes a
/// `rename_collision` issue to `warnings` if both dirs coexist (ambiguous —
/// operator must resolve). An `Err` carries the path and the raw OS error only.
pub fn migrate_subfolder_rename(
    parent: &Path,
    old_name: &str,
    new_name: &str,
    warnings: &mut Vec<SyncIssue>,
) -> Result<bool, String> {
    if old_name == new_name || old_name.is_empty() || new_name.is_empty() {
        return Ok(false);
    }
    let old_dir = parent.join(old_name);
    let new_dir = parent.join(new_name);
    if !old_dir.exists() {
        return Ok(false);
    }
    if new_dir.exists() {
        warnings.push(
            SyncIssue::new(SyncIssueCode::RenameCollision, old_name)
                .target(new_name)
                .detail(format!("{}/", folder_label(parent))),
        );
        return Ok(false);
    }
    fs::rename(&old_dir, &new_dir).map_err(|e| format!("{}: {}", old_dir.display(), e))?;
    Ok(true)
}

/// The last segment of `dir`, for a report — a folder name, never prose.
fn folder_label(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.display().to_string())
}

// ── T-000190: legacy requirement dirs -> docs/InOutBox/{outbox,inbox} ────────

/// A filename that belongs to a REQ exchange — body, response receipt or the
/// sender's close marker. Only these move to the InOutBox layout; `registry.md`
/// (the frozen ledger archive), unknown files and subdirectories stay behind.
fn is_req_channel_file(name: &str) -> bool {
    name.starts_with("REQ-")
        && (name.ends_with(".md") || name.ends_with(".response.md") || name.ends_with(".impl.md"))
}

/// One planned move in a pair migration. `Dedup` means the destination already
/// holds byte-identical content: the source is deleted, the copy kept.
/// `Copy` keeps the source: the file also lands on the pair's OTHER side,
/// which has no channel copy of its own (the mirror prunes an inbox replica
/// that lacks an outbox master — a body found only on the recipient's legacy
/// dir must reach the sender's outbox or it is eaten on the next pass).
#[derive(Debug)]
enum Planned {
    Move { src: PathBuf, dst: PathBuf },
    Dedup { src: PathBuf, dst: PathBuf },
    Copy { src: PathBuf, dst: PathBuf },
}

/// Build the move plan for one legacy dir into its new channel dir.
/// `conflicts` collects same-name-different-bytes pairs — both versions are
/// preserved (source stays in the frozen legacy dir, target keeps its own),
/// and an issue is raised rather than an overwrite or an mtime pick.
fn plan_dir(
    src_dir: &Path,
    dst_dir: &Path,
    warnings: &mut Vec<SyncIssue>,
) -> Result<Vec<Planned>, String> {
    let mut plan = Vec::new();
    if !src_dir.is_dir() {
        return Ok(plan);
    }
    for entry in fs::read_dir(src_dir).map_err(|e| format!("{}: {}", src_dir.display(), e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_req_channel_file(name) {
            continue;
        }
        let dst = dst_dir.join(name);
        if dst.exists() {
            let a = fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            let b = fs::read(&dst).map_err(|e| format!("{}: {}", dst.display(), e))?;
            if a == b {
                plan.push(Planned::Dedup { src: path, dst });
            } else {
                warnings.push(
                    SyncIssue::new(SyncIssueCode::MigrationDiverged, name.to_string())
                        .target(dst_dir.display().to_string()),
                );
            }
        } else {
            plan.push(Planned::Move { src: path, dst });
        }
    }
    Ok(plan)
}

/// Apply one side's plan: move (or dedup-delete) every planned file.
/// On the first failure every completed step is rolled back — the pair works
/// on the legacy layout for one more pass rather than half-migrated.
fn apply_plan(plan: &[Planned]) -> Result<usize, String> {
    let mut done = 0usize;
    let mut moved: Vec<&Planned> = Vec::new();
    let result = (|| -> Result<(), String> {
        for step in plan {
            match step {
                Planned::Move { src, dst } => {
                    if let Some(p) = dst.parent() {
                        fs::create_dir_all(p).map_err(|e| format!("{}: {}", p.display(), e))?;
                    }
                    move_file(src, dst)?;
                }
                Planned::Dedup { src, .. } => {
                    fs::remove_file(src).map_err(|e| format!("{}: {}", src.display(), e))?;
                }
                Planned::Copy { src, dst } => {
                    if let Some(p) = dst.parent() {
                        fs::create_dir_all(p).map_err(|e| format!("{}: {}", p.display(), e))?;
                    }
                    fs::copy(src, dst)
                        .map_err(|e| format!("{} -> {}: {}", src.display(), dst.display(), e))?;
                }
            }
            moved.push(step);
            done += 1;
        }
        Ok(())
    })();
    if let Err(e) = result {
        // Roll back: put every moved file back to its source; a deduped
        // source is restored by COPYING the pre-existing destination bytes —
        // moving them would erase the target copy the dedup relied on.
        rollback_steps(moved.into_iter().rev());
        return Err(e);
    }
    Ok(done)
}

fn rollback_steps<'a>(steps: impl Iterator<Item = &'a Planned>) {
    for step in steps {
        match step {
            Planned::Move { src, dst } => {
                let _ = move_file(dst, src);
            }
            Planned::Dedup { src, dst } => {
                let _ = fs::copy(dst, src);
            }
            Planned::Copy { dst, .. } => {
                let _ = fs::remove_file(dst);
            }
        }
    }
}

/// Move `src` to `dst`, falling back to copy+remove across volumes where
/// `fs::rename` refuses (EXDEV — e.g. repos on different drives).
fn move_file(src: &Path, dst: &Path) -> Result<(), String> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(src, dst)
                .map_err(|e| format!("{} -> {}: {}", src.display(), dst.display(), e))?;
            // Verify the copy before letting the source go.
            let (a, b) = (
                fs::read(src).map_err(|e| e.to_string())?,
                fs::read(dst).map_err(|e| e.to_string())?,
            );
            if a != b {
                let _ = fs::remove_file(dst);
                return Err(format!("copy verification failed: {}", dst.display()));
            }
            fs::remove_file(src).map_err(|e| format!("{}: {}", src.display(), e))
        }
    }
}

/// Remove `dir` when it holds nothing after the migration — residue (frozen
/// ledgers, unknown files, subdirs) keeps the archive directory alive.
fn remove_dir_if_empty(dir: &Path) -> Result<bool, String> {
    match fs::remove_dir(dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Ok(false), // non-empty or otherwise busy — archive stays
    }
}

/// Recursively collect `(relative_path, file_path)` under `dir`.
fn walk_files(dir: &Path, rel: &Path, out: &mut Vec<(PathBuf, PathBuf)>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {}", dir.display(), e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let r = rel.join(entry.file_name());
        if p.is_dir() {
            walk_files(&p, &r, out)?;
        } else if p.is_file() {
            out.push((r, p));
        }
    }
    Ok(())
}

/// Drop `dir` and every subdirectory left empty — the mirror of a merge that
/// walked nested paths (`<body>.ack/<key>` markers sit one level down).
fn prune_tree(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            prune_tree(&p);
        }
    }
    let _ = fs::remove_dir(dir);
}

/// T-000190: merge a channel dir a rename COLLISION left behind into the
/// new-named dir — both name the same counterparty, so the old dir's files
/// belong to the same pair and migrate in the same pass instead of waiting
/// for the blocking dir to retire (an InOutBox lane dir never does).
/// Recursive; identical targets dedup, divergent same-name files keep BOTH
/// and report `MigrationDiverged` — the old dir keeps the frozen version for
/// the operator to resolve. A failed move rolls this call's own steps back.
pub fn merge_channel_dir(
    src_dir: &Path,
    dst_dir: &Path,
    warnings: &mut Vec<SyncIssue>,
) -> Result<usize, String> {
    let mut files = Vec::new();
    if src_dir.is_dir() {
        walk_files(src_dir, Path::new(""), &mut files)?;
    }
    let mut plan = Vec::new();
    for (rel, src) in files {
        let dst = dst_dir.join(&rel);
        if dst.exists() {
            let a = fs::read(&src).map_err(|e| format!("{}: {}", src.display(), e))?;
            let b = fs::read(&dst).map_err(|e| format!("{}: {}", dst.display(), e))?;
            if a == b {
                plan.push(Planned::Dedup { src, dst });
            } else {
                warnings.push(
                    SyncIssue::new(
                        SyncIssueCode::MigrationDiverged,
                        rel.to_string_lossy().replace('\\', "/"),
                    )
                    .target(dst_dir.display().to_string()),
                );
            }
        } else {
            plan.push(Planned::Move { src, dst });
        }
    }
    let done = apply_plan(&plan)?;
    prune_tree(src_dir);
    Ok(done)
}

/// Outcome of one REQ pair's legacy->InOutBox migration.
#[derive(Debug, Default)]
pub struct ReqMigrationOutcome {
    /// Files moved or deduped into the new layout, both sides summed.
    pub migrated: usize,
    /// The pair's channel may sync on the new layout this pass. False when a
    /// failure rolled the migration back — the legacy dirs keep working.
    pub channel_ready: bool,
}

/// Cross-populate channel files that only ONE side of the pair still has.
/// The new layout splits each file's copies across repos: the sender's
/// `outbox/` holds the master and the recipient's `inbox/` the replica, and
/// the mirror prunes a replica whose master is absent. A REQ body the sender
/// already lost locally survives only on the recipient's legacy dir — moving
/// it into `inbox/` alone would be eaten on the very next pass. So every file
/// a side migrates also lands as a COPY in each other side's new dir whenever
/// that side has no channel file of the same name. The copy reads the file's
/// post-move location (`dst`), which is why this runs after the side plans.
/// Byte-identical targets dedup; divergent same-name targets keep both and
/// report `MigrationDiverged`.
fn plan_cross(
    sides: &[(&Path, &Path)],
    plans: &[Vec<Planned>],
    warnings: &mut Vec<SyncIssue>,
) -> Result<Vec<Planned>, String> {
    let mut cross = Vec::new();
    let mut planned: std::collections::HashMap<PathBuf, PathBuf> = std::collections::HashMap::new();
    for (i, plan_i) in plans.iter().enumerate() {
        // Post-migration location of every file this side contributes — a
        // diverged name has no plan entry and stays frozen, so it is also
        // exempt from crossing.
        for step in plan_i {
            // `src` exists at plan time; `dst` is where the bytes live once
            // the side plan applies — the Copy must read the latter.
            let (src, post) = match step {
                Planned::Move { src, dst } | Planned::Dedup { src, dst } => (src, dst),
                Planned::Copy { .. } => continue,
            };
            let Some(name) = post.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            for (j, (src_j, dst_j)) in sides.iter().enumerate() {
                if i == j || *dst_j == sides[i].1 {
                    continue;
                }
                // The other side holds its own version — its side plan owns
                // the name, a cross copy would collide.
                if src_j.join(name).is_file() {
                    continue;
                }
                let dst = dst_j.join(name);
                if dst.exists() {
                    let a = fs::read(src).map_err(|e| format!("{}: {}", src.display(), e))?;
                    let b = fs::read(&dst).map_err(|e| format!("{}: {}", dst.display(), e))?;
                    if a != b {
                        warnings.push(
                            SyncIssue::new(SyncIssueCode::MigrationDiverged, name.to_string())
                                .target(dst_j.display().to_string()),
                        );
                    }
                    continue;
                }
                if let Some(first) = planned.get(&dst) {
                    // Another side already feeds this target — dedup identical
                    // bytes, keep both (and report) on divergence.
                    let a = fs::read(src).map_err(|e| format!("{}: {}", src.display(), e))?;
                    let b = fs::read(first).map_err(|e| format!("{}: {}", first.display(), e))?;
                    if a != b {
                        warnings.push(
                            SyncIssue::new(SyncIssueCode::MigrationDiverged, name.to_string())
                                .target(dst_j.display().to_string()),
                        );
                    }
                    continue;
                }
                cross.push(Planned::Copy {
                    src: post.clone(),
                    dst: dst.clone(),
                });
                planned.insert(dst, src.clone());
            }
        }
    }
    Ok(cross)
}

/// T-000190: move a REQ pair's legacy folders onto the InOutBox layout.
///
/// `legacy` is a list of `(source_dir, target_dir)` sides — for a
/// client->server pair that is `backend-requirements/` -> `outbox/<server>`
/// plus `client-requirements/<client>/` -> `inbox/<client>`; the same shape
/// covers server->microservice and consumer->codex pairs.
///
/// The move is logically atomic for the pair: each side is planned first, and
/// a filesystem failure rolls back everything this call already did — the
/// channel never runs half on the old layout, half on the new. Conflicting
/// bytes at a target keep BOTH versions (the legacy file stays, an issue is
/// raised). A file only one side still holds is cross-populated to every
/// other side's new dir so no replica is left without its master. Idempotent:
/// a second call sees an empty source list and returns `channel_ready`
/// without touching anything.
pub fn migrate_req_pair(
    sides: &[(&Path, &Path)],
    warnings: &mut Vec<SyncIssue>,
) -> Result<ReqMigrationOutcome, String> {
    let mut plans: Vec<Vec<Planned>> = Vec::new();
    for (src, dst) in sides {
        plans.push(plan_dir(src, dst, warnings)?);
    }
    let cross = plan_cross(sides, &plans, warnings)?;
    let mut out = ReqMigrationOutcome {
        channel_ready: true,
        ..Default::default()
    };
    let mut applied: Vec<(&Vec<Planned>, usize)> = Vec::new();
    for ((_, _), plan) in sides.iter().zip(plans.iter()) {
        match apply_plan(plan) {
            Ok(n) => {
                applied.push((plan, n));
                out.migrated += n;
            }
            Err(e) => {
                // This side already rolled itself back; roll the earlier
                // sides back too so the pair stays consistent.
                for (plan, _) in applied.iter().rev() {
                    rollback_steps(plan.iter().rev());
                }
                out.channel_ready = false;
                warnings.push(SyncIssue::new(SyncIssueCode::MigrationFailed, e));
                return Ok(out);
            }
        }
    }
    match apply_plan(&cross) {
        Ok(n) => out.migrated += n,
        Err(e) => {
            // The cross plan rolled itself back; put the side moves back so
            // the whole pair returns to the legacy layout.
            for (plan, _) in applied.iter().rev() {
                rollback_steps(plan.iter().rev());
            }
            out.channel_ready = false;
            warnings.push(SyncIssue::new(SyncIssueCode::MigrationFailed, e));
            return Ok(out);
        }
    }
    // All sides applied — drop legacy dirs that came out empty.
    for (src, _) in sides {
        let _ = remove_dir_if_empty(src);
    }
    Ok(out)
}

/// T-000190/T-000245: migrate one legacy ANNOUNCE folder onto the pairwise
/// layout. Announcements never had a sender copy, so each body lands in TWO
/// places at once — the recipient's `inbox/<sender>/` replica AND the sender's
/// `outbox/<recipient>/` master. The outbox copy is not optional: the mirror
/// prunes anything absent from it, and a body migrated into the inbox alone
/// would be eaten on the very next pass ("an absent source is not a
/// deletion" is exactly why both halves are written before the channel runs).
///
/// Per file: a byte-identical destination dedups; a divergent same-name
/// destination keeps BOTH versions and raises `MigrationDiverged`; an
/// unwritable copy rolls back only what this file just created and reports
/// `MigrationFailed` — the legacy file then stays and the pass retries.
/// `keep_legacy` marks a shared recipient: there the legacy file REMAINS as
/// the old-agent projection the dual-write would produce anyway. `closed`
/// names files whose new-layout body was already delivered and cleaned — a
/// leftover projection of an acknowledged announcement; they are left in
/// place and must NOT spawn fresh copies (the projection retire pass owns
/// them). Otherwise the legacy file goes once both new copies are verified,
/// and the dir drops when left empty. Idempotent on repeat.
pub fn migrate_announce_dir(
    legacy_dir: &Path,
    new_inbox: &Path,
    new_outbox: &Path,
    keep_legacy: bool,
    closed: &std::collections::BTreeSet<String>,
    warnings: &mut Vec<SyncIssue>,
) -> Result<usize, String> {
    if !legacy_dir.is_dir() {
        return Ok(0);
    }
    let mut migrated = 0usize;
    for entry in fs::read_dir(legacy_dir).map_err(|e| format!("{}: {}", legacy_dir.display(), e))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !(name.starts_with("ANNOUNCE-") && name.ends_with(".md")) || closed.contains(name) {
            continue;
        }
        let src_bytes = fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        // Divergence gate: a same-name file with different bytes at either
        // destination freezes this file — keep both, report, never pick one.
        let mut diverged = false;
        for dst_dir in [new_inbox, new_outbox] {
            let dst = dst_dir.join(name);
            if dst.exists() {
                let b = fs::read(&dst).map_err(|e| format!("{}: {}", dst.display(), e))?;
                if b != src_bytes {
                    diverged = true;
                }
            }
        }
        if diverged {
            warnings.push(
                SyncIssue::new(SyncIssueCode::MigrationDiverged, name.to_string())
                    .target(legacy_dir.display().to_string()),
            );
            continue;
        }
        // Write the missing copies; on failure remove only what this file
        // just created — the earlier identical state is not ours to undo.
        let mut created: Vec<PathBuf> = Vec::new();
        let mut failure: Option<String> = None;
        for dst_dir in [new_inbox, new_outbox] {
            let dst = dst_dir.join(name);
            if dst.exists() {
                continue;
            }
            if let Some(p) = dst.parent() {
                if let Err(e) = fs::create_dir_all(p) {
                    failure = Some(format!("{}: {}", p.display(), e));
                    break;
                }
            }
            if let Err(e) = fs::write(&dst, &src_bytes) {
                failure = Some(format!("{}: {}", dst.display(), e));
                break;
            }
            created.push(dst);
        }
        if let Some(e) = failure {
            for c in created {
                let _ = fs::remove_file(&c);
            }
            warnings.push(SyncIssue::new(SyncIssueCode::MigrationFailed, e));
            continue;
        }
        migrated += 1;
        if !keep_legacy {
            fs::remove_file(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
        }
    }
    if !keep_legacy {
        let _ = remove_dir_if_empty(legacy_dir);
    }
    Ok(migrated)
}

/// F-033 Stage 1f Case C: migrate files from the flat `server-requirements/`
/// on the microservice side into per-parent subfolders `server-requirements/<parent>/`.
///
/// Attribution algorithm per flat REQ file:
/// - Collect matching parents (byte-equal copy at `<parent-side>/<filename>`).
/// - Collect conflicting parents (copy exists but content differs).
/// - If any conflicts → leave file in flat root + push error (unsafe).
/// - Else if 0 matches → leave + warn (orphan).
/// - Else if 1 match → `fs::rename` into that parent's subfolder.
/// - Else (>= 2 same-content) → `fs::copy` into each matching parent's subfolder,
///   then `fs::remove_file` the source.
///
/// `parent_lookup` is a callable `fn(filename) -> Vec<(parent_canonical, existing_path_on_parent)>`.
/// Passing it as a closure keeps this helper unit-testable without DB / repo deps.
///
/// Returns number of flat files migrated (moved or copied-N-and-removed). Files
/// left in place are reported to `warnings` as issues: `migration_left_in_place`
/// (no matching parent copy), `migration_diverged` (copies differ), or
/// `migration_failed` with the raw OS error. An `Err` carries the path and the
/// raw OS error only.
pub fn migrate_flat_to_nested<F>(
    flat_root: &Path,
    mut parent_lookup: F,
    warnings: &mut Vec<SyncIssue>,
) -> Result<usize, String>
where
    F: FnMut(&str) -> Vec<(String, std::path::PathBuf)>,
{
    if !flat_root.exists() || !flat_root.is_dir() {
        return Ok(0);
    }
    // Collect flat REQ/.response files (skip subdirs and non-matching names).
    let mut flat_files: Vec<std::path::PathBuf> = Vec::new();
    for entry in fs::read_dir(flat_root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("REQ-") {
            continue;
        }
        if !name.ends_with(".md") {
            continue;
        }
        flat_files.push(p);
    }

    let mut migrated: usize = 0;
    for flat in flat_files {
        let fname = flat
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let flat_content = match fs::read(&flat) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(SyncIssue::new(SyncIssueCode::MigrationFailed, fname).detail(e));
                continue;
            }
        };
        let candidates = parent_lookup(&fname);
        let mut matches: Vec<String> = Vec::new();
        let mut conflicts: Vec<String> = Vec::new();
        for (parent_canonical, candidate_path) in candidates {
            match fs::read(&candidate_path) {
                Ok(c) if c == flat_content => matches.push(parent_canonical),
                Ok(_) => conflicts.push(parent_canonical),
                Err(_) => { /* parent-side file missing or unreadable — ignore */ }
            }
        }
        if !conflicts.is_empty() {
            warnings.push(
                SyncIssue::new(SyncIssueCode::MigrationDiverged, fname)
                    .target(conflicts.join(", ")),
            );
            continue;
        }
        match matches.len() {
            0 => {
                warnings.push(SyncIssue::new(SyncIssueCode::MigrationLeftInPlace, fname));
            }
            1 => {
                let dst = flat_root.join(&matches[0]).join(&fname);
                if let Some(dst_dir) = dst.parent() {
                    fs::create_dir_all(dst_dir).map_err(|e| e.to_string())?;
                }
                fs::rename(&flat, &dst).map_err(|e| format!("{}: {}", flat.display(), e))?;
                migrated += 1;
            }
            _ => {
                // Multi-parent same-content: duplicate into each matching parent's
                // subfolder. If any copy fails mid-loop, roll back the successful
                // ones and leave the flat source intact — preserves "all or nothing"
                // for the parent set so the next sync pass can retry cleanly without
                // ghost partial copies in some parents.
                let mut created: Vec<std::path::PathBuf> = Vec::new();
                let mut copy_error: Option<String> = None;
                for parent_canonical in &matches {
                    let dst = flat_root.join(parent_canonical).join(&fname);
                    if let Some(dst_dir) = dst.parent() {
                        if let Err(e) = fs::create_dir_all(dst_dir) {
                            copy_error = Some(e.to_string());
                            break;
                        }
                    }
                    if let Err(e) = fs::copy(&flat, &dst) {
                        copy_error = Some(format!("{}: {}", dst.display(), e));
                        break;
                    }
                    created.push(dst);
                }
                if let Some(err) = copy_error {
                    for dst in &created {
                        let _ = fs::remove_file(dst);
                    }
                    // Rolled back; left in the flat root for the next pass.
                    warnings.push(
                        SyncIssue::new(SyncIssueCode::MigrationFailed, fname.clone()).detail(err),
                    );
                    continue;
                }
                fs::remove_file(&flat).map_err(|e| format!("{}: {}", flat.display(), e))?;
                migrated += 1;
            }
        }
    }
    Ok(migrated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── F-033 Stage 1e: replay_rename_in_dir ─────────────────────────────────

    #[test]
    fn test_replay_rename_noop_when_old_missing() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        // new exists but no old
        fs::create_dir_all(parent.join("new-name")).unwrap();
        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert_eq!(result, RenameOutcome::NoOp, "no-op when old_dir missing");
        assert!(parent.join("new-name").exists());
    }

    #[test]
    fn test_replay_rename_success() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old-name")).unwrap();
        fs::write(parent.join("old-name/REQ-001.md"), "x").unwrap();

        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert_eq!(result, RenameOutcome::Renamed);
        assert!(!parent.join("old-name").exists());
        assert!(parent.join("new-name").exists());
        assert!(parent.join("new-name/REQ-001.md").exists());
    }

    #[test]
    fn test_replay_rename_collision_when_both_exist() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old-name")).unwrap();
        fs::create_dir_all(parent.join("new-name")).unwrap();
        // new-name is non-empty → genuine collision (empty new-name now auto-heals)
        fs::write(parent.join("new-name/REQ-existing.md"), "y").unwrap();

        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert_eq!(
            result,
            RenameOutcome::Collision,
            "both dirs present and new non-empty → collision"
        );
        assert!(parent.join("old-name").exists(), "old preserved");
        assert!(parent.join("new-name").exists(), "new preserved");
    }

    #[test]
    fn test_replay_rename_same_name_noop() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("same")).unwrap();
        let result = replay_rename_in_dir(tmp.path(), "same", "same").unwrap();
        assert_eq!(result, RenameOutcome::NoOp);
    }

    #[test]
    fn test_replay_rename_empty_new_merges() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old-name")).unwrap();
        fs::write(parent.join("old-name/REQ-001.md"), "x").unwrap();
        // new exists but is empty → auto-heal: remove it and proceed with rename
        fs::create_dir_all(parent.join("new-name")).unwrap();

        let result = replay_rename_in_dir(parent, "old-name", "new-name").unwrap();
        assert_eq!(result, RenameOutcome::Renamed);
        assert!(!parent.join("old-name").exists());
        assert!(parent.join("new-name").exists());
        assert!(parent.join("new-name/REQ-001.md").exists());
    }

    // ── F-033 Stage 1f Case B: migrate_subfolder_rename ──────────────────────

    #[test]
    fn test_migrate_subfolder_rename_basic() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("ProjectName")).unwrap();
        fs::write(parent.join("ProjectName/REQ-001.md"), "data").unwrap();

        let mut warnings = Vec::new();
        let result =
            migrate_subfolder_rename(parent, "ProjectName", "repo-canonical", &mut warnings)
                .unwrap();
        assert!(result);
        assert!(!parent.join("ProjectName").exists());
        assert!(parent.join("repo-canonical/REQ-001.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_migrate_subfolder_rename_ambiguous_both_exist() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        fs::create_dir_all(parent.join("old")).unwrap();
        fs::create_dir_all(parent.join("new")).unwrap();

        let mut warnings = Vec::new();
        let result = migrate_subfolder_rename(parent, "old", "new", &mut warnings).unwrap();
        assert!(!result);
        assert!(parent.join("old").exists());
        assert!(parent.join("new").exists());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, SyncIssueCode::RenameCollision);
        assert_eq!(
            (warnings[0].subject.as_str(), warnings[0].target.as_deref()),
            ("old", Some("new"))
        );
    }

    // ── F-033 Stage 1f Case C: migrate_flat_to_nested ────────────────────────

    #[test]
    fn test_case_c_single_parent_migrates_to_subfolder() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "payload").unwrap();
        fs::write(ms_flat.join("REQ-002.md"), "payload2").unwrap();

        // Parent side — both REQ present with identical content.
        let parent_side = tmp.path().join("parent-side");
        fs::create_dir_all(&parent_side).unwrap();
        fs::write(parent_side.join("REQ-001.md"), "payload").unwrap();
        fs::write(parent_side.join("REQ-002.md"), "payload2").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parent-A".to_string(), parent_side.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 2);
        assert!(ms_flat.join("parent-A/REQ-001.md").exists());
        assert!(ms_flat.join("parent-A/REQ-002.md").exists());
        assert!(!ms_flat.join("REQ-001.md").exists());
        assert!(!ms_flat.join("REQ-002.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_case_c_multi_parent_same_content_duplicates() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-007.md"), "shared").unwrap();

        let pa = tmp.path().join("pa");
        let pb = tmp.path().join("pb");
        fs::create_dir_all(&pa).unwrap();
        fs::create_dir_all(&pb).unwrap();
        fs::write(pa.join("REQ-007.md"), "shared").unwrap();
        fs::write(pb.join("REQ-007.md"), "shared").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![
                ("parentA".to_string(), pa.join(name)),
                ("parentB".to_string(), pb.join(name)),
            ]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 1);
        assert!(ms_flat.join("parentA/REQ-007.md").exists());
        assert!(ms_flat.join("parentB/REQ-007.md").exists());
        assert!(!ms_flat.join("REQ-007.md").exists());
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_case_c_multi_parent_different_content_unsafe() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-010.md"), "payloadA").unwrap();

        let pa = tmp.path().join("pa");
        let pb = tmp.path().join("pb");
        fs::create_dir_all(&pa).unwrap();
        fs::create_dir_all(&pb).unwrap();
        fs::write(pa.join("REQ-010.md"), "payloadA").unwrap();
        fs::write(pb.join("REQ-010.md"), "payloadB-different").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![
                ("parentA".to_string(), pa.join(name)),
                ("parentB".to_string(), pb.join(name)),
            ]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 0);
        assert!(ms_flat.join("REQ-010.md").exists(), "kept in flat root");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, SyncIssueCode::MigrationDiverged);
        assert_eq!(warnings[0].subject, "REQ-010.md");
        assert_eq!(warnings[0].target.as_deref(), Some("parentB"));
    }

    #[test]
    fn test_case_c_orphan_file() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-999.md"), "stale").unwrap();

        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        // parent has no REQ-999

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parentA".to_string(), pa.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 0);
        assert!(ms_flat.join("REQ-999.md").exists());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, SyncIssueCode::MigrationLeftInPlace);
        assert_eq!(warnings[0].subject, "REQ-999.md");
        assert_eq!(warnings[0].detail, None);
    }

    #[test]
    fn test_case_c_idempotent() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "x").unwrap();
        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        fs::write(pa.join("REQ-001.md"), "x").unwrap();

        let mk_lookup = || {
            let pa_cloned = pa.clone();
            move |name: &str| -> Vec<(String, std::path::PathBuf)> {
                vec![("parentA".to_string(), pa_cloned.join(name))]
            }
        };

        let mut w1 = Vec::new();
        let m1 = migrate_flat_to_nested(&ms_flat, mk_lookup(), &mut w1).unwrap();
        assert_eq!(m1, 1);

        // Second pass: flat root is empty of REQ-*.md → no-op.
        let mut w2 = Vec::new();
        let m2 = migrate_flat_to_nested(&ms_flat, mk_lookup(), &mut w2).unwrap();
        assert_eq!(m2, 0);
        assert!(w2.is_empty());
    }

    #[test]
    fn test_case_c_response_file_migrates_too() {
        let tmp = TempDir::new().unwrap();
        let ms_flat = tmp.path().join("server-requirements");
        fs::create_dir_all(&ms_flat).unwrap();
        fs::write(ms_flat.join("REQ-001.md"), "req").unwrap();
        fs::write(ms_flat.join("REQ-001.response.md"), "resp").unwrap();

        let pa = tmp.path().join("pa");
        fs::create_dir_all(&pa).unwrap();
        fs::write(pa.join("REQ-001.md"), "req").unwrap();
        fs::write(pa.join("REQ-001.response.md"), "resp").unwrap();

        let lookup = |name: &str| -> Vec<(String, std::path::PathBuf)> {
            vec![("parentA".to_string(), pa.join(name))]
        };

        let mut warnings = Vec::new();
        let migrated = migrate_flat_to_nested(&ms_flat, lookup, &mut warnings).unwrap();
        assert_eq!(migrated, 2);
        assert!(ms_flat.join("parentA/REQ-001.md").exists());
        assert!(ms_flat.join("parentA/REQ-001.response.md").exists());
    }

    // ── B-000021: confirm_pair bilateral REQ-pair delete ─────────────────────

    fn mk_repo(id: i64, github_name: &str, role: &str, local_path: &Path) -> Repository {
        Repository {
            id,
            project_id: None,
            github_name: Some(github_name.to_string()),
            github_url: None,
            role: Some(role.to_string()),
            description: None,
            language: None,
            last_pushed_at: None,
            added_at: String::new(),
            updated_at: String::new(),
            local_path: Some(local_path.to_string_lossy().to_string()),
            github_id: None,
            deploy_target: None,
        }
    }

    fn seed_pair(dir: &Path, filename: &str) {
        fs::create_dir_all(dir).unwrap();
        let stem = filename.trim_end_matches(".md");
        fs::write(dir.join(filename), "req body").unwrap();
        fs::write(dir.join(format!("{stem}.response.md")), "receipt body").unwrap();
    }

    /// F-000039: seed the full triple (req + response + impl-ack) in a dir.
    fn seed_triple(dir: &Path, filename: &str) {
        seed_pair(dir, filename);
        let stem = filename.trim_end_matches(".md");
        fs::write(dir.join(format!("{stem}.impl.md")), "").unwrap();
    }

    #[test]
    fn test_confirm_pair_client_to_server_deletes_both_sides() {
        let tmp = TempDir::new().unwrap();
        let client_base = tmp.path().join("client");
        let server_base = tmp.path().join("server");
        let client = mk_repo(1, "owner/web-client", "client", &client_base);
        let server = mk_repo(2, "owner/backend", "server", &server_base);

        let client_dir = client_base.join("docs/backend-requirements");
        let server_dir = server_base.join("docs/client-requirements/web-client");
        // F-000039: seed the full triple to confirm .impl.md is torn down too.
        seed_triple(&client_dir, "REQ-001_login.md");
        seed_triple(&server_dir, "REQ-001_login.md");

        confirm_pair(&client, &server, "REQ-001_login.md").unwrap();

        assert!(!client_dir.join("REQ-001_login.md").exists());
        assert!(!client_dir.join("REQ-001_login.response.md").exists());
        assert!(!client_dir.join("REQ-001_login.impl.md").exists());
        assert!(!server_dir.join("REQ-001_login.md").exists());
        assert!(!server_dir.join("REQ-001_login.response.md").exists());
        assert!(!server_dir.join("REQ-001_login.impl.md").exists());
    }

    #[test]
    fn test_confirm_pair_server_to_microservice_deletes_both_sides() {
        let tmp = TempDir::new().unwrap();
        let server_base = tmp.path().join("server");
        let ms_base = tmp.path().join("ms");
        let server = mk_repo(1, "owner/backend", "server", &server_base);
        let ms_server = mk_repo(2, "owner/avatar-ms", "server", &ms_base);

        let server_dir = server_base.join("docs/microservice-requirements/avatar-ms");
        let ms_dir = ms_base.join("docs/server-requirements/backend");
        seed_pair(&server_dir, "REQ-007_blobs.md");
        seed_pair(&ms_dir, "REQ-007_blobs.md");

        confirm_pair(&server, &ms_server, "REQ-007_blobs.md").unwrap();

        assert!(!server_dir.join("REQ-007_blobs.md").exists());
        assert!(!server_dir.join("REQ-007_blobs.response.md").exists());
        assert!(!ms_dir.join("REQ-007_blobs.md").exists());
        assert!(!ms_dir.join("REQ-007_blobs.response.md").exists());
    }

    #[test]
    fn test_confirm_pair_only_deletes_target_specific_paths() {
        // Sibling MS with same NNN should NOT be touched (replicates C1
        // disambiguation invariant from v0.27.1).
        let tmp = TempDir::new().unwrap();
        let server_base = tmp.path().join("server");
        let ms_a_base = tmp.path().join("ms-a");
        let ms_b_base = tmp.path().join("ms-b");
        let server = mk_repo(1, "owner/backend", "server", &server_base);
        let ms_a = mk_repo(2, "owner/avatar-ms", "server", &ms_a_base);
        let _ms_b = mk_repo(3, "owner/notify-ms", "server", &ms_b_base);

        let server_dir_a = server_base.join("docs/microservice-requirements/avatar-ms");
        let server_dir_b = server_base.join("docs/microservice-requirements/notify-ms");
        let ms_a_dir = ms_a_base.join("docs/server-requirements/backend");
        let ms_b_dir = ms_b_base.join("docs/server-requirements/backend");
        seed_pair(&server_dir_a, "REQ-005_x.md");
        seed_pair(&server_dir_b, "REQ-005_x.md");
        seed_pair(&ms_a_dir, "REQ-005_x.md");
        seed_pair(&ms_b_dir, "REQ-005_x.md");

        confirm_pair(&server, &ms_a, "REQ-005_x.md").unwrap();

        // MS-A pair gone
        assert!(!server_dir_a.join("REQ-005_x.md").exists());
        assert!(!ms_a_dir.join("REQ-005_x.md").exists());
        // MS-B pair untouched (sibling NNN collision protected)
        assert!(server_dir_b.join("REQ-005_x.md").exists());
        assert!(server_dir_b.join("REQ-005_x.response.md").exists());
        assert!(ms_b_dir.join("REQ-005_x.md").exists());
        assert!(ms_b_dir.join("REQ-005_x.response.md").exists());
    }

    #[test]
    fn test_confirm_pair_unknown_role_errors() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        let source = mk_repo(1, "owner/tool", "tool", &a);
        let target = mk_repo(2, "owner/other", "server", &b);

        let err = confirm_pair(&source, &target, "REQ-001.md").unwrap_err();
        assert!(err.contains("Unexpected source repo role"), "got: {}", err);
    }

    #[test]
    fn test_confirm_pair_missing_local_path_noop() {
        let tmp = TempDir::new().unwrap();
        let a_path = tmp.path().join("a");
        let mut source = mk_repo(1, "owner/client", "client", &a_path);
        source.local_path = None;
        let target = mk_repo(2, "owner/server", "server", tmp.path());

        // Does not error — silent no-op when either side has no local_path,
        // mirroring previous in-place behavior (paths can't be derived).
        confirm_pair(&source, &target, "REQ-001.md").unwrap();
    }

    // ── F-000039: auto_close_impl_pairs ──────────────────────────────────────

    #[test]
    fn test_auto_close_impl_pairs_tears_down_triple_both_sides() {
        let tmp = TempDir::new().unwrap();
        let sender_dir = tmp.path().join("sender");
        let recipient_dir = tmp.path().join("recipient");
        fs::create_dir_all(&sender_dir).unwrap();
        fs::create_dir_all(&recipient_dir).unwrap();
        // Sender (acknowledged): base + impl-ack. Recipient: base + response.
        fs::write(sender_dir.join("REQ-001_x.md"), "req").unwrap();
        fs::write(sender_dir.join("REQ-001_x.impl.md"), "").unwrap();
        fs::write(recipient_dir.join("REQ-001_x.md"), "req").unwrap();
        fs::write(recipient_dir.join("REQ-001_x.response.md"), "receipt").unwrap();

        let closed = auto_close_impl_pairs(&sender_dir, &recipient_dir).unwrap();
        assert_eq!(closed, vec!["REQ-001_x.md".to_string()]);

        // Everything gone in both dirs.
        assert!(!sender_dir.join("REQ-001_x.md").exists());
        assert!(!sender_dir.join("REQ-001_x.impl.md").exists());
        assert!(!recipient_dir.join("REQ-001_x.md").exists());
        assert!(!recipient_dir.join("REQ-001_x.response.md").exists());

        // Second call is a no-op (idempotent) — impl file already gone.
        let closed_again = auto_close_impl_pairs(&sender_dir, &recipient_dir).unwrap();
        assert!(closed_again.is_empty());
    }

    #[test]
    fn test_auto_close_impl_pairs_untouched_without_impl() {
        let tmp = TempDir::new().unwrap();
        let sender_dir = tmp.path().join("sender");
        let recipient_dir = tmp.path().join("recipient");
        fs::create_dir_all(&sender_dir).unwrap();
        fs::create_dir_all(&recipient_dir).unwrap();
        // A live pair with NO impl-ack — must be left alone.
        fs::write(sender_dir.join("REQ-002_y.md"), "req").unwrap();
        fs::write(recipient_dir.join("REQ-002_y.md"), "req").unwrap();
        fs::write(recipient_dir.join("REQ-002_y.response.md"), "receipt").unwrap();

        let closed = auto_close_impl_pairs(&sender_dir, &recipient_dir).unwrap();
        assert!(closed.is_empty());
        assert!(sender_dir.join("REQ-002_y.md").exists());
        assert!(recipient_dir.join("REQ-002_y.md").exists());
        assert!(recipient_dir.join("REQ-002_y.response.md").exists());
    }

    // ── T-000190: migrate_req_pair legacy -> InOutBox ────────────────────────

    #[test]
    fn test_migrate_req_pair_moves_the_triple_byte_identical() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("docs/backend-requirements");
        let new = tmp.path().join("docs/InOutBox/outbox/backend");
        seed_triple(&legacy, "REQ-001_login.md");
        let body = fs::read(legacy.join("REQ-001_login.md")).unwrap();
        let resp = fs::read(legacy.join("REQ-001_login.response.md")).unwrap();
        let imp = fs::read(legacy.join("REQ-001_login.impl.md")).unwrap();

        let mut w = Vec::new();
        let o = migrate_req_pair(&[(legacy.as_path(), new.as_path())], &mut w).unwrap();
        assert!(o.channel_ready);
        assert_eq!(o.migrated, 3);
        assert!(w.is_empty());
        assert_eq!(fs::read(new.join("REQ-001_login.md")).unwrap(), body);
        assert_eq!(
            fs::read(new.join("REQ-001_login.response.md")).unwrap(),
            resp
        );
        assert_eq!(fs::read(new.join("REQ-001_login.impl.md")).unwrap(), imp);
        // Empty legacy dir is gone.
        assert!(!legacy.exists());
    }

    #[test]
    fn test_migrate_req_pair_keeps_residue_dir_alive() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy");
        let new = tmp.path().join("new");
        seed_pair(&legacy, "REQ-001_x.md");
        // The frozen number ledger and an unknown file are NOT channel files:
        // they stay, and the legacy dir must survive for them.
        fs::write(legacy.join("registry.md"), "- REQ-001 | out | x\n").unwrap();
        fs::write(legacy.join("README.md"), "hi").unwrap();
        // A subdirectory never migrates either.
        fs::create_dir_all(legacy.join("nested")).unwrap();

        let mut w = Vec::new();
        let o = migrate_req_pair(&[(legacy.as_path(), new.as_path())], &mut w).unwrap();
        assert!(o.channel_ready);
        assert_eq!(o.migrated, 2);
        assert!(new.join("REQ-001_x.md").exists());
        assert!(new.join("REQ-001_x.response.md").exists());
        assert!(legacy.join("registry.md").exists());
        assert!(legacy.join("README.md").exists());
        assert!(legacy.is_dir(), "residue keeps the archive dir alive");
    }

    #[test]
    fn test_migrate_req_pair_dedups_identical_target() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy");
        let new = tmp.path().join("new");
        seed_pair(&legacy, "REQ-001_x.md");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("REQ-001_x.md"), "req body").unwrap();
        // Same bytes on both sides: dedup — source deleted, target kept.

        let mut w = Vec::new();
        let o = migrate_req_pair(&[(legacy.as_path(), new.as_path())], &mut w).unwrap();
        assert!(o.channel_ready);
        assert_eq!(o.migrated, 2); // body dedup + response move
        assert!(w.is_empty());
        assert!(new.join("REQ-001_x.md").exists());
        assert!(!legacy.join("REQ-001_x.md").exists());
    }

    #[test]
    fn test_migrate_req_pair_conflict_keeps_both_versions() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy");
        let new = tmp.path().join("new");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("REQ-001_x.md"), "legacy bytes").unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("REQ-001_x.md"), "new bytes").unwrap();

        let mut w = Vec::new();
        let o = migrate_req_pair(&[(legacy.as_path(), new.as_path())], &mut w).unwrap();
        assert!(o.channel_ready);
        assert_eq!(o.migrated, 0, "a diverged pair moves nothing");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].code, crate::models::SyncIssueCode::MigrationDiverged);
        // Both versions survive — no overwrite, no guess.
        assert_eq!(fs::read(new.join("REQ-001_x.md")).unwrap(), b"new bytes");
        assert_eq!(
            fs::read(legacy.join("REQ-001_x.md")).unwrap(),
            b"legacy bytes"
        );
        assert!(legacy.is_dir(), "the diverged file keeps the dir alive");
    }

    #[test]
    fn test_migrate_req_pair_idempotent_second_pass() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join("legacy");
        let new = tmp.path().join("new");
        seed_pair(&legacy, "REQ-001_x.md");

        let mut w = Vec::new();
        migrate_req_pair(&[(legacy.as_path(), new.as_path())], &mut w).unwrap();
        let mut w2 = Vec::new();
        let o2 = migrate_req_pair(&[(legacy.as_path(), new.as_path())], &mut w2).unwrap();
        assert!(o2.channel_ready);
        assert_eq!(o2.migrated, 0);
        assert!(w2.is_empty());
    }

    #[test]
    fn test_migrate_req_pair_rolls_back_earlier_sides_on_failure() {
        let tmp = TempDir::new().unwrap();
        let a_src = tmp.path().join("a-legacy");
        let a_dst = tmp.path().join("a-new");
        let b_src = tmp.path().join("b-legacy");
        // Side B's destination is blocked by a FILE: create_dir_all fails and
        // side A — already applied — must be put back.
        let blocker = tmp.path().join("blocker");
        seed_pair(&a_src, "REQ-001_x.md");
        seed_pair(&b_src, "REQ-002_y.md");
        fs::write(&blocker, "not a dir").unwrap();
        let b_dst = blocker.join("sub");

        let mut w = Vec::new();
        let o = migrate_req_pair(
            &[
                (a_src.as_path(), a_dst.as_path()),
                (b_src.as_path(), b_dst.as_path()),
            ],
            &mut w,
        )
        .unwrap();
        assert!(!o.channel_ready, "the failed pair stays on the old layout");
        // Side A's files are back where they started.
        assert!(a_src.join("REQ-001_x.md").exists());
        assert!(a_src.join("REQ-001_x.response.md").exists());
        assert!(!a_dst.join("REQ-001_x.md").exists());
        // Side B never moved.
        assert!(b_src.join("REQ-002_y.md").exists());
    }

    #[test]
    fn test_migrate_req_pair_cross_populates_a_one_sided_body() {
        // The sender lost its own copy of the body long ago — only an orphan
        // response receipt remains; the recipient still holds the whole pair.
        // Without the cross copy the body lands in `inbox/` alone and the
        // mirror prunes the masterless replica on the very next pass.
        let tmp = TempDir::new().unwrap();
        let sender_legacy = tmp.path().join("client/docs/backend-requirements");
        let sender_out = tmp.path().join("client/docs/InOutBox/outbox/backend");
        let recipient_legacy = tmp
            .path()
            .join("server/docs/client-requirements/web-client");
        let recipient_in = tmp.path().join("server/docs/InOutBox/inbox/web-client");
        fs::create_dir_all(&sender_legacy).unwrap();
        fs::write(sender_legacy.join("REQ-159_x.response.md"), "receipt body").unwrap();
        seed_pair(&recipient_legacy, "REQ-159_x.md");
        let body = fs::read(recipient_legacy.join("REQ-159_x.md")).unwrap();

        let mut w = Vec::new();
        let o = migrate_req_pair(
            &[
                (sender_legacy.as_path(), sender_out.as_path()),
                (recipient_legacy.as_path(), recipient_in.as_path()),
            ],
            &mut w,
        )
        .unwrap();
        assert!(o.channel_ready);
        assert!(w.is_empty(), "{:?}", w);
        // The body reaches the sender's outbox — the master the replica needs.
        assert_eq!(fs::read(sender_out.join("REQ-159_x.md")).unwrap(), body);
        assert_eq!(fs::read(recipient_in.join("REQ-159_x.md")).unwrap(), body);
        // The response ends up on both sides too — the recipient-authored
        // copy in the inbox plus the carried-back replica in the outbox.
        assert!(sender_out.join("REQ-159_x.response.md").exists());
        assert!(recipient_in.join("REQ-159_x.response.md").exists());
    }

    #[test]
    fn test_migrate_req_pair_cross_dedups_a_two_sided_pair() {
        // Both sides hold the same pair — nothing crosses, the side plans own
        // every name.
        let tmp = TempDir::new().unwrap();
        let a_src = tmp.path().join("a-legacy");
        let a_dst = tmp.path().join("a-new");
        let b_src = tmp.path().join("b-legacy");
        let b_dst = tmp.path().join("b-new");
        seed_pair(&a_src, "REQ-001_x.md");
        seed_pair(&b_src, "REQ-001_x.md");

        let mut w = Vec::new();
        let o = migrate_req_pair(
            &[
                (a_src.as_path(), a_dst.as_path()),
                (b_src.as_path(), b_dst.as_path()),
            ],
            &mut w,
        )
        .unwrap();
        assert!(o.channel_ready);
        assert_eq!(o.migrated, 4, "two moves per side, no extra copies");
        assert!(w.is_empty(), "{:?}", w);
        assert!(a_dst.join("REQ-001_x.md").exists());
        assert!(b_dst.join("REQ-001_x.md").exists());
    }

    #[test]
    fn test_merge_channel_dir_merges_a_collision_leftover_recursively() {
        // A rename COLLISION leaves `outbox/<old>` beside `outbox/<new>` — the
        // live dir never empties, so the old one's files merge in the same
        // pass: moves land, byte-identical targets dedup, divergent names
        // keep BOTH and report, nested `.ack/` markers come along.
        let tmp = TempDir::new().unwrap();
        let old = tmp.path().join("docs/InOutBox/outbox/old-name");
        let new = tmp.path().join("docs/InOutBox/outbox/new-name");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("NOTE-001_x.md"), "note").unwrap();
        fs::write(old.join("NOTE-002_same.md"), "same").unwrap();
        fs::write(new.join("NOTE-002_same.md"), "same").unwrap();
        fs::write(old.join("NOTE-003_diff.md"), "old bytes").unwrap();
        fs::write(new.join("NOTE-003_diff.md"), "new bytes").unwrap();
        // A nested ack marker rides the merge too.
        fs::create_dir_all(old.join("ANNOUNCE-001_a.md.ack")).unwrap();
        fs::write(old.join("ANNOUNCE-001_a.md.ack/helper"), "").unwrap();

        let mut w = Vec::new();
        let n = merge_channel_dir(&old, &new, &mut w).unwrap();
        assert_eq!(n, 3, "1 move + 1 dedup + 1 nested move");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].code, crate::models::SyncIssueCode::MigrationDiverged);
        assert_eq!(fs::read(new.join("NOTE-001_x.md")).unwrap(), b"note");
        assert!(new.join("ANNOUNCE-001_a.md.ack/helper").is_file());
        // Divergent pair keeps both versions.
        assert_eq!(
            fs::read(new.join("NOTE-003_diff.md")).unwrap(),
            b"new bytes"
        );
        assert_eq!(
            fs::read(old.join("NOTE-003_diff.md")).unwrap(),
            b"old bytes"
        );
        // Everything else merged away — only the frozen file keeps the dir.
        assert!(old.is_dir());
        assert!(!old.join("NOTE-002_same.md").exists());
        assert!(!old.join("ANNOUNCE-001_a.md.ack").exists());
    }

    #[test]
    fn test_confirm_pair_on_the_inoutbox_layout() {
        let tmp = TempDir::new().unwrap();
        let client_base = tmp.path().join("client");
        let server_base = tmp.path().join("server");
        let client = mk_repo(1, "owner/web-client", "client", &client_base);
        let server = mk_repo(2, "owner/backend", "server", &server_base);

        // T-000190 paths: sender outbox/<server>, recipient inbox/<client>.
        let out = crate::sync::outbox_dir(&client_base, "backend");
        let inb = crate::sync::inbox_dir(&server_base, "web-client");
        seed_triple(&out, "REQ-002_y.md");
        seed_triple(&inb, "REQ-002_y.md");

        confirm_pair(&client, &server, "REQ-002_y.md").unwrap();
        for d in [&out, &inb] {
            assert!(!d.join("REQ-002_y.md").exists());
            assert!(!d.join("REQ-002_y.response.md").exists());
            assert!(!d.join("REQ-002_y.impl.md").exists());
        }
    }

    #[test]
    fn test_confirm_pair_spans_migration_residue_and_new_layout() {
        // A pair that half-migrated (body moved, response still legacy) still
        // closes cleanly — every candidate dir is swept.
        let tmp = TempDir::new().unwrap();
        let client_base = tmp.path().join("client");
        let server_base = tmp.path().join("server");
        let client = mk_repo(1, "owner/web-client", "client", &client_base);
        let server = mk_repo(2, "owner/backend", "server", &server_base);

        let out = crate::sync::outbox_dir(&client_base, "backend");
        let legacy_sender = client_base.join("docs/backend-requirements");
        let inb = crate::sync::inbox_dir(&server_base, "web-client");
        let legacy_recipient = server_base.join("docs/client-requirements/web-client");
        seed_pair(&out, "REQ-003_z.md");
        fs::create_dir_all(&legacy_sender).unwrap();
        fs::write(legacy_sender.join("REQ-003_z.impl.md"), "").unwrap();
        seed_pair(&inb, "REQ-003_z.md");
        fs::create_dir_all(&legacy_recipient).unwrap();
        fs::write(legacy_recipient.join("REQ-003_z.md"), "leftover").unwrap();

        confirm_pair(&client, &server, "REQ-003_z.md").unwrap();
        for d in [&out, &legacy_sender, &inb, &legacy_recipient] {
            for f in ["REQ-003_z.md", "REQ-003_z.response.md", "REQ-003_z.impl.md"] {
                assert!(!d.join(f).exists(), "{} still holds {}", d.display(), f);
            }
        }
    }

    // ── T-000245: migrate_announce_dir legacy -> InOutBox ────────────────────

    fn announce_dirs(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let legacy = tmp.join("docs/server-announcements/backend");
        let inbox = tmp.join("docs/InOutBox/inbox/backend");
        let outbox = tmp.join("docs/InOutBox/outbox/web-client");
        (legacy, inbox, outbox)
    }

    #[test]
    fn test_migrate_announce_dir_copies_to_inbox_and_outbox_byte_identical() {
        let tmp = TempDir::new().unwrap();
        let (legacy, inbox, outbox) = announce_dirs(tmp.path());
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("ANNOUNCE-001_new.md"), "b\u{fffd}ytes").unwrap();
        let body = fs::read(legacy.join("ANNOUNCE-001_new.md")).unwrap();

        let mut w = Vec::new();
        let n = migrate_announce_dir(&legacy, &inbox, &outbox, false, &Default::default(), &mut w)
            .unwrap();
        assert_eq!(n, 1);
        assert!(w.is_empty());
        // Both halves exist — the mirror would prune an inbox-only orphan.
        assert_eq!(fs::read(inbox.join("ANNOUNCE-001_new.md")).unwrap(), body);
        assert_eq!(fs::read(outbox.join("ANNOUNCE-001_new.md")).unwrap(), body);
        // Non-shared: the legacy file retires with the migration.
        assert!(!legacy.exists(), "empty legacy dir is dropped");
    }

    #[test]
    fn test_migrate_announce_dir_keep_legacy_for_a_shared_recipient() {
        let tmp = TempDir::new().unwrap();
        let (legacy, inbox, outbox) = announce_dirs(tmp.path());
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("ANNOUNCE-002_x.md"), "news").unwrap();

        let mut w = Vec::new();
        let n = migrate_announce_dir(&legacy, &inbox, &outbox, true, &Default::default(), &mut w)
            .unwrap();
        assert_eq!(n, 1);
        // The projection stays — an older agent on the shared branch still
        // reads announcements from here.
        assert!(legacy.join("ANNOUNCE-002_x.md").exists());
        assert!(inbox.join("ANNOUNCE-002_x.md").exists());
        assert!(outbox.join("ANNOUNCE-002_x.md").exists());
    }

    #[test]
    fn test_migrate_announce_dir_dedups_and_diverges() {
        let tmp = TempDir::new().unwrap();
        let (legacy, inbox, outbox) = announce_dirs(tmp.path());
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&inbox).unwrap();
        fs::write(legacy.join("ANNOUNCE-003_same.md"), "same").unwrap();
        fs::write(inbox.join("ANNOUNCE-003_same.md"), "same").unwrap();
        fs::write(legacy.join("ANNOUNCE-004_diff.md"), "legacy").unwrap();
        fs::write(inbox.join("ANNOUNCE-004_diff.md"), "different").unwrap();

        let mut w = Vec::new();
        let n = migrate_announce_dir(&legacy, &inbox, &outbox, false, &Default::default(), &mut w)
            .unwrap();
        assert_eq!(n, 1, "only the dedup'd file migrates");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].code, crate::models::SyncIssueCode::MigrationDiverged);
        // Diverged pair keeps both versions.
        assert_eq!(
            fs::read(inbox.join("ANNOUNCE-004_diff.md")).unwrap(),
            b"different"
        );
        assert_eq!(
            fs::read(legacy.join("ANNOUNCE-004_diff.md")).unwrap(),
            b"legacy"
        );
        assert!(legacy.is_dir(), "the diverged file keeps the dir alive");
    }

    #[test]
    fn test_migrate_announce_dir_never_reseeds_a_closed_announcement() {
        let tmp = TempDir::new().unwrap();
        let (legacy, inbox, outbox) = announce_dirs(tmp.path());
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("ANNOUNCE-005_done.md"), "old news").unwrap();
        let closed: std::collections::BTreeSet<String> =
            ["ANNOUNCE-005_done.md".to_string()].into_iter().collect();

        let mut w = Vec::new();
        let n = migrate_announce_dir(&legacy, &inbox, &outbox, true, &closed, &mut w).unwrap();
        assert_eq!(n, 0, "a closed name must not spawn fresh copies");
        assert!(!inbox.join("ANNOUNCE-005_done.md").exists());
        assert!(!outbox.join("ANNOUNCE-005_done.md").exists());
        // The leftover projection itself is left for the retire pass.
        assert!(legacy.join("ANNOUNCE-005_done.md").exists());
    }

    #[test]
    fn test_migrate_announce_dir_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let (legacy, inbox, outbox) = announce_dirs(tmp.path());
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("ANNOUNCE-006_i.md"), "x").unwrap();
        let mut w = Vec::new();
        migrate_announce_dir(&legacy, &inbox, &outbox, false, &Default::default(), &mut w).unwrap();
        let mut w2 = Vec::new();
        let n2 = migrate_announce_dir(
            &legacy,
            &inbox,
            &outbox,
            false,
            &Default::default(),
            &mut w2,
        )
        .unwrap();
        assert_eq!(n2, 0);
        assert!(w2.is_empty());
    }
}
