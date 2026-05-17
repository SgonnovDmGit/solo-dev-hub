use crate::models::{DoneTask, TodoTask};

use super::util::split_pipe_respecting_escape;

// ── F-021 TODO / DONE parsers ────────────────────────────────────────────────

/// Detect YYYY-MM-DD, DD.MM.YYYY or DD/MM/YYYY at trimmed-equal-10 length.
/// Used as an anchor in the done parser so column shift (no id, or extra middle field)
/// doesn't land date-text in the description column.
fn is_date_like(s: &str) -> bool {
    let s = s.trim();
    if s.len() != 10 {
        return false;
    }
    let b = s.as_bytes();
    // YYYY-MM-DD
    if b[4] == b'-' && b[7] == b'-' {
        return b
            .iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit());
    }
    // DD.MM.YYYY or DD/MM/YYYY
    if (b[2] == b'.' || b[2] == b'/') && b[5] == b[2] {
        return b
            .iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 2 | 5) || c.is_ascii_digit());
    }
    false
}

/// True if the line's first stripped word could be a task ID we want to accept
/// (`T-…`, `F-…`, `B-…`, `v0…`, or begins with an ASCII digit for generic IDs).
fn looks_like_task_id_start(r: &str) -> bool {
    r.chars().next().map_or(false, |c| {
        c == 'T' || c == 'F' || c == 'B' || c == 'v' || c.is_ascii_digit()
    })
}

/// Parse `docs/todo.md` tasks. Canonical 5 fields: id | description | effort | priority | status.
/// Tolerant: accepts 3–6+ fields. `status` is always the last field; `id` is always the first.
/// With 4 fields: id | description | effort | status (priority empty).
/// With 3 fields: id | description | status (effort and priority empty).
/// With 6+ fields: description = middle slots joined by ` | ` (e.g. when a feature-id prefix
/// widens the line like `T-037 | F-019 | desc | L | must | open`).
pub fn parse_todo_tasks(content: &str) -> (Vec<TodoTask>, Vec<String>) {
    let mut tasks = Vec::new();
    let mut warnings = Vec::new();
    // T-000109: track the current release-grouping header. The convention is
    // `## v0.32.0 — ...` (or just `## v0.32.0`); subsequent task lines inherit
    // this as their `version` until the next such header. Non-version `##`
    // headers (e.g. `## Format`) don't reset; they're ignored.
    let mut current_version = String::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            // First whitespace-separated token. Accept `v<digit>...` only.
            if let Some(tok) = rest.split_whitespace().next() {
                if tok.starts_with('v') && tok.len() > 1 && tok.as_bytes()[1].is_ascii_digit() {
                    current_version = tok.to_string();
                }
            }
            continue;
        }
        let rest_opt = trimmed.strip_prefix("- [ ] ").or_else(|| {
            trimmed
                .strip_prefix("- ")
                .filter(|r| looks_like_task_id_start(r))
        });
        let Some(rest) = rest_opt else { continue };
        let parts: Vec<String> = split_pipe_respecting_escape(rest)
            .into_iter()
            .map(|s| s.trim().to_string())
            .collect();
        let n = parts.len();
        if n < 3 {
            warnings.push(format!("Unparseable todo line: {}", line));
            continue;
        }
        let id = parts[0].clone();
        let (description, effort, priority, status, created_at) = match n {
            3 => (
                parts[1].clone().replace("\\n", "\n"),
                String::new(),
                String::new(),
                parts[2].clone(),
                String::new(),
            ),
            4 => (
                parts[1].clone().replace("\\n", "\n"),
                parts[2].clone(),
                String::new(),
                parts[3].clone(),
                String::new(),
            ),
            5 => (
                parts[1].clone().replace("\\n", "\n"),
                parts[2].clone(),
                parts[3].clone(),
                parts[4].clone(),
                String::new(),
            ),
            6 => (
                parts[1].clone().replace("\\n", "\n"),
                parts[2].clone(),
                parts[3].clone(),
                parts[4].clone(),
                parts[5].clone(),
            ),
            _ => {
                // n >= 7: last 4 = effort, priority, status, created_at; middle = description
                let desc_slots = &parts[1..n - 4];
                (
                    desc_slots.join(" | ").replace("\\n", "\n"),
                    parts[n - 4].clone(),
                    parts[n - 3].clone(),
                    parts[n - 2].clone(),
                    parts[n - 1].clone(),
                )
            }
        };
        tasks.push(TodoTask {
            id,
            description,
            effort,
            priority,
            status,
            created_at,
            version: current_version.clone(),
        });
    }
    (tasks, warnings)
}

