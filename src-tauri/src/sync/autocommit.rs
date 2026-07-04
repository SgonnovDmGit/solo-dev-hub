// T-000137: auto-commit SDH-synced cross-repo files so the recipient repo's
// local AI treats them as real history instead of foreign uncommitted changes.

use std::path::{Path, PathBuf};

use crate::git_ops::{commit_paths, is_gitignored, CommitOutcome, GitBinary};

/// Doc subfolders that hold cross-repo synced content (REQ pairs, API contracts,
/// announcements). Auto-commit is scoped (git pathspec) to these — the dev's own
/// WIP anywhere else (including docs/ root files like the sender's own api.md, or
/// the skeleton CLAUDE.md/.gitignore) is never swept in.
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

pub const SDH_COMMIT_NAME: &str = "Solo Dev Hub";
pub const SDH_COMMIT_EMAIL: &str = "noreply@solodevhub.app";

/// Commit any changes (add/modify/delete) within the cross-repo folders that
/// exist in `repo_root`, on `branch`, with the SDH marker identity + message.
/// Returns `NothingToCommit` if no cross-repo folder exists or nothing changed;
/// `WrongBranch` if the repo isn't currently on `branch` (never checks out).
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
        if is_gitignored(git, repo_root, &rel)? {
            continue;
        }
        tracked.push(rel);
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
        Some("Cross-repo requirement, API-contract and announcement files synced by Solo Dev Hub."),
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
}
