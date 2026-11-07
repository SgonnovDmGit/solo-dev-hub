// .gitignore managed-block sync. Thin wrapper over the shared dedup-aware
// section-merge in `managed_block` — targets `<repo_root>/.gitignore`.

use super::managed_block::sync_managed_block;
use std::path::Path;

/// Sync the managed `.gitignore` section between markers (dedup-aware).
/// Returns true if the file was modified. See `sync_managed_block` for the
/// full merge semantics.
pub fn sync_gitignore_section(template: &str, repo_root: &Path) -> Result<bool, String> {
    sync_managed_block(template, &repo_root.join(".gitignore"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── sync_gitignore_section tests (F-016 dedup-merge) ──────────────────────

    #[test]
    fn test_sync_gitignore_creates_new_file_with_block() {
        let tmp = TempDir::new().unwrap();
        let changed = sync_gitignore_section("# Section\n*.log\n.env", tmp.path()).unwrap();
        assert!(changed);
        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains("solo-dev-hub:begin"));
        assert!(content.contains("solo-dev-hub:end"));
        assert!(content.contains("*.log"));
        assert!(content.contains(".env"));
        assert!(
            !content.contains("# Section"),
            "comments from template NOT carried into block"
        );
    }

    #[test]
    fn test_sync_gitignore_dedup_filters_duplicates() {
        let tmp = TempDir::new().unwrap();
        let user = "# Секреты\n.env\n\n# AI\nCLAUDE.*\n.claude/\n*.exe\n";
        fs::write(tmp.path().join(".gitignore"), user).unwrap();

        // Template has .env (dup), CLAUDE.* (dup), .claude/ (dup), new ones below
        let template =
            "# Секреты\n.env\n\n# AI\nCLAUDE.*\n.claude/\n\n# Docs\ndocs/todo.md\ndocs/done.md";
        let changed = sync_gitignore_section(template, tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(
            content.starts_with("# Секреты"),
            "user content preserved at top"
        );
        assert!(content.contains("*.exe"), "user rules preserved");
        // .env appears ONCE (user's), not duplicated in block
        assert_eq!(
            content.matches("\n.env\n").count() + content.matches("\n.env$").count(),
            1
        );
        // Block contains only NEW rules (docs/todo.md, docs/done.md)
        let block_start = content.find("solo-dev-hub:begin").unwrap();
        let block_end = content.find("solo-dev-hub:end").unwrap();
        let block = &content[block_start..block_end];
        assert!(block.contains("docs/todo.md"), "new rules in block");
        assert!(block.contains("docs/done.md"), "new rules in block");
        assert!(!block.contains(".env"), "duplicate NOT in block");
        assert!(!block.contains("CLAUDE.*"), "duplicate NOT in block");
        assert!(!block.contains(".claude/"), "duplicate NOT in block");
    }

    #[test]
    fn test_sync_gitignore_removes_block_when_all_duplicates() {
        let tmp = TempDir::new().unwrap();
        let user = ".env\n*.log\ndocs/todo.md\n";
        fs::write(tmp.path().join(".gitignore"), user).unwrap();

        // All template rules already in user — block shouldn't be created
        let changed = sync_gitignore_section(".env\n*.log\ndocs/todo.md", tmp.path()).unwrap();
        assert!(!changed, "no changes needed — all duplicates");

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(
            !content.contains("solo-dev-hub"),
            "no block created when all dup"
        );
    }

    #[test]
    fn test_sync_gitignore_rebuilds_block_on_update() {
        let tmp = TempDir::new().unwrap();
        // Initial: user has .env; block has docs/todo.md (now stale)
        let initial =
            ".env\n\n# --- solo-dev-hub:begin ---\nold-rule\n# --- solo-dev-hub:end ---\n";
        fs::write(tmp.path().join(".gitignore"), initial).unwrap();

        // New template: .env (dup with user), new-rule (new), *.bak (new)
        let changed = sync_gitignore_section(".env\nnew-rule\n*.bak", tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains("new-rule"));
        assert!(content.contains("*.bak"));
        assert!(!content.contains("old-rule"), "old block content replaced");
        let block_start = content.find("solo-dev-hub:begin").unwrap();
        let block_end = content.find("solo-dev-hub:end").unwrap();
        let block = &content[block_start..block_end];
        assert!(!block.contains(".env"), ".env not duplicated in new block");
    }

    #[test]
    fn test_sync_gitignore_idempotent() {
        let tmp = TempDir::new().unwrap();
        let template = ".env\n*.log";
        sync_gitignore_section(template, tmp.path()).unwrap();
        let before = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();

        let changed2 = sync_gitignore_section(template, tmp.path()).unwrap();
        assert!(!changed2, "second call = no change");
        let after = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn test_sync_gitignore_self_heals_orphan_begin() {
        let tmp = TempDir::new().unwrap();
        // File has only begin marker — orphan
        let broken = ".env\n# --- solo-dev-hub:begin ---\nsome-rule\n";
        fs::write(tmp.path().join(".gitignore"), broken).unwrap();

        let changed = sync_gitignore_section("new-rule", tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains(".env"), "user content preserved");
        assert!(
            content.contains("solo-dev-hub:begin"),
            "new valid block created"
        );
        assert!(
            content.contains("solo-dev-hub:end"),
            "new valid block created"
        );
        assert!(content.contains("new-rule"), "template rule added");
        // Old orphan line was stripped
        assert_eq!(
            content.matches("solo-dev-hub:begin").count(),
            1,
            "only one begin marker"
        );
    }

    #[test]
    fn test_sync_gitignore_self_heals_orphan_end() {
        let tmp = TempDir::new().unwrap();
        let broken = ".env\nstuff\n# --- solo-dev-hub:end ---\n";
        fs::write(tmp.path().join(".gitignore"), broken).unwrap();

        let changed = sync_gitignore_section("new-rule", tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(content.contains(".env"));
        assert!(content.contains("new-rule"));
        assert_eq!(content.matches("solo-dev-hub:end").count(), 1);
    }

    // ── T-000244: the bundled template ignores the hand-back markers ────────

    use crate::git_ops::{check_ignore, GitBinary};
    use crate::sync::shared::gitignore_template_for;
    use std::path::PathBuf;
    use std::process::Command;

    /// The real bundled `.gitignore` template.
    fn bundled_gitignore() -> String {
        crate::template_seeder::bundled_file_content("_global", ".gitignore.tmpl")
            .expect("bundled .gitignore.tmpl")
    }

    /// Init a git repo (identity + pinned `master`) so `check-ignore` has a repo.
    fn init_git(dir: &Path) {
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "T"],
            vec!["symbolic-ref", "HEAD", "refs/heads/master"],
        ] {
            let out = Command::new("git")
                .args(&args)
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
    }

    #[test]
    fn test_bundled_gitignore_carries_both_marker_globs() {
        let template = bundled_gitignore();
        assert!(
            template
                .lines()
                .any(|l| l.trim() == "docs/sdh_skills/*.approved"),
            "the approval marker glob is missing:\n{}",
            template
        );
        assert!(
            template
                .lines()
                .any(|l| l.trim() == "docs/sdh_skills/*.conflict"),
            "the conflict marker glob is missing:\n{}",
            template
        );
        assert!(
            template.lines().any(|l| l.trim() == "docs/sdh_skills/"),
            "the ordinary repo still ignores the whole folder"
        );
    }

    #[test]
    fn test_shared_render_keeps_the_marker_globs_and_drops_the_folder() {
        let shared = gitignore_template_for(&bundled_gitignore(), true);
        assert!(
            !shared.lines().any(|l| l.trim() == "docs/sdh_skills/"),
            "shared render stops ignoring the owner's rule copies:\n{}",
            shared
        );
        assert!(shared
            .lines()
            .any(|l| l.trim() == "docs/sdh_skills/*.approved"));
        assert!(shared
            .lines()
            .any(|l| l.trim() == "docs/sdh_skills/*.conflict"));
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_normal_render_ignores_both_markers_for_git() {
        let tmp = TempDir::new().unwrap();
        init_git(tmp.path());
        sync_gitignore_section(&bundled_gitignore(), tmp.path()).unwrap();
        let git = GitBinary(PathBuf::from("git"));

        for marker in [
            "docs/sdh_skills/sdh-alpha.md.approved",
            "docs/sdh_skills/a.md.conflict",
        ] {
            assert!(
                check_ignore(&git, tmp.path(), Path::new(marker), true).unwrap(),
                "{} must be ignored by the rendered .gitignore",
                marker
            );
        }
    }

    #[test]
    #[ignore = "spawns git: slow on Windows; run with --include-ignored (CI does)"]
    fn test_shared_render_leaves_the_rule_copy_visible_but_ignores_the_markers() {
        let tmp = TempDir::new().unwrap();
        init_git(tmp.path());
        let shared = gitignore_template_for(&bundled_gitignore(), true);
        sync_gitignore_section(&shared, tmp.path()).unwrap();
        let git = GitBinary(PathBuf::from("git"));

        assert!(
            !check_ignore(
                &git,
                tmp.path(),
                Path::new("docs/sdh_skills/sdh-alpha.md"),
                true
            )
            .unwrap(),
            "the owner's committed rule copy travels through git"
        );
        for marker in [
            "docs/sdh_skills/sdh-alpha.md.approved",
            "docs/sdh_skills/a.md.conflict",
        ] {
            assert!(
                check_ignore(&git, tmp.path(), Path::new(marker), true).unwrap(),
                "{} stays ignored in a shared repo",
                marker
            );
        }
    }

    #[test]
    fn test_marker_glob_of_the_user_is_not_duplicated_by_the_block() {
        let tmp = TempDir::new().unwrap();
        // The user already ignores the markers by hand.
        fs::write(
            tmp.path().join(".gitignore"),
            "# mine\ndocs/sdh_skills/*.approved\n",
        )
        .unwrap();

        let changed = sync_gitignore_section(&bundled_gitignore(), tmp.path()).unwrap();

        assert!(changed);
        let content = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert_eq!(
            content.matches("docs/sdh_skills/*.approved").count(),
            1,
            "the user's line is kept, not duplicated:\n{}",
            content
        );
        assert!(
            content.contains("docs/sdh_skills/*.conflict"),
            "the other glob is new, so the block carries it"
        );
        // And a second render changes nothing.
        assert!(!sync_gitignore_section(&bundled_gitignore(), tmp.path()).unwrap());
        assert_eq!(
            fs::read_to_string(tmp.path().join(".gitignore")).unwrap(),
            content
        );
    }
}
