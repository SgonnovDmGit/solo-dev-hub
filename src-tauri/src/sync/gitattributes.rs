// .gitattributes managed-block sync (B-000024). Thin wrapper over the shared
// dedup-aware section-merge in `managed_block` — targets
// `<repo_root>/.gitattributes`. .gitattributes uses the same `#` comment syntax
// and one-rule-per-line format as .gitignore, so the merge logic is shared
// verbatim — only the target file differs.

use super::managed_block::sync_managed_block;
use std::path::Path;

/// Sync the managed `.gitattributes` section between markers (dedup-aware).
/// Returns true if the file was modified. See `sync_managed_block` for the
/// full merge semantics.
pub fn sync_gitattributes_section(template: &str, repo_root: &Path) -> Result<bool, String> {
    sync_managed_block(template, &repo_root.join(".gitattributes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_creates_new_gitattributes_with_block() {
        let tmp = TempDir::new().unwrap();
        let changed =
            sync_gitattributes_section("# Normalize\n* text=auto eol=lf\n*.png binary", tmp.path())
                .unwrap();
        assert!(changed);
        let content = fs::read_to_string(tmp.path().join(".gitattributes")).unwrap();
        assert!(content.contains("solo-dev-hub:begin"));
        assert!(content.contains("solo-dev-hub:end"));
        assert!(content.contains("* text=auto eol=lf"));
        assert!(content.contains("*.png binary"));
        assert!(
            !content.contains("# Normalize"),
            "comments from template NOT carried into block"
        );
    }

    #[test]
    fn test_dedup_filters_existing_user_rules() {
        let tmp = TempDir::new().unwrap();
        // User already has the catch-all + one binary rule
        let user = "* text=auto eol=lf\n*.png binary\n";
        fs::write(tmp.path().join(".gitattributes"), user).unwrap();

        let template = "* text=auto eol=lf\n*.png binary\n*.exe binary\n*.sh text eol=lf";
        let changed = sync_gitattributes_section(template, tmp.path()).unwrap();
        assert!(changed);

        let content = fs::read_to_string(tmp.path().join(".gitattributes")).unwrap();
        // User's catch-all preserved at top and NOT duplicated in the block
        assert!(content.starts_with("* text=auto eol=lf"));
        let block_start = content.find("solo-dev-hub:begin").unwrap();
        let block = &content[block_start..];
        assert!(block.contains("*.exe binary"), "new rule lands in block");
        assert!(
            block.contains("*.sh text eol=lf"),
            "new rule lands in block"
        );
        assert!(
            !block.contains("* text=auto eol=lf"),
            "duplicate catch-all NOT re-added to block"
        );
        assert!(
            !block.contains("*.png binary"),
            "duplicate binary rule NOT re-added to block"
        );
    }

    #[test]
    fn test_preserves_user_content_outside_block() {
        let tmp = TempDir::new().unwrap();
        let user = "# my custom attrs\n*.myext text eol=lf\n";
        fs::write(tmp.path().join(".gitattributes"), user).unwrap();

        sync_gitattributes_section("*.exe binary", tmp.path()).unwrap();
        let content = fs::read_to_string(tmp.path().join(".gitattributes")).unwrap();
        assert!(content.contains("*.myext text eol=lf"), "user rule kept");
        assert!(content.contains("# my custom attrs"), "user comment kept");
        assert!(content.contains("*.exe binary"), "managed rule added");
    }

    #[test]
    fn test_idempotent() {
        let tmp = TempDir::new().unwrap();
        let template = "* text=auto eol=lf\n*.exe binary";
        sync_gitattributes_section(template, tmp.path()).unwrap();
        let before = fs::read_to_string(tmp.path().join(".gitattributes")).unwrap();

        let changed2 = sync_gitattributes_section(template, tmp.path()).unwrap();
        assert!(!changed2, "second call = no change");
        let after = fs::read_to_string(tmp.path().join(".gitattributes")).unwrap();
        assert_eq!(before, after);
    }
}
