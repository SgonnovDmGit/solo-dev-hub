// T-000137: auto-commit SDH-synced cross-repo files so the recipient repo's
// local AI treats them as real history instead of foreign uncommitted changes.

use std::path::{Path, PathBuf};

use crate::git_ops::{
    commit_paths, commit_paths_index_only, is_ignored_for_add, list_markers_in_rev, CommitOutcome,
    GitBinary,
};

/// Doc subfolders that hold cross-repo synced content (REQ pairs, API contracts,
/// announcements). Auto-commit is scoped (git pathspec) to these (plus the
/// managed root files below) — the dev's own WIP anywhere else (including docs/
/// root files like the sender's own api.md, or the skeleton CLAUDE.md /
/// docs/project.md) is never swept in.
pub const CROSS_REPO_DIRS: &[&str] = &[
    "docs/backend-requirements",
    "docs/client-requirements",
    "docs/microservice-requirements",
    "docs/server-requirements",
    "docs/server-api",
    "docs/microservice-api",
    "docs/server-announcements",
    "docs/microservice-announcements",
];

/// App-managed root files written by the sync skeleton (a managed block is
/// merged into each on every sync). Auto-committed alongside the cross-repo
/// folders so a template change (e.g. a new .gitignore line) doesn't sit as a
/// foreign uncommitted diff. Skip-if-gitignored, same as the folders.
pub const MANAGED_ROOT_FILES: &[&str] = &[".gitignore", ".gitattributes"];

pub const SDH_COMMIT_NAME: &str = "Solo Dev Hub";
pub const SDH_COMMIT_EMAIL: &str = "noreply@solodevhub.app";

/// Subject and body of the marker-removal commit: deletion only, never an add.
const MARKER_REMOVAL_SUBJECT: &str = "chore(sdh-sync): untrack rule hand-back markers";
const MARKER_REMOVAL_BODY: &str = "Rule hand-back markers (docs/sdh_skills/*.approved, \
*.conflict) are local service files; Solo Dev Hub removed them from git. The \
working-tree files are untouched.";

