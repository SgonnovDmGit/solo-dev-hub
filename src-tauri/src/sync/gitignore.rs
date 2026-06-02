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
}