/// Parse `docs/done.md` (v0.13.9+ format).
///
/// Structure:
/// ```
/// ## 2026-04-21             ← date header; applies to tasks below until next header
/// - D-001 | Task X | v0.13.9
/// -  | Task without id | v0.13.9
/// ```
///
/// Line format (3 pipe-separated fields): `- <id?> | <description> | <version>`.
/// - If `id` is empty → assign `D-NNN` from a per-file counter (in-memory only — file is NOT modified)
/// - Tolerant: accepts 2 fields (description + version) or 3 fields (with id)
/// - `date` is inherited from the nearest preceding `## <date>` section header. If none, empty.
/// - `## <anything>` is scanned for a date-like substring (YYYY-MM-DD / DD.MM.YYYY / DD/MM/YYYY),
///   so legacy headers like `## День 29.03.2026` still produce a date.
///
/// Legacy `- [x] ...` rows (checkbox, 4 fields with `commit`) are NOT supported — user rewrites
/// per-project when migrating. Parser tolerates them by extracting what it can; unparseable lines
/// produce warnings.
pub fn parse_done_tasks(content: &str) -> (Vec<DoneTask>, Vec<String>) {
    let mut tasks = Vec::new();
    let mut warnings = Vec::new();
    let mut current_date = String::new();
    let mut auto_counter: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // Section header with date: `## <...date...>`.
        if let Some(rest) = trimmed.strip_prefix("## ") {
            if let Some(d) = rest.split_whitespace().find(|w| is_date_like(w)) {
                current_date = d.trim().to_string();
            }
            continue;
        }

        // Task line: `- ...`. Accept legacy `- [x] ` / `- [X] ` prefixes for transition.
        let rest_opt = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("- [X] "))
            .or_else(|| trimmed.strip_prefix("- "));
        let Some(rest) = rest_opt else { continue };

        let parts: Vec<String> = split_pipe_respecting_escape(rest)
            .into_iter()
            .map(|s| s.trim().to_string())
            .collect();

        let (id_raw, description, version) = match parts.len() {
            // Canonical v0.13.9+ — 3 fields (id slot may be empty but pipes present).
            3 => (parts[0].clone(), parts[1].clone(), parts[2].clone()),
            // Legacy migration tolerance: old `- [x] id | desc | date | commit` (4 fields).
            // Keep id/desc, drop inline date (section-header wins), treat commit as version slot.
            4 => (parts[0].clone(), parts[1].clone(), parts[3].clone()),
            _ => {
                warnings.push(format!(
                    "Unparseable done line ({} fields, expected 3 — id slot may be empty but pipes must be present): {}",
                    parts.len(),
                    line
                ));
                continue;
            }
        };

        let id = if id_raw.is_empty() {
            auto_counter += 1;
            format!("D-{:06}", auto_counter)
        } else {
            id_raw
        };

        tasks.push(DoneTask {
            id,
            description: description.replace("\\n", "\n"),
            date: current_date.clone(),
            version,
        });
    }

    (tasks, warnings)
}

