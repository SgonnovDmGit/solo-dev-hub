// ── Legacy (old-format) import ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ParsedHeader {
    pub github_name: String,
}

#[derive(Debug, Clone)]
pub struct ParsedBug {
    #[allow(dead_code)]
    pub id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: String,
    pub category: String,
    pub is_resolved: bool,
    pub fix_attempts: i32,
    #[allow(dead_code)]
    pub created_at: Option<String>,
    #[allow(dead_code)]
    pub resolved_at: Option<String>,
}

#[derive(Debug)]
pub struct ParsedMarkdown {
    #[allow(dead_code)]
    pub github_name: String,
    pub bugs: Vec<ParsedBug>,
    pub skipped_lines: usize,
}

pub fn parse_header(md: &str) -> Option<ParsedHeader> {
    for line in md.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# Bug List:") {
            let github_name = rest.trim().to_string();
            if !github_name.is_empty() {
                return Some(ParsedHeader { github_name });
            }
        }
    }
    None
}

pub fn parse_bug_entry(line: &str, desc_lines: &[&str]) -> Option<ParsedBug> {
    let trimmed = line.trim();

    let (is_resolved, rest) = if let Some(r) = trimmed.strip_prefix("- [ ]") {
        (false, r.trim())
    } else if let Some(r) = trimmed.strip_prefix("- [x]") {
        (true, r.trim())
    } else {
        return None;
    };

    let (priority, category, rest) = if let Some(r) = rest.strip_prefix("**[") {
        if let Some(end) = r.find("]**") {
            let inner = &r[..end];
            let after = r[end + 3..].trim();
            if let Some(pipe_pos) = inner.find('|') {
                let priority = inner[..pipe_pos].to_lowercase();
                let category = inner[pipe_pos + 1..].to_lowercase();
                (priority, category, after)
            } else {
                let priority = inner.to_lowercase();
                (priority, "unknown".to_string(), after)
            }
        } else {
            ("medium".to_string(), "unknown".to_string(), rest)
        }
    } else {
        ("medium".to_string(), "unknown".to_string(), rest)
    };

    let id = if let Some(comment_start) = rest.find("<!-- id:") {
        let after = &rest[comment_start + 8..];
        if let Some(comment_end) = after.find(" -->") {
            let raw = after[..comment_end].trim().to_string();
            if raw.is_empty() {
                None
            } else {
                Some(raw)
            }
        } else {
            None
        }
    } else {
        None
    };

    let fix_attempts = if let Some(att_start) = rest.find("[attempts:") {
        let after = &rest[att_start + 10..];
        let trimmed_after = after.trim_start();
        if let Some(att_end) = trimmed_after.find(']') {
            trimmed_after[..att_end].trim().parse::<i32>().unwrap_or(0)
        } else {
            0
        }
    } else {
        0
    };

    let title_end = rest
        .find(" [attempts:")
        .or_else(|| rest.find(" <!-- id:"))
        .unwrap_or(rest.len());
    let title = rest[..title_end].trim().to_string();

    if title.is_empty() {
        return None;
    }

    let mut desc_parts: Vec<String> = Vec::new();
    let mut created_at: Option<String> = None;
    let mut resolved_at: Option<String> = None;

    for desc_line in desc_lines {
        let dl = desc_line.trim();
        if let Some(content) = dl.strip_prefix("> ") {
            if let Some(date) = content.strip_prefix("Created: ") {
                created_at = Some(date.trim().to_string());
            } else if let Some(date) = content.strip_prefix("Resolved: ") {
                resolved_at = Some(date.trim().to_string());
            } else {
                desc_parts.push(content.to_string());
            }
        } else if dl == ">" {
            desc_parts.push(String::new());
        }
    }

    let description = if desc_parts.is_empty() {
        None
    } else {
        Some(desc_parts.join("\n"))
    };

    Some(ParsedBug {
        id,
        title,
        description,
        priority,
        category,
        is_resolved,
        fix_attempts,
        created_at,
        resolved_at,
    })
}

