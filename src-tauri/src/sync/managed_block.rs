// Generic managed-block sync (dedup-aware), shared by .gitignore and
// .gitattributes. Manages content between `# --- solo-dev-hub:begin ---` /
// `# --- solo-dev-hub:end ---` markers. Both file formats use `#` line comments
// and one-rule-per-line syntax, so the merge logic is identical — only the
// target path differs (T-000097 split; generalized for B-000024).

use regex::Regex;
use std::fs;
use std::path::Path;

/// Sync a managed section in `target` between markers — with **dedup logic**.
/// Принципы:
/// - Из template берём только rule-строки (не комментарии и не пустые).
/// - Отфильтровываем те, которые **уже есть** в user-контенте (exact-match trimmed).
/// - Оставшиеся правила (если есть) → блок между маркерами.
/// - Если новых правил нет — блок удаляется (или не создаётся).
///
/// Returns true if file was modified.
pub fn sync_managed_block(template: &str, target: &Path) -> Result<bool, String> {
    if template.trim().is_empty() {
        return Ok(false);
    }

    let existing = if target.exists() {
        fs::read_to_string(target).map_err(|e| e.to_string())?
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
    fs::write(target, desired).map_err(|e| e.to_string())?;
    Ok(true)
}
