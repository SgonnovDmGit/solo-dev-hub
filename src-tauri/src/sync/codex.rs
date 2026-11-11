// Codex broadcast: mirror a codex project's per-lane svod folders into each
// connected consumer's same-role repo, read-only. Role-matched, best-effort.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::db::AppDb;
use crate::models::{lookup, SyncIssue, SyncIssueCode as Code};
use crate::sync;

/// Aggregated result of broadcasting one codex into one consumer.
#[derive(Debug, Default)]
pub struct CodexBroadcastOutcome {
    /// Files written across all lanes (sum of MirrorOutcome.copied).
    pub copied: usize,
    /// Files/dirs pruned from consumer mirrors (sum of MirrorOutcome.pruned).
    pub pruned: usize,
    /// Non-fatal per-lane failures surfaced to the user.
    pub errors: Vec<SyncIssue>,
}

/// The two audience lanes a codex can publish on.
const CODEX_LANES: [&str; 2] = ["server", "admin_client"];

/// Broadcast `codex_project_id`'s svod into `consumer_project_id`, one lane at a
/// time. For each lane R in CODEX_LANES:
///   - If the codex has NO repo of role R → the codex simply doesn't publish that
///     lane: silent skip (not an error).
///   - If the codex publishes lane R but the consumer has no repo of role R
///     (or it has no local_path) → skip + error (role-matched delivery expected).
///   - Otherwise mirror `<codex-lane-repo>/<codex_source_path|practices>/` →
///     `<consumer-lane-repo>/docs/codex/<codex-name>/` via `sync::mirror_dir`.
///
/// A missing codex source folder surfaces as an error (mirror_dir errs, never
/// wipes the consumer). `<codex-name>` is the codex project's `name`.
pub fn broadcast_codex_to_consumer(
    db: &AppDb,
    codex_project_id: i64,
    consumer_project_id: i64,
) -> CodexBroadcastOutcome {
    let mut outcome = CodexBroadcastOutcome::default();

    // Resolve the codex project name once — it names the consumer inbox folder.
    let codex_name = match db.get_project(codex_project_id) {
        Ok(p) => p.name,
        Err(e) => {
            outcome.errors.push(
                SyncIssue::lookup(lookup::PROJECT, format!("#{}", codex_project_id)).detail(e),
            );
            return outcome;
        }
    };

    for role in CODEX_LANES {
        // Does the codex publish this lane? No repo of role R → silent skip.
        let codex_repo = match db.repo_of_project_with_role(codex_project_id, role) {
            Ok(r) => r,
            Err(_) => continue,
        };

        // The codex publishes this lane — its source folder must exist on disk.
        let Some(codex_local) = codex_repo.local_path.as_deref() else {
            outcome
                .errors
                .push(SyncIssue::new(Code::NoLocalPath, codex_repo.display_name()));
            continue;
        };
        let codex_base = Path::new(codex_local);
        if sync::ensure_root_exists(codex_base).is_err() {
            outcome.errors.push(
                SyncIssue::new(Code::RootMissing, codex_repo.display_name()).detail(codex_local),
            );
            continue;
        }

        // Role-matched delivery: the consumer must have a repo of the same role.
        let consumer_repo = match db.repo_of_project_with_role(consumer_project_id, role) {
            Ok(r) => r,
            Err(_) => {
                let consumer = db
                    .get_project(consumer_project_id)
                    .map(|p| p.name)
                    .unwrap_or_else(|_| format!("#{}", consumer_project_id));
                outcome
                    .errors
                    .push(SyncIssue::new(Code::MissingCounterpart, consumer).target(role));
                continue;
            }
        };
        let Some(consumer_local) = consumer_repo.local_path.as_deref() else {
            outcome.errors.push(SyncIssue::new(
                Code::NoLocalPath,
                consumer_repo.display_name(),
            ));
            continue;
        };
        let consumer_base = Path::new(consumer_local);
        if sync::ensure_root_exists(consumer_base).is_err() {
            outcome.errors.push(
                SyncIssue::new(Code::RootMissing, consumer_repo.display_name())
                    .detail(consumer_local),
            );
            continue;
        }

        let source_path = match db.codex_source_path_for_repo(codex_repo.id) {
            Ok(p) => p,
            Err(e) => {
                outcome.errors.push(
                    SyncIssue::lookup(lookup::CODEX_SOURCE, codex_repo.display_name()).detail(e),
                );
                continue;
            }
        };

        let src = codex_base.join(&source_path);
        let dst = consumer_base.join("docs").join("codex").join(&codex_name);
        match sync::mirror_dir(&src, &dst) {
            Ok(o) => {
                outcome.copied += o.copied;
                outcome.pruned += o.pruned;
            }
            Err(e) => outcome.errors.push(
                SyncIssue::new(Code::CopyFailed, format!("docs/codex/{}/", codex_name))
                    .target(consumer_repo.display_name())
                    .detail(e),
            ),
        }
    }

    outcome
}