/// Parse legacy "# Bug List:" format markdown.
pub fn parse_markdown_legacy(md: &str) -> Option<ParsedMarkdown> {
    let header = parse_header(md)?;
    let lines: Vec<&str> = md.lines().collect();

    let mut bugs: Vec<ParsedBug> = Vec::new();
    let mut skipped_lines = 0usize;
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.starts_with("- [ ]") || trimmed.starts_with("- [x]") {
            let mut desc_lines: Vec<&str> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let next = lines[j];
                let next_trimmed = next.trim();
                if next_trimmed.starts_with('>')
                    || (next.starts_with("  ") && !next_trimmed.starts_with("- "))
                {
                    desc_lines.push(next);
                    j += 1;
                } else {
                    break;
                }
            }

            if let Some(bug) = parse_bug_entry(line, &desc_lines) {
                bugs.push(bug);
            } else {
                skipped_lines += 1;
            }

            i = j;
        } else {
            i += 1;
        }
    }

    Some(ParsedMarkdown {
        github_name: header.github_name,
        bugs,
        skipped_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_header() {
        let md = "# Bug List: owner/repo\n\n**Project:** Test";
        let header = parse_header(md).unwrap();
        assert_eq!(header.github_name, "owner/repo");
    }

    #[test]
    fn test_parse_bug_entry_open() {
        let line = "- [ ] **[CRITICAL|BACKEND]** Bug title [attempts: 2] <!-- id:42 -->";
        let desc = ["  > Description", "  > Created: 2026-03-20"];
        let bug = parse_bug_entry(line, &desc).unwrap();
        assert_eq!(bug.title, "Bug title");
        assert_eq!(bug.priority, "critical");
        assert_eq!(bug.category, "backend");
        assert!(!bug.is_resolved);
        assert_eq!(bug.fix_attempts, 2);
        assert_eq!(bug.id.as_deref(), Some("42"));
        assert_eq!(bug.description.as_deref(), Some("Description"));
        assert_eq!(bug.created_at.as_deref(), Some("2026-03-20"));
    }

    #[test]
    fn test_parse_bug_entry_resolved() {
        let line = "- [x] **[MEDIUM|UI_UX]** Fixed bug [attempts: 1] <!-- id:41 -->";
        let desc = ["  > Was broken", "  > Resolved: 2026-03-18"];
        let bug = parse_bug_entry(line, &desc).unwrap();
        assert_eq!(bug.title, "Fixed bug");
        assert_eq!(bug.priority, "medium");
        assert_eq!(bug.category, "ui_ux");
        assert!(bug.is_resolved);
        assert_eq!(bug.fix_attempts, 1);
        assert_eq!(bug.id.as_deref(), Some("41"));
        assert_eq!(bug.resolved_at.as_deref(), Some("2026-03-18"));
    }

    #[test]
    fn test_parse_bug_entry_no_id() {
        let line = "- [ ] **[LOW]** No id bug [attempts: 0]";
        let bug = parse_bug_entry(line, &[]).unwrap();
        assert_eq!(bug.title, "No id bug");
        assert_eq!(bug.category, "unknown");
        assert_eq!(bug.id, None);
        assert_eq!(bug.fix_attempts, 0);
    }

    #[test]
    fn test_parse_full_markdown() {
        let md = r#"# Bug List: owner/repo

**Project:** My Project
**Role:** server
**Exported:** 2026-03-28

## Open Bugs (1)

- [ ] **[CRITICAL|BACKEND]** Bug title [attempts: 2] <!-- id:42 -->
  > Description
  > Created: 2026-03-20

## Resolved Bugs (1)

- [x] **[MEDIUM|UI_UX]** Fixed bug [attempts: 1] <!-- id:41 -->
  > Was broken
  > Resolved: 2026-03-18
"#;
        let parsed = parse_markdown_legacy(md).unwrap();
        assert_eq!(parsed.github_name, "owner/repo");
        assert_eq!(parsed.bugs.len(), 2);

        let open = parsed.bugs.iter().find(|b| !b.is_resolved).unwrap();
        assert_eq!(open.title, "Bug title");
        assert_eq!(open.id.as_deref(), Some("42"));

        let resolved = parsed.bugs.iter().find(|b| b.is_resolved).unwrap();
        assert_eq!(resolved.title, "Fixed bug");
        assert_eq!(resolved.id.as_deref(), Some("41"));
    }

    #[test]
    fn test_parse_legacy() {
        let md = r#"# Bug List: owner/repo

## Open Bugs (1)

- [ ] **[HIGH|BACKEND]** Login crash [attempts: 1] <!-- id:10 -->
  > Crash on login page
  > Created: 2026-03-01

## Resolved Bugs (0)

"#;
        let parsed = parse_markdown_legacy(md).unwrap();
        assert_eq!(parsed.bugs.len(), 1);
        let b = &parsed.bugs[0];
        assert_eq!(b.title, "Login crash");
        assert_eq!(b.priority, "high");
        assert_eq!(b.category, "backend");
        assert!(!b.is_resolved);
        assert_eq!(b.id.as_deref(), Some("10"));
    }
}