/// Commit any changes (add/modify/delete) within the cross-repo folders that
/// exist in `repo_root`, on `branch`, with the SDH marker identity + message.
/// Returns `NothingToCommit` if no cross-repo folder exists or nothing changed;
/// `WrongBranch` if the repo isn't currently on `branch` (never checks out).
///
/// T-000244: a rule hand-back marker the branch still tracks is deleted from
/// git even when there is nothing else to commit (and when `docs/sdh_skills/`
/// no longer exists on disk — the deletion is read from `HEAD`). A marker is
/// never added or modified by the commit: only its removal travels. This repo's
/// push stays the owner's own step; the app never pushes here.
pub fn autocommit_repo(
    git: &GitBinary,
    repo_root: &Path,
    branch: &str,
) -> Result<CommitOutcome, String> {
    // Cross-repo folders present on disk that the dev has NOT gitignored.
    // A gitignored folder is the dev's deliberate "local inbox" (Option A):
    // git won't add it, and forcing it in would fight that choice — so skip it.
    // Skipping also fixes the crash where `git add` on an ignored path exits 1
    // and aborted the whole commit for the tracked folders alongside it.
    let mut tracked: Vec<PathBuf> = Vec::new();
    for d in CROSS_REPO_DIRS {
        let rel = PathBuf::from(d);
        if !repo_root.join(&rel).exists() {
            continue;
        }
        if is_ignored_for_add(git, repo_root, &rel)? {
            continue;
        }
        tracked.push(rel);
    }
    // Managed root files (.gitignore / .gitattributes) — same existence +
    // skip-if-gitignored logic. A repo that gitignores its own .gitignore or
    // .gitattributes just skips it (no crash).
    for f in MANAGED_ROOT_FILES {
        let rel = PathBuf::from(f);
        if !repo_root.join(&rel).exists() {
            continue;
        }
        if is_ignored_for_add(git, repo_root, &rel)? {
            continue;
        }
        tracked.push(rel);
    }
    // T-000244: rule hand-back markers still tracked in the branch. Read from
    // HEAD, not from the worktree or the current sweep, so a restart or a
    // failed commit still finds them. `docs/sdh_skills/` is deliberately NOT
    // added to `CROSS_REPO_DIRS`: only the markers' deletion has a path in.
    if !list_markers_in_rev(git, repo_root, "HEAD")?.is_empty() {
        let (subject, body) = if tracked.is_empty() {
            (MARKER_REMOVAL_SUBJECT, MARKER_REMOVAL_BODY)
        } else {
            (
                "chore(sdh-sync): sync cross-repo files",
                "Cross-repo requirement, API-contract and announcement files (plus managed root files) synced by Solo Dev Hub.",
            )
        };
        return commit_paths_index_only(
            git,
            repo_root,
            branch,
            &tracked,
            subject,
            Some(body),
            SDH_COMMIT_NAME,
            SDH_COMMIT_EMAIL,
        );
    }
    if tracked.is_empty() {
        return Ok(CommitOutcome::NothingToCommit);
    }
    commit_paths(
        git,
        repo_root,
        branch,
        &tracked,
        "chore(sdh-sync): sync cross-repo files",
        Some("Cross-repo requirement, API-contract and announcement files (plus managed root files) synced by Solo Dev Hub."),
        SDH_COMMIT_NAME,
        SDH_COMMIT_EMAIL,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_ops::check_git_available;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// Init a git repo with a stable identity + pinned `master` branch so commits
    /// work independently of the host's global git config. Mirrors the harness
    /// used in `git_ops.rs` tests.
    fn init_test_repo(dir: &Path) {
        run_git(dir, &["init", "-q"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["symbolic-ref", "HEAD", "refs/heads/master"]);
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
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

    /// Seed a baseline commit so the branch is born (commit_paths' branch guard
    /// needs a resolvable branch).
    fn seed_baseline(dir: &Path) {
        fs::write(dir.join("README.md"), "# r\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-q", "-m", "init"]);
    }

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let full = dir.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    /// Files listed in HEAD's tree commit.
    fn head_files(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["log", "-1", "--name-only", "--format="])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect()
    }

    fn staged_files(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["diff", "--cached", "--name-only"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.to_string())
            .collect()
    }

    #[test]
    fn test_autocommit_repo_commits_cross_repo_file() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // A synced api.md inside a cross-repo folder, uncommitted.
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });

        // File is in HEAD (git tracks with forward slashes on all platforms).
        assert!(
            head_files(tmp.path()).contains(&"docs/server-api/api.md".to_string()),
            "docs/server-api/api.md must be in HEAD tree, got {:?}",
            head_files(tmp.path())
        );

        // Commit author is the SDH marker identity.
        let an = Command::new("git")
            .args(["log", "-1", "--format=%an"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&an.stdout).trim(),
            SDH_COMMIT_NAME,
            "commit author must be the SDH marker"
        );
    }

    #[test]
    fn test_autocommit_repo_leaves_outside_files_uncommitted() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());

        // A file OUTSIDE the cross-repo dirs, staged as the dev's own WIP.
        write_file(tmp.path(), "src/lib.rs", "fn main() {}\n");
        run_git(tmp.path(), &["add", "src/lib.rs"]);
        // A synced file inside a cross-repo folder.
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });

        // The commit contains only the cross-repo file, not the WIP.
        let hf = head_files(tmp.path());
        assert!(hf.contains(&"docs/server-api/api.md".to_string()));
        assert!(
            !hf.contains(&"src/lib.rs".to_string()),
            "src/lib.rs must NOT be in the commit, got {:?}",
            hf
        );
        // WIP is still staged, untouched by the scoped commit.
        assert!(
            staged_files(tmp.path()).contains(&"src/lib.rs".to_string()),
            "src/lib.rs must remain staged"
        );
    }

    #[test]
    fn test_autocommit_repo_wrong_branch_no_commit() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        // Repo is on master; ask for dev.
        let outcome = autocommit_repo(&git, tmp.path(), "dev").unwrap();
        assert_eq!(
            outcome,
            CommitOutcome::WrongBranch {
                current: Some("master".to_string()),
                expected: "dev".to_string(),
            }
        );
        // No commit made — the synced file is not in HEAD.
        assert!(
            !head_files(tmp.path()).contains(&"docs/server-api/api.md".to_string()),
            "no commit must be made on wrong branch"
        );
    }

    #[test]
    fn test_autocommit_repo_skips_gitignored_folder() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // Dev gitignores the incoming announcements inbox (local-only) but
        // tracks the synced API contract.
        write_file(tmp.path(), ".gitignore", "docs/server-announcements/\n");
        run_git(tmp.path(), &["add", ".gitignore"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add gitignore"]);
        write_file(
            tmp.path(),
            "docs/server-announcements/ANNOUNCE-001.md",
            "# a\n",
        );
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        // Previously the gitignored folder made `git add` exit 1 and aborted
        // the whole commit; now it's skipped and the tracked file commits.
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });

        let hf = head_files(tmp.path());
        assert!(
            hf.contains(&"docs/server-api/api.md".to_string()),
            "tracked api.md must be committed, got {:?}",
            hf
        );
        assert!(
            !hf.iter().any(|f| f.contains("ANNOUNCE-001")),
            "gitignored announcement must NOT be committed, got {:?}",
            hf
        );
    }

    #[test]
    fn test_autocommit_repo_skips_ignored_folder_that_holds_tracked_files() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // The folder was tracked first, and only later added to .gitignore.
        write_file(
            tmp.path(),
            "docs/client-requirements/x.md",
            "# x
",
        );
        run_git(tmp.path(), &["add", "docs/client-requirements/x.md"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "track inbox"]);
        write_file(
            tmp.path(),
            ".gitignore",
            "docs/client-requirements/
",
        );
        write_file(
            tmp.path(),
            "docs/server-api/api.md",
            "# API
",
        );

        let git = check_git_available().expect("git available");
        // `git add -- docs/client-requirements` refuses this path even though
        // it holds tracked files; the folder must be skipped, the rest commit.
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });
        let hf = head_files(tmp.path());
        assert!(
            hf.contains(&"docs/server-api/api.md".to_string()),
            "{:?}",
            hf
        );
        assert!(hf.contains(&".gitignore".to_string()), "{:?}", hf);
    }

    #[test]
    fn test_autocommit_repo_all_folders_gitignored_is_noop() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        write_file(
            tmp.path(),
            ".gitignore",
            "docs/backend-requirements/\ndocs/server-announcements/\n",
        );
        run_git(tmp.path(), &["add", ".gitignore"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add gitignore"]);
        write_file(tmp.path(), "docs/backend-requirements/REQ-001.md", "# r\n");
        write_file(
            tmp.path(),
            "docs/server-announcements/ANNOUNCE-001.md",
            "# a\n",
        );

        let git = check_git_available().expect("git available");
        // Every present cross-repo folder is gitignored → clean no-op, no error.
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::NothingToCommit);
    }

    #[test]
    fn test_autocommit_repo_no_cross_repo_folders_is_noop() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // Only a non-cross-repo file present.
        write_file(tmp.path(), "docs/todo.md", "- [ ] x\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::NothingToCommit);
    }

    #[test]
    fn test_autocommit_commits_managed_gitignore() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // App-managed .gitignore (with a managed-block marker line) written by
        // the sync skeleton, plus a synced cross-repo file — both uncommitted.
        write_file(
            tmp.path(),
            ".gitignore",
            "# manager:begin\ndocs/codex/\n# manager:end\n",
        );
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });

        let hf = head_files(tmp.path());
        assert!(
            hf.contains(&".gitignore".to_string()),
            "managed .gitignore must be in HEAD tree, got {:?}",
            hf
        );
    }

    #[test]
    fn test_autocommit_commits_managed_gitattributes() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // App-managed .gitattributes written by the sync skeleton, plus a synced
        // cross-repo file — both uncommitted.
        write_file(tmp.path(), ".gitattributes", "* text=auto\n");
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });

        let hf = head_files(tmp.path());
        assert!(
            hf.contains(&".gitattributes".to_string()),
            "managed .gitattributes must be in HEAD tree, got {:?}",
            hf
        );
    }

    #[test]
    fn test_autocommit_skips_gitignored_managed_root_file() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // A committed .gitignore that ignores .gitattributes.
        write_file(tmp.path(), ".gitignore", ".gitattributes\n");
        run_git(tmp.path(), &["add", ".gitignore"]);
        run_git(tmp.path(), &["commit", "-q", "-m", "add gitignore"]);
        // The gitignored .gitattributes plus a tracked synced file.
        write_file(tmp.path(), ".gitattributes", "* text=auto\n");
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });

        let hf = head_files(tmp.path());
        assert!(
            hf.contains(&"docs/server-api/api.md".to_string()),
            "tracked api.md must be committed, got {:?}",
            hf
        );
        assert!(
            !hf.contains(&".gitattributes".to_string()),
            "gitignored .gitattributes must NOT be committed, got {:?}",
            hf
        );
    }

    // ── T-000244: markers travel out of git as deletions only ────────────────

    /// `<status> <path>` of everything staged against HEAD.
    fn staged_status(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["diff", "--cached", "--name-status", "-z"])
            .current_dir(dir)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let fields: Vec<&str> = text.split('\0').filter(|s| !s.is_empty()).collect();
        fields
            .chunks(2)
            .map(|c| format!("{} {}", c[0], c[1]))
            .collect()
    }

    /// Files of HEAD's tree.
    fn tree_files(dir: &Path) -> Vec<String> {
        let out = Command::new("git")
            .args(["ls-tree", "-r", "--name-only", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.to_string())
            .collect()
    }

    /// A repo whose HEAD tracks a rule copy and a marker, exactly as a
    /// contributor's agent leaves it behind (committed together with the edit).
    fn repo_with_tracked_marker(dir: &Path) {
        init_test_repo(dir);
        write_file(dir, "README.md", "# r\n");
        write_file(dir, "docs/sdh_skills/sdh-alpha.md", "ALPHA REVISED\n");
        write_file(dir, "docs/sdh_skills/sdh-alpha.md.approved", "approved\n");
        run_git(dir, &["add", "-A", "."]);
        run_git(dir, &["commit", "-q", "-m", "marker committed by an agent"]);
    }

    /// A tracked marker that is also untracked from the index (the refresh
    /// sweep's staging), so both halves of the fix meet in one repo.
    fn repo_with_swept_marker(dir: &Path) {
        repo_with_tracked_marker(dir);
        run_git(
            dir,
            &[
                "rm",
                "-q",
                "--cached",
                "docs/sdh_skills/sdh-alpha.md.approved",
            ],
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_autocommit_delivers_a_marker_deletion_with_nothing_else_to_commit() {
        let tmp = TempDir::new().unwrap();
        repo_with_swept_marker(tmp.path());

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();

        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });
        let tree = tree_files(tmp.path());
        assert!(
            !tree.iter().any(|f| f.ends_with(".approved")),
            "the marker left the branch: {:?}",
            tree
        );
        assert!(tree.contains(&"docs/sdh_skills/sdh-alpha.md".to_string()));
        // The live marker and the copy stay on disk, byte-for-byte.
        assert_eq!(
            fs::read_to_string(tmp.path().join("docs/sdh_skills/sdh-alpha.md.approved")).unwrap(),
            "approved\n"
        );
        assert!(
            staged_status(tmp.path()).is_empty(),
            "{:?}",
            staged_status(tmp.path())
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_autocommit_finds_the_marker_deletion_after_a_restart_without_the_folder() {
        let tmp = TempDir::new().unwrap();
        repo_with_tracked_marker(tmp.path());
        // The whole folder is gone from disk (an old clone, a manual cleanup):
        // the deletion is still owed to the branch.
        fs::remove_dir_all(tmp.path().join("docs/sdh_skills")).unwrap();

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();

        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });
        assert!(!tree_files(tmp.path())
            .iter()
            .any(|f| f.ends_with(".approved")));
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_autocommit_never_adds_a_live_marker_or_the_rule_copy() {
        let tmp = TempDir::new().unwrap();
        repo_with_swept_marker(tmp.path());
        // A second, still-untracked marker and an untracked rule copy beside it.
        write_file(tmp.path(), "docs/sdh_skills/sdh-beta.md", "BETA\n");
        write_file(
            tmp.path(),
            "docs/sdh_skills/sdh-beta.md.approved",
            "approved\n",
        );

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();

        assert_eq!(outcome, CommitOutcome::Committed { files: 1 });
        let tree = tree_files(tmp.path());
        assert!(
            !tree
                .iter()
                .any(|f| f.ends_with(".approved") || f.ends_with(".conflict")),
            "no marker may be added: {:?}",
            tree
        );
        assert!(
            !tree.contains(&"docs/sdh_skills/sdh-beta.md".to_string()),
            "docs/sdh_skills/ is not a cross-repo folder: {:?}",
            tree
        );
        assert!(tmp
            .path()
            .join("docs/sdh_skills/sdh-beta.md.approved")
            .exists());
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_autocommit_marker_deletion_keeps_foreign_wip_out_and_intact() {
        let tmp = TempDir::new().unwrap();
        repo_with_swept_marker(tmp.path());
        write_file(tmp.path(), "src/lib.rs", "fn main() {}\n");
        run_git(tmp.path(), &["add", "src/lib.rs"]);
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();

        // The cross-repo file travels with the deletion; the foreign staged
        // file does not, and it is still staged afterwards.
        assert_eq!(outcome, CommitOutcome::Committed { files: 2 });
        let tree = tree_files(tmp.path());
        assert!(
            tree.contains(&"docs/server-api/api.md".to_string()),
            "{:?}",
            tree
        );
        assert!(!tree.contains(&"src/lib.rs".to_string()), "{:?}", tree);
        assert!(staged_status(tmp.path()).contains(&"A src/lib.rs".to_string()));
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_autocommit_failed_marker_commit_loses_nothing_and_is_retried() {
        let tmp = TempDir::new().unwrap();
        repo_with_swept_marker(tmp.path());
        // Signing is on but no key exists: the commit fails. The app must not
        // bypass signing, and must not lose the staged deletion either.
        run_git(tmp.path(), &["config", "commit.gpgsign", "true"]);
        run_git(
            tmp.path(),
            &["config", "user.signingkey", "0000000000000000"],
        );

        let git = check_git_available().expect("git available");
        let failed = autocommit_repo(&git, tmp.path(), "master");

        assert!(
            failed.is_err(),
            "the signed commit must fail, got {:?}",
            failed
        );
        assert!(
            tree_files(tmp.path())
                .iter()
                .any(|f| f.ends_with(".approved")),
            "the marker is still tracked in HEAD"
        );
        assert!(
            staged_status(tmp.path())
                .contains(&"D docs/sdh_skills/sdh-alpha.md.approved".to_string()),
            "the staged deletion survived the failure: {:?}",
            staged_status(tmp.path())
        );

        // The owner turns signing off; the next pass delivers the deletion.
        run_git(tmp.path(), &["config", "commit.gpgsign", "false"]);
        let retried = autocommit_repo(&git, tmp.path(), "master").unwrap();
        assert_eq!(retried, CommitOutcome::Committed { files: 1 });
        assert!(!tree_files(tmp.path())
            .iter()
            .any(|f| f.ends_with(".approved")));
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_autocommit_marker_deletion_waits_for_the_allowed_branch() {
        let tmp = TempDir::new().unwrap();
        repo_with_swept_marker(tmp.path());
        write_file(tmp.path(), "docs/server-api/api.md", "# API\n");

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "dev").unwrap();

        assert_eq!(
            outcome,
            CommitOutcome::WrongBranch {
                current: Some("master".to_string()),
                expected: "dev".to_string(),
            }
        );
        let tree = tree_files(tmp.path());
        assert!(tree.iter().any(|f| f.ends_with(".approved")), "{:?}", tree);
        assert!(
            !tree.contains(&"docs/server-api/api.md".to_string()),
            "{:?}",
            tree
        );
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_autocommit_without_tracked_markers_ignores_docs_sdh_skills_entirely() {
        let tmp = TempDir::new().unwrap();
        init_test_repo(tmp.path());
        seed_baseline(tmp.path());
        // Untracked rule copies and markers only: nothing is tracked in HEAD,
        // so the ordinary path must not stage or commit any of them.
        write_file(tmp.path(), "docs/sdh_skills/sdh-alpha.md", "ALPHA\n");
        write_file(
            tmp.path(),
            "docs/sdh_skills/sdh-alpha.md.approved",
            "approved\n",
        );

        let git = check_git_available().expect("git available");
        let outcome = autocommit_repo(&git, tmp.path(), "master").unwrap();

        assert_eq!(outcome, CommitOutcome::NothingToCommit);
        assert!(tree_files(tmp.path())
            .iter()
            .all(|f| !f.contains("sdh_skills")));
        assert!(staged_status(tmp.path()).is_empty());
    }
}