/// Parse docs/done.md and count entries per `## YYYY-MM-DD` section header
/// falling within [start, end] (inclusive). Returns Vec<(date, count)> sorted by date.
/// Missing file → empty vec (not an error).
pub fn parse_done_entries_in_period(
    path: &std::path::Path,
    start: &str,
    end: &str,
) -> std::io::Result<Vec<(String, i64)>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = std::fs::read_to_string(path)?;
    let mut current_date: Option<String> = None;
    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(date_str) = trimmed.strip_prefix("## ") {
            // P5 review-fix: match parse_done_tasks's date tolerance — accept
            // legacy `## День 29.03.2026` headers via `is_date_like` (which
            // recognises ISO + DD.MM.YYYY + DD/MM/YYYY). Normalise everything
            // to YYYY-MM-DD for the range comparison so the Dashboard chart
            // doesn't silently drop tasks from old projects.
            current_date = date_str
                .split_whitespace()
                .find(|w| is_date_like(w))
                .map(|d| {
                    let d = d.trim();
                    let b = d.as_bytes();
                    if b[4] == b'-' {
                        d.to_string()
                    } else {
                        // DD.MM.YYYY or DD/MM/YYYY → YYYY-MM-DD
                        format!("{}-{}-{}", &d[6..10], &d[3..5], &d[0..2])
                    }
                });
        } else if trimmed.starts_with("- ") {
            if let Some(d) = &current_date {
                if d.as_str() >= start && d.as_str() <= end {
                    *counts.entry(d.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    let mut out: Vec<(String, i64)> = counts.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── F-021 TODO / DONE parser tests ────────────────────────────────────────

    #[test]
    fn test_parse_todo_valid() {
        let md = "# Tasks\n\n- [ ] T-001 | Add feature X | M | must | open\n- T-002 | Refactor Y | L | should | in-progress\n";
        let (tasks, warnings) = parse_todo_tasks(md);
        assert!(warnings.is_empty(), "got warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "T-001");
        assert_eq!(tasks[0].description, "Add feature X");
        assert_eq!(tasks[0].effort, "M");
        assert_eq!(tasks[0].priority, "must");
        assert_eq!(tasks[0].status, "open");
        assert_eq!(tasks[1].status, "in-progress");
    }

    #[test]
    fn test_parse_todo_empty_and_malformed() {
        let md = "# Tasks\n\n(no tasks yet)\n- [ ] only two fields\n- [ ] T-005 | Enough fields | S | could | open\n";
        let (tasks, warnings) = parse_todo_tasks(md);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "T-005");
        assert_eq!(warnings.len(), 1, "malformed line should yield one warning");
    }

    #[test]
    fn test_parse_done_v2_section_header_date() {
        let md = "# Done\n\n## 2026-04-21\n- T-001 | Add feature X | v0.13.9\n- T-002 | Bump deps | v0.13.9\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert!(warnings.is_empty(), "got warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "T-001");
        assert_eq!(tasks[0].date, "2026-04-21");
        assert_eq!(tasks[0].version, "v0.13.9");
        assert_eq!(tasks[1].id, "T-002");
        assert_eq!(tasks[1].date, "2026-04-21");
    }

    #[test]
    fn test_parse_done_auto_id_when_empty() {
        // Empty id slot (just `-  | desc | version`) → D-000001, D-000002 assigned sequentially (6-digit).
        let md = "## 2026-04-21\n-  | Task without id | v0.13.9\n-  | Another | v0.13.9\n- T-001 | Has id | v0.13.9\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert!(warnings.is_empty(), "got warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].id, "D-000001");
        assert_eq!(tasks[1].id, "D-000002");
        assert_eq!(
            tasks[2].id, "T-001",
            "explicit legacy 3-digit id kept untouched"
        );
    }

    #[test]
    fn test_parse_done_two_fields_rejected() {
        // Post v0.13.13: 2-field lines are rejected (must be 3 fields, id slot may be empty
        // but the pipes must be present — contract-wise one shape, not two).
        let md = "## 2026-04-21\n- Quick fix | v0.13.9\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert_eq!(tasks.len(), 0);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("expected 3"));
    }

    #[test]
    fn test_parse_done_three_fields_empty_id_accepted() {
        // Canonical 3-field form with empty id slot — still works, gets D-NNNNNN (6-digit).
        let md = "## 2026-04-21\n- | Quick fix | v0.13.9\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert!(warnings.is_empty());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "D-000001");
        assert_eq!(tasks[0].description, "Quick fix");
        assert_eq!(tasks[0].version, "v0.13.9");
    }

    // v0.20.1: id length-leniency tests — parser must accept both legacy 3-digit
    // (T-001 / B-042 / D-001) and new 6-digit zero-padded (T-000001 / etc) IDs.
    #[test]
    fn test_parse_todo_accepts_legacy_3_digit_id() {
        let input = "- [ ] T-001 | Legacy task | 2 | high | open | 2026-04-26\n";
        let (tasks, warnings) = parse_todo_tasks(input);
        assert!(warnings.is_empty());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "T-001");
    }

    #[test]
    fn test_parse_todo_accepts_new_6_digit_id() {
        let input = "- [ ] T-000042 | New task | 2 | high | open | 2026-04-26\n";
        let (tasks, warnings) = parse_todo_tasks(input);
        assert!(warnings.is_empty());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "T-000042");
    }

    #[test]
    fn test_parse_todo_mixed_id_lengths() {
        // 3-digit + 4-digit + 6-digit + F-prefix all coexist.
        let input = "- [ ] T-001 | Legacy | 1 | low | open | 2026-04-20\n\
                     - [ ] T-1234 | 4-digit transitional | 2 | medium | open | 2026-04-21\n\
                     - [ ] T-000042 | 6-digit new | 4 | high | review | 2026-04-26\n\
                     - [ ] F-000007 | feature 6-digit | 8 | high | open | 2026-04-26\n";
        let (tasks, warnings) = parse_todo_tasks(input);
        assert!(warnings.is_empty());
        assert_eq!(tasks.len(), 4);
        assert_eq!(tasks[0].id, "T-001");
        assert_eq!(tasks[1].id, "T-1234");
        assert_eq!(tasks[2].id, "T-000042");
        assert_eq!(tasks[3].id, "F-000007");
    }

    #[test]
    fn test_parse_done_accepts_legacy_3_digit_id() {
        let md = "## 2026-04-26\n- T-045 | Fix login regression | v0.13.9\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert!(warnings.is_empty(), "got warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "T-045");
    }

    // ── T-000109: todo.md `## vX.Y.Z` release-grouping inheritance ──────────────

    #[test]
    fn test_parse_todo_version_header_inheritance() {
        let md = "# Tasks\n\n\
                  ## v0.32.0 — pre-v1.0.0 polish\n\
                  - [ ] T-000109 | Version column | 3 | low | open | 2026-05-14\n\
                  - [ ] T-000110 | Auto-detect | 2 | medium | open | 2026-05-14\n";
        let (tasks, warnings) = parse_todo_tasks(md);
        assert!(warnings.is_empty(), "warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].version, "v0.32.0");
        assert_eq!(tasks[1].version, "v0.32.0");
    }

    #[test]
    fn test_parse_todo_version_header_resets_across_sections() {
        let md = "## v0.32.0 — minor polish\n\
                  - [ ] T-000111 | Bare-multiline hint | 1 | low | open | 2026-05-14\n\
                  ## v1.0.0 — public launch\n\
                  - [ ] T-000064 | Public flip | 3 | high | open | 2026-04-26\n\
                  - [ ] T-000074 | Release closure | 1 | high | open | 2026-05-08\n";
        let (tasks, warnings) = parse_todo_tasks(md);
        assert!(warnings.is_empty(), "warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].version, "v0.32.0");
        assert_eq!(tasks[1].version, "v1.0.0");
        assert_eq!(tasks[2].version, "v1.0.0");
    }

    #[test]
    fn test_parse_todo_no_version_header_yields_empty() {
        let md = "## Format\n- [ ] T-001 | desc | 2 | high | open | 2026-04-26\n";
        let (tasks, warnings) = parse_todo_tasks(md);
        assert!(warnings.is_empty());
        assert_eq!(tasks.len(), 1);
        assert_eq!(
            tasks[0].version, "",
            "non-version `##` header must not set version"
        );
    }

    #[test]
    fn test_parse_todo_tasks_before_first_version_header_yield_empty() {
        let md = "- [ ] T-001 | early task | 1 | low | open | 2026-04-01\n\
                  ## v0.32.0\n\
                  - [ ] T-002 | grouped task | 1 | low | open | 2026-05-14\n";
        let (tasks, warnings) = parse_todo_tasks(md);
        assert!(warnings.is_empty());
        assert_eq!(tasks.len(), 2);
        assert_eq!(
            tasks[0].version, "",
            "task above first version header has no version"
        );
        assert_eq!(tasks[1].version, "v0.32.0");
    }

    #[test]
    fn test_parse_done_accepts_new_6_digit_id() {
        let md = "## 2026-04-26\n- T-000045 | Fix login regression | v0.20.0\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert!(warnings.is_empty(), "got warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "T-000045");
    }

    #[test]
    fn test_parse_done_mixed_legacy_and_new_id_lengths() {
        let md = "## 2026-04-26\n\
                  - D-007 | legacy 3-digit | v0.18.0\n\
                  - T-000045 | new 6-digit | v0.20.0\n\
                  - F-000013 | feature | v0.20.0\n\
                  -  | empty slot synth | v0.20.0\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert!(warnings.is_empty(), "got warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 4);
        assert_eq!(tasks[0].id, "D-007");
        assert_eq!(tasks[1].id, "T-000045");
        assert_eq!(tasks[2].id, "F-000013");
        assert_eq!(tasks[3].id, "D-000001", "synth uses 6-digit");
    }

    #[test]
    fn test_parse_done_legacy_header_extracts_date() {
        // Legacy headers like `## День 29.03.2026` still produce a date (anywhere in header).
        let md = "## День 29.03.2026\n- T-001 | Old entry | v0.1.0\n";
        let (tasks, warnings) = parse_done_tasks(md);
        assert!(warnings.is_empty());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].date, "29.03.2026");
    }

    #[test]
    fn test_parse_done_no_section_header() {
        // Tasks without a preceding `## date` → date is empty.
        let md = "- T-001 | Orphan | v0.1.0\n";
        let (tasks, _warnings) = parse_done_tasks(md);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].date, "");
    }

    #[test]
    fn test_parse_todo_tolerant_4_and_6_fields() {
        // 4 fields: id | description | effort | status (priority empty)
        // 6 fields (new canonical): id | description | effort | priority | status | created_at
        let md = "- [ ] T-010 | Short task | S | open\n- [ ] T-037 | Full rebrand package | L | must | open | 2026-04-26\n";
        let (tasks, warnings) = parse_todo_tasks(md);
        assert!(warnings.is_empty(), "got warnings: {:?}", warnings);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, "T-010");
        assert_eq!(tasks[0].effort, "S");
        assert_eq!(tasks[0].status, "open");
        assert_eq!(tasks[0].priority, "");
        assert_eq!(tasks[1].id, "T-037");
        assert_eq!(tasks[1].description, "Full rebrand package");
        assert_eq!(tasks[1].effort, "L");
        assert_eq!(tasks[1].priority, "must");
        assert_eq!(tasks[1].status, "open");
        assert_eq!(tasks[1].created_at, "2026-04-26");
    }

    /// B-002 retry#4: regression против реального `docs/done.md` этого репо.
    /// Парсер не должен panic'ить, hang'ить или возвращать нереальные результаты
    /// на фактическом файле пользователя (134 строки, смесь форматов: с id и без,
    /// с commit и без, кириллица, старые секционные заголовки `## День DD.MM.YYYY`).
    #[test]
    fn test_parse_done_real_file_does_not_panic() {
        let content = include_str!("../../../docs/done.md");
        let (tasks, warnings) = parse_done_tasks(content);
        eprintln!(
            "done.md stats: {} tasks, {} warnings",
            tasks.len(),
            warnings.len()
        );
        if let Some(first) = tasks.first() {
            eprintln!(
                "first task id={:?} desc={:?} date={:?} version={:?}",
                first.id, first.description, first.date, first.version
            );
        }
        if let Some(last) = tasks.last() {
            eprintln!(
                "last task id={:?} desc={:?} date={:?} version={:?}",
                last.id, last.description, last.date, last.version
            );
        }
        assert!(tasks.len() + warnings.len() > 0);
    }

    #[test]
    fn test_parse_todo_real_file_does_not_panic() {
        let content = include_str!("../../../docs/todo.md");
        let (tasks, warnings) = parse_todo_tasks(content);
        assert!(tasks.len() + warnings.len() > 0);
    }

    /// B-002 retry#4: repro сериализации ReadDoneResult точь-в-точь как в Tauri IPC.
    /// Если serde_json упадёт — это корень зависания (panic в command'е Tauri иногда
    /// не пробрасывается в promise reject, а IPC-канал висит).
    #[test]
    fn test_serialize_done_real_file_to_json() {
        use crate::models::ReadDoneResult;
        let content = include_str!("../../../docs/done.md");
        let (tasks, warnings) = parse_done_tasks(content);
        let result = ReadDoneResult { tasks, warnings };
        let json = serde_json::to_string(&result).expect("serde must not fail");
        eprintln!("JSON size: {} bytes", json.len());
        assert!(json.len() > 0);
    }

    #[test]
    fn test_is_date_like() {
        assert!(is_date_like("2026-04-19"));
        assert!(is_date_like("19.04.2026"));
        assert!(is_date_like("19/04/2026"));
        assert!(!is_date_like("Hello world"));
        assert!(!is_date_like("2026-4-19"));
        assert!(!is_date_like("20260419")); // length-mismatch
    }

    #[test]
    fn test_parse_done_entries_in_period_counts_per_day() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("done.md");
        std::fs::write(
            &path,
            "# Done\n\n\
             ## 2026-04-20\n\
             - T-001 | First task | v0.1\n\
             - T-002 | Second task | v0.1\n\n\
             ## 2026-04-22\n\
             - T-003 | Third | v0.2\n\n\
             ## 2026-04-25\n\
             - T-004 | Outside period | v0.3\n",
        )
        .unwrap();

        let result = parse_done_entries_in_period(&path, "2026-04-20", "2026-04-24").unwrap();
        // Expected: 2026-04-20 -> 2, 2026-04-22 -> 1, others omitted
        assert_eq!(result.len(), 2);
        let map: std::collections::HashMap<String, i64> = result.into_iter().collect();
        assert_eq!(map.get("2026-04-20"), Some(&2));
        assert_eq!(map.get("2026-04-22"), Some(&1));
        assert!(!map.contains_key("2026-04-25"));
    }

    #[test]
    fn test_parse_done_entries_missing_file_returns_empty() {
        let r = parse_done_entries_in_period(
            std::path::Path::new("/no/such/file.md"),
            "2026-04-01",
            "2026-04-30",
        )
        .unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn test_parse_todo_6_fields_new_format() {
        let input = "- [ ] T-042 | Add Go deploy template | 4 | high | open | 2026-04-26\n";
        let (tasks, warnings) = parse_todo_tasks(input);
        assert!(warnings.is_empty(), "no warnings for valid 6-field");
        assert_eq!(tasks.len(), 1);
        let t = &tasks[0];
        assert_eq!(t.id, "T-042");
        assert_eq!(t.description, "Add Go deploy template");
        assert_eq!(t.effort, "4");
        assert_eq!(t.priority, "high");
        assert_eq!(t.status, "open");
        assert_eq!(t.created_at, "2026-04-26");
    }

    #[test]
    fn test_parse_todo_5_fields_legacy_empty_created_at() {
        let input = "- [ ] T-001 | Legacy task | 2 | medium | open\n";
        let (tasks, _) = parse_todo_tasks(input);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].created_at, "");
        assert_eq!(tasks[0].status, "open");
    }

    #[test]
    fn test_parse_todo_mixed_5_and_6_field_lines() {
        let input = "- [ ] T-001 | Legacy task | 2 | medium | open\n- [ ] T-002 | New task | 4 | high | review | 2026-04-26\n";
        let (tasks, _) = parse_todo_tasks(input);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].created_at, "");
        assert_eq!(tasks[1].created_at, "2026-04-26");
    }

    #[test]
    fn test_parse_todo_7_or_more_fields_joined_into_description() {
        let input =
            "- [ ] T-001 | Multi pipe \\| not escaped | wat | 4 | high | open | 2026-04-26\n";
        let (tasks, _) = parse_todo_tasks(input);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "T-001");
        assert_eq!(tasks[0].effort, "4");
        assert_eq!(tasks[0].priority, "high");
        assert_eq!(tasks[0].status, "open");
        assert_eq!(tasks[0].created_at, "2026-04-26");
    }
}
