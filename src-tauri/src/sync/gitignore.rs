// .gitignore managed-block sync (dedup-aware). Manages content between
// `# --- solo-dev-hub:begin ---` / `# --- solo-dev-hub:end ---` markers.

use regex::Regex;
use std::fs;
use std::path::Path;

/// Sync a managed section in .gitignore between markers — with **dedup logic**.
/// Приципы:
/// - Из template берём только rule-строки (не комментарии и не пустые).
/// - Отфильтровываем те, которые **уже есть** в user-контенте (exact-match trimmed).
/// - Оставшиеся правила (если есть) → блок между маркерами.
/// - Если новых правил нет — блок удаляется (или не создаётся).
///
/// Returns true if file was modified.
pub fn sync_gitignore_section(template: &str, repo_root: &Path) -> Result<bool, String> {
    if template.trim().is_empty() {
        return Ok(false);
    }
    let target = repo_root.join(".gitignore");

    let existing = if target.exists() {
        fs::read_to_string(&target).map_err(|e| e.to_string())?
    } else {
        String::new()
    };

    let has_begin = existing.contains("solo-dev-hub:begin");
    let has_end = existing.contains("solo-dev-hub:end");

    // User content = everything outside any existing managed block.
    // Self-heal orphan markers: if only one marker present, just strip that line
    // and treat rest as user content — block will be rebuilt below.
    let block_re =
        Regex::new(r"(?s)\n*# --- solo-dev-hub:begin.*?# --- solo-dev-hub:end ---\n*").unwrap();
    let user_content = if has_begin && has_end {
        block_re.replace(&existing, "\n").to_string()
    } else if has_begin || has_end {
        // Orphan: strip any line containing either marker string, keep rest as user content.
        existing
            .lines()
            .filter(|line| {
                !line.contains("solo-dev-hub:begin") && !line.contains("solo-dev-hub:end")
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        existing.clone()
    };

    // Collect user's actual rules (trimmed, non-comment, non-empty).
    use std::collections::HashSet;
    let user_rules: HashSet<&str> = user_content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    // Extract rules from template only (drop comments) and keep those NOT already in user_rules.
    let new_rules: Vec<&str> = template
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter(|l| !user_rules.contains(l))
        .collect();

    // Decide desired final content.
    let user_trimmed = user_content.trim_end();
    let desired = if new_rules.is_empty() {
        if user_trimmed.is_empty() {
            String::new()
        } else {
            format!("{}\n", user_trimmed)
        }
    } else {
        let block = format!(
            "# --- solo-dev-hub:begin ---\n{}\n# --- solo-dev-hub:end ---",
            new_rules.join("\n")
        );
        if user_trimmed.is_empty() {
            format!("{}\n", block)
        } else {
            format!("{}\n\n{}\n", user_trimmed, block)
        }
    };

    if desired == existing {
        return Ok(false);
    }
    fs::write(&target, desired).map_err(|e| e.to_string())?;
    Ok(true)
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