/// T-000247: the consumer's codex inbox root — the folder the mirrors live in.
pub fn codex_inbox_root(consumer_root: &Path) -> std::path::PathBuf {
    consumer_root.join("docs").join("codex")
}

/// T-000247: the FULL expected-name set of one consumer's `docs/codex/` root —
/// the names of every codex connected to `consumer_project_id`, whether or not
/// its source/lane is currently available. Any lookup failure is fatal to the
/// whole set: a partial answer is exactly what a sweep must never trust.
pub fn connected_codex_names(
    db: &AppDb,
    consumer_project_id: i64,
) -> Result<BTreeSet<String>, String> {
    let ids = db
        .list_connected_codices(consumer_project_id)
        .map_err(|e| e.to_string())?;
    let mut names = BTreeSet::new();
    for id in ids {
        names.insert(db.get_project(id).map_err(|e| e.to_string())?.name);
    }
    Ok(names)
}

/// A codex name is used as a single path component — anything else (empty,
/// `.`/`..`, a path separator) is a DB anomaly the sweep refuses to act on.
fn valid_inbox_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && Path::new(name).components().count() == 1
}

/// T-000247: delete top-level entries of `<consumer_root>/docs/codex/` that are
/// not in `expected` — the root belongs to the app's mirrors alone, so stray
/// files (a script run from the wrong cwd, a dropped snapshot) must not ride
/// the shared pathspec to a contributor. Returns the number of removed entries.
///
/// Safety contract: the root (and its `docs` parent) must not be a
/// symlink/junction — a misdirected inbox is refused, never followed. Expected
/// names are matched case-insensitively (a case-only rename on Windows must
/// not delete a fresh mirror). An empty or invalid set refuses: "unknown"
/// never reads as "empty inbox". A missing root is a no-op — the sweep never
/// creates the folder just to clean it.
pub fn sweep_codex_inbox(
    consumer_root: &Path,
    expected: &BTreeSet<String>,
) -> Result<usize, String> {
    let inbox = codex_inbox_root(consumer_root);
    if !inbox.exists() {
        return Ok(0);
    }
    if expected.is_empty() {
        return Err("refusing to sweep docs/codex/ with an empty expected-name set".to_string());
    }
    for name in expected {
        if !valid_inbox_name(name) {
            return Err(format!(
                "codex name {:?} is not a valid path component — refusing to sweep",
                name
            ));
        }
    }
    // The root and its `docs` parent must be real directories — a link here
    // would point the mirror elsewhere, and deleting through it is a write
    // outside the contract.
    for dir in [inbox.as_path(), inbox.parent().unwrap_or(inbox.as_path())] {
        let meta = fs::symlink_metadata(dir)
            .map_err(|e| format!("could not inspect {}: {}", dir.display(), e))?;
        if meta.file_type().is_symlink() {
            return Err(format!(
                "{} is a symlink/junction — refusing to sweep",
                dir.display()
            ));
        }
        if !meta.is_dir() {
            return Err(format!(
                "{} is not a directory — refusing to sweep",
                dir.display()
            ));
        }
    }
    // Windows is the primary platform and its FS is case-insensitive: a
    // name that differs only by case IS the expected mirror.
    let expected_lower: BTreeSet<String> = expected.iter().map(|n| n.to_lowercase()).collect();
    let mut pruned = 0usize;
    let entries =
        fs::read_dir(&inbox).map_err(|e| format!("could not list {}: {}", inbox.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("could not read {}: {}", inbox.display(), e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if expected_lower.contains(&name.to_lowercase()) {
            continue;
        }
        let path = entry.path();
        let ft = entry
            .file_type()
            .map_err(|e| format!("could not stat {}: {}", path.display(), e))?;
        let res = if ft.is_symlink() {
            // A link entry is removed itself, never followed: file links go
            // via remove_file, directory links/junctions need remove_dir.
            fs::remove_file(&path).or_else(|_| fs::remove_dir(&path))
        } else if ft.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        res.map_err(|e| format!("could not remove {}: {}", path.display(), e))?;
        pruned += 1;
    }
    Ok(pruned)
}

/// T-000247 D4b: the outcome of preparing a consumer's `docs/codex/` root for
/// publishing — `safe` says the folder may ride the shared pathspec this pass;
/// `reason` carries why not (the caller excludes `docs/codex` from the commit
/// paths and reports it, the rest of the shared set is unaffected).
#[derive(Debug, Default)]
pub struct CodexRootPrep {
    pub safe: bool,
    pub pruned: usize,
    pub reason: Option<String>,
}

impl CodexRootPrep {
    fn unsafe_(reason: impl Into<String>) -> Self {
        CodexRootPrep {
            safe: false,
            pruned: 0,
            reason: Some(reason.into()),
        }
    }

    /// Prepend caller-held context (e.g. the name-set resolution error) to the
    /// refusal reason — a `safe` verdict has no reason to prefix.
    pub fn with_reason_prefix(mut self, prefix: impl Into<String>) -> Self {
        if let Some(r) = &mut self.reason {
            *r = format!("{}{}", prefix.into(), r);
        }
        self
    }
}

/// Prepare `consumer_root`'s codex inbox for publishing. `expected` is the
/// consumer's resolved connection set (`None` = the project has no resolvable
/// connection set — an unassigned repo, or the DB refused it upstream). With
/// an empty/unknown set the function is only a verdict: an existing non-empty
/// inbox cannot be proven safe and is refused; an absent or empty one is a
/// no-op. With a non-empty set the root is swept and every expected name is
/// checked to be a mirror directory, not a file.
pub fn prepare_codex_root(
    consumer_root: &Path,
    expected: Option<&BTreeSet<String>>,
) -> CodexRootPrep {
    let inbox = codex_inbox_root(consumer_root);
    let inbox_has_entries = fs::read_dir(&inbox)
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    match expected {
        None => {
            if inbox_has_entries {
                CodexRootPrep::unsafe_(
                    "docs/codex/ holds files but the project's connected codices are unknown; \
                     not publishing unverified content",
                )
            } else {
                CodexRootPrep {
                    safe: true,
                    ..Default::default()
                }
            }
        }
        Some(set) if set.is_empty() => {
            if inbox_has_entries {
                CodexRootPrep::unsafe_(
                    "docs/codex/ holds files but no connected codex is known locally; \
                     not publishing unverified content",
                )
            } else {
                CodexRootPrep {
                    safe: true,
                    ..Default::default()
                }
            }
        }
        Some(set) => match sweep_codex_inbox(consumer_root, set) {
            Err(e) => CodexRootPrep::unsafe_(e),
            Ok(pruned) => {
                for name in set {
                    let p = inbox.join(name);
                    match fs::symlink_metadata(&p) {
                        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
                        Ok(_) => {
                            return CodexRootPrep::unsafe_(format!(
                                "docs/codex/{} exists but is not a mirror directory",
                                name
                            ))
                        }
                        Err(_) => {} // absent mirror is fine — a lane may not deliver it
                    }
                }
                CodexRootPrep {
                    safe: true,
                    pruned,
                    reason: None,
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn make_db() -> AppDb {
        AppDb::new(PathBuf::from(":memory:")).unwrap()
    }

    /// Create a `practices/` folder under `repo_root` holding one nested tree:
    /// intro.md at the top, guide/style.md one level down.
    fn seed_practices(repo_root: &Path, source: &str) {
        let src = repo_root.join(source);
        fs::create_dir_all(src.join("guide")).unwrap();
        fs::write(src.join("intro.md"), "intro").unwrap();
        fs::write(src.join("guide/style.md"), "style").unwrap();
    }

    #[test]
    fn test_broadcast_both_lanes_happy_path() {
        let db = make_db();
        let tmp = TempDir::new().unwrap();

        let codex = db.create_project("MyCodex", None, "codex").unwrap();
        let consumer = db.create_project("Consumer", None, "standard").unwrap();

        // Codex publishes both lanes, each with a practices tree on disk.
        let cx_srv = tmp.path().join("codex-server");
        let cx_admin = tmp.path().join("codex-admin");
        seed_practices(&cx_srv, "practices");
        seed_practices(&cx_admin, "practices");
        db.insert_local_repository(
            cx_srv.to_str().unwrap(),
            "codex-server",
            Some(codex.id),
            Some("server"),
        )
        .unwrap();
        db.insert_local_repository(
            cx_admin.to_str().unwrap(),
            "codex-admin",
            Some(codex.id),
            Some("admin_client"),
        )
        .unwrap();

        // Consumer has both same-role repos.
        let co_srv = tmp.path().join("consumer-server");
        let co_admin = tmp.path().join("consumer-admin");
        fs::create_dir_all(&co_srv).unwrap();
        fs::create_dir_all(&co_admin).unwrap();
        db.insert_local_repository(
            co_srv.to_str().unwrap(),
            "consumer-server",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();
        db.insert_local_repository(
            co_admin.to_str().unwrap(),
            "consumer-admin",
            Some(consumer.id),
            Some("admin_client"),
        )
        .unwrap();

        let out = broadcast_codex_to_consumer(&db, codex.id, consumer.id);

        assert!(out.errors.is_empty(), "unexpected errors: {:?}", out.errors);
        assert_eq!(out.copied, 4, "two files per lane × two lanes");
        assert_eq!(out.pruned, 0);
        // Both consumer inboxes mirror the practices tree under docs/codex/<name>.
        for base in [&co_srv, &co_admin] {
            let inbox = base.join("docs/codex/MyCodex");
            assert_eq!(fs::read_to_string(inbox.join("intro.md")).unwrap(), "intro");
            assert_eq!(
                fs::read_to_string(inbox.join("guide/style.md")).unwrap(),
                "style"
            );
        }
    }

    #[test]
    fn test_broadcast_role_matched_miss_surfaces_error() {
        let db = make_db();
        let tmp = TempDir::new().unwrap();

        let codex = db.create_project("MyCodex", None, "codex").unwrap();
        let consumer = db.create_project("Consumer", None, "standard").unwrap();

        // Codex publishes BOTH lanes.
        let cx_srv = tmp.path().join("codex-server");
        let cx_admin = tmp.path().join("codex-admin");
        seed_practices(&cx_srv, "practices");
        seed_practices(&cx_admin, "practices");
        db.insert_local_repository(
            cx_srv.to_str().unwrap(),
            "codex-server",
            Some(codex.id),
            Some("server"),
        )
        .unwrap();
        db.insert_local_repository(
            cx_admin.to_str().unwrap(),
            "codex-admin",
            Some(codex.id),
            Some("admin_client"),
        )
        .unwrap();

        // Consumer has ONLY a server repo — the admin lane has no target.
        let co_srv = tmp.path().join("consumer-server");
        fs::create_dir_all(&co_srv).unwrap();
        db.insert_local_repository(
            co_srv.to_str().unwrap(),
            "consumer-server",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();

        let out = broadcast_codex_to_consumer(&db, codex.id, consumer.id);

        // Server lane delivered.
        assert_eq!(out.copied, 2);
        let srv_inbox = co_srv.join("docs/codex/MyCodex");
        assert!(srv_inbox.join("intro.md").exists());
        // Admin lane produced exactly one role-miss error, no inbox created.
        assert_eq!(out.errors.len(), 1);
        assert_eq!(
            out.errors[0].code,
            Code::MissingCounterpart,
            "unexpected error: {:?}",
            out.errors
        );
        assert_eq!(out.errors[0].target.as_deref(), Some("admin_client"));
    }

    #[test]
    fn test_broadcast_codex_publishes_one_lane_silent_skip() {
        let db = make_db();
        let tmp = TempDir::new().unwrap();

        let codex = db.create_project("MyCodex", None, "codex").unwrap();
        let consumer = db.create_project("Consumer", None, "standard").unwrap();

        // Codex has ONLY a server repo — it doesn't publish the admin lane.
        let cx_srv = tmp.path().join("codex-server");
        seed_practices(&cx_srv, "practices");
        db.insert_local_repository(
            cx_srv.to_str().unwrap(),
            "codex-server",
            Some(codex.id),
            Some("server"),
        )
        .unwrap();

        // Consumer has BOTH repos.
        let co_srv = tmp.path().join("consumer-server");
        let co_admin = tmp.path().join("consumer-admin");
        fs::create_dir_all(&co_srv).unwrap();
        fs::create_dir_all(&co_admin).unwrap();
        db.insert_local_repository(
            co_srv.to_str().unwrap(),
            "consumer-server",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();
        db.insert_local_repository(
            co_admin.to_str().unwrap(),
            "consumer-admin",
            Some(consumer.id),
            Some("admin_client"),
        )
        .unwrap();

        let out = broadcast_codex_to_consumer(&db, codex.id, consumer.id);

        // Only the server mirror happens; the missing codex admin lane is a
        // silent skip, NOT an error.
        assert_eq!(out.copied, 2);
        assert!(out.errors.is_empty(), "unexpected errors: {:?}", out.errors);
        assert!(co_srv.join("docs/codex/MyCodex/intro.md").exists());
        assert!(
            !co_admin.join("docs/codex/MyCodex").exists(),
            "no admin inbox should be created"
        );
    }

    #[test]
    fn test_broadcast_custom_source_path() {
        let db = make_db();
        let tmp = TempDir::new().unwrap();

        let codex = db.create_project("MyCodex", None, "codex").unwrap();
        let consumer = db.create_project("Consumer", None, "standard").unwrap();

        // Codex server lane broadcasts from a non-default folder.
        let cx_srv = tmp.path().join("codex-server");
        seed_practices(&cx_srv, "docs/practices");
        let cx_repo = db
            .insert_local_repository(
                cx_srv.to_str().unwrap(),
                "codex-server",
                Some(codex.id),
                Some("server"),
            )
            .unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE repositories SET codex_source_path = 'docs/practices' WHERE id = ?1",
                rusqlite::params![cx_repo.id],
            )
            .unwrap();
        }

        let co_srv = tmp.path().join("consumer-server");
        fs::create_dir_all(&co_srv).unwrap();
        db.insert_local_repository(
            co_srv.to_str().unwrap(),
            "consumer-server",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();

        let out = broadcast_codex_to_consumer(&db, codex.id, consumer.id);

        assert!(out.errors.is_empty(), "unexpected errors: {:?}", out.errors);
        assert_eq!(out.copied, 2);
        let inbox = co_srv.join("docs/codex/MyCodex");
        assert_eq!(fs::read_to_string(inbox.join("intro.md")).unwrap(), "intro");
    }

    #[test]
    fn test_broadcast_prune_propagation() {
        let db = make_db();
        let tmp = TempDir::new().unwrap();

        let codex = db.create_project("MyCodex", None, "codex").unwrap();
        let consumer = db.create_project("Consumer", None, "standard").unwrap();

        let cx_srv = tmp.path().join("codex-server");
        seed_practices(&cx_srv, "practices");
        db.insert_local_repository(
            cx_srv.to_str().unwrap(),
            "codex-server",
            Some(codex.id),
            Some("server"),
        )
        .unwrap();

        let co_srv = tmp.path().join("consumer-server");
        fs::create_dir_all(&co_srv).unwrap();
        db.insert_local_repository(
            co_srv.to_str().unwrap(),
            "consumer-server",
            Some(consumer.id),
            Some("server"),
        )
        .unwrap();

        // Initial broadcast delivers both files.
        let first = broadcast_codex_to_consumer(&db, codex.id, consumer.id);
        assert_eq!(first.copied, 2);
        let inbox = co_srv.join("docs/codex/MyCodex");
        assert!(inbox.join("intro.md").exists());

        // Delete a file from the codex source, re-broadcast → it is pruned.
        fs::remove_file(cx_srv.join("practices/intro.md")).unwrap();
        let second = broadcast_codex_to_consumer(&db, codex.id, consumer.id);
        assert_eq!(second.copied, 0);
        assert_eq!(second.pruned, 1);
        assert!(!inbox.join("intro.md").exists(), "deleted file pruned");
        assert!(inbox.join("guide/style.md").exists(), "survivor remains");
    }

    // ----- T-000247: inbox root sweep + publish preparation -------------------

    fn names(list: &[&str]) -> std::collections::BTreeSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sweep_removes_strays_and_keeps_expected_mirrors() {
        let tmp = TempDir::new().unwrap();
        let inbox = tmp.path().join("docs/codex");
        fs::create_dir_all(inbox.join("Swan Practise")).unwrap();
        fs::write(inbox.join("Swan Practise/rule.md"), "r").unwrap();
        fs::create_dir_all(inbox.join("Other Codex")).unwrap();
        fs::write(inbox.join("release.ps1"), "junk").unwrap();
        fs::create_dir_all(inbox.join("stray-dir")).unwrap();

        let pruned =
            sweep_codex_inbox(tmp.path(), &names(&["Swan Practise", "Other Codex"])).unwrap();
        assert_eq!(pruned, 2);
        assert!(inbox.join("Swan Practise/rule.md").exists());
        assert!(inbox.join("Other Codex").is_dir());
        assert!(!inbox.join("release.ps1").exists());
        assert!(!inbox.join("stray-dir").exists());
    }

    #[test]
    fn sweep_keeps_a_case_only_renamed_mirror() {
        // The mirror on disk carries a different case than the project name —
        // on Windows that is the same entry, deleting it would be data loss.
        let tmp = TempDir::new().unwrap();
        let inbox = tmp.path().join("docs/codex");
        fs::create_dir_all(inbox.join("Swan Practise")).unwrap();
        fs::write(inbox.join("Swan Practise/rule.md"), "r").unwrap();

        let pruned = sweep_codex_inbox(tmp.path(), &names(&["swan practise"])).unwrap();
        assert_eq!(pruned, 0);
        assert!(inbox.join("Swan Practise/rule.md").exists());
    }

    #[test]
    fn sweep_refuses_an_empty_or_invalid_name_set() {
        let tmp = TempDir::new().unwrap();
        let inbox = tmp.path().join("docs/codex");
        fs::create_dir_all(inbox.join("stray")).unwrap();

        let empty = std::collections::BTreeSet::new();
        assert!(sweep_codex_inbox(tmp.path(), &empty).is_err());
        assert!(
            inbox.join("stray").exists(),
            "refused sweep must not delete"
        );

        for bad in ["", ".", "..", "a/b", "a\\b"] {
            assert!(
                sweep_codex_inbox(tmp.path(), &names(&[bad])).is_err(),
                "name {:?} must be refused",
                bad
            );
        }
        assert!(inbox.join("stray").exists());
    }

    #[test]
    fn sweep_is_a_noop_without_an_inbox() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            sweep_codex_inbox(tmp.path(), &names(&["Swan Practise"])).unwrap(),
            0
        );
        // The root is never created just to clean it.
        assert!(!tmp.path().join("docs/codex").exists());
    }

    #[cfg(unix)]
    #[test]
    fn sweep_refuses_a_symlink_root() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        fs::create_dir_all(tmp.path().join("docs")).unwrap();
        std::os::unix::fs::symlink(&real, tmp.path().join("docs/codex")).unwrap();

        let err = sweep_codex_inbox(tmp.path(), &names(&["Swan Practise"])).unwrap_err();
        assert!(err.contains("symlink"), "{}", err);
        assert!(real.is_dir(), "the link target must not be touched");
    }

    #[cfg(unix)]
    #[test]
    fn sweep_removes_a_stray_link_without_following_it() {
        let tmp = TempDir::new().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.md"), "k").unwrap();
        let inbox = tmp.path().join("docs/codex");
        fs::create_dir_all(&inbox).unwrap();
        std::os::unix::fs::symlink(&outside, inbox.join("bad-link")).unwrap();

        let pruned = sweep_codex_inbox(tmp.path(), &names(&["Swan Practise"])).unwrap();
        assert_eq!(pruned, 1);
        assert!(!inbox.join("bad-link").exists());
        assert!(outside.join("keep.md").exists(), "the target must survive");
    }

    #[test]
    fn prep_without_a_connection_set_is_safe_only_when_empty() {
        let tmp = TempDir::new().unwrap();
        // No inbox at all: nothing to publish, nothing to fear.
        let prep = prepare_codex_root(tmp.path(), None);
        assert!(prep.safe);

        // An inbox with files but no resolvable connection set is unverifiable.
        fs::create_dir_all(tmp.path().join("docs/codex/stray")).unwrap();
        let prep = prepare_codex_root(tmp.path(), None);
        assert!(!prep.safe);
        assert!(prep.reason.is_some());

        // Same verdict for an empty resolved set.
        let empty = std::collections::BTreeSet::new();
        let prep = prepare_codex_root(tmp.path(), Some(&empty));
        assert!(!prep.safe);
    }

    #[test]
    fn prep_refuses_an_expected_name_that_is_a_file() {
        let tmp = TempDir::new().unwrap();
        let inbox = tmp.path().join("docs/codex");
        fs::create_dir_all(&inbox).unwrap();
        fs::write(inbox.join("Swan Practise"), "not a dir").unwrap();

        let set = names(&["Swan Practise"]);
        let prep = prepare_codex_root(tmp.path(), Some(&set));
        assert!(!prep.safe);
        assert!(prep
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("not a mirror directory"));
    }

    #[test]
    fn prep_sweeps_and_reports_pruned() {
        let tmp = TempDir::new().unwrap();
        let inbox = tmp.path().join("docs/codex");
        fs::create_dir_all(inbox.join("Swan Practise")).unwrap();
        fs::write(inbox.join("junk.txt"), "j").unwrap();

        let set = names(&["Swan Practise"]);
        let prep = prepare_codex_root(tmp.path(), Some(&set));
        assert!(prep.safe);
        assert_eq!(prep.pruned, 1);
        assert!(inbox.join("Swan Practise").is_dir());
        assert!(!inbox.join("junk.txt").exists());
    }

    #[test]
    fn connected_codex_names_resolves_the_full_set() {
        let db = make_db();
        let consumer = db.create_project("Consumer", None, "standard").unwrap();
        let cx_a = db.create_project("Swan Practise", None, "codex").unwrap();
        let cx_b = db.create_project("Other Codex", None, "codex").unwrap();
        db.connect_codex(consumer.id, cx_a.id).unwrap();
        db.connect_codex(consumer.id, cx_b.id).unwrap();

        let set = connected_codex_names(&db, consumer.id).unwrap();
        assert_eq!(set, names(&["Other Codex", "Swan Practise"]));

        // A project with no connections resolves to an EMPTY set — not an
        // error, and never a sweep trigger.
        let lonely = db.create_project("Lonely", None, "standard").unwrap();
        assert!(connected_codex_names(&db, lonely.id).unwrap().is_empty());
    }
}
