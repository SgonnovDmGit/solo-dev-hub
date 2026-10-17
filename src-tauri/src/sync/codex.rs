// Codex broadcast: mirror a codex project's per-lane svod folders into each
// connected consumer's same-role repo, read-only. Role-matched, best-effort.

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
}
