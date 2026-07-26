/// Read a (possibly user-edited) text file tolerating invalid UTF-8. A file
/// saved as UTF-16 / Windows-1251 / with a stray byte decodes lossily (invalid
/// sequences → U+FFFD) instead of erroring — one mis-encoded todo.md / done.md /
/// bug-reports.md must never crash its read or blank the Dashboard. IO errors
/// (missing file, permissions) still propagate. Shared by every user-editable-MD
/// read site so the whole app is uniformly tolerant.
pub fn read_to_string_lossy(path: &std::path::Path) -> std::io::Result<String> {
    Ok(String::from_utf8_lossy(&std::fs::read(path)?).into_owned())
}

/// F-026 helper: split by `|` but treat `\|` as an escaped literal pipe.
/// Replacement for naive `str::split('|')` / `splitn(N, '|')` that breaks when
/// a field contains a legit `|` (e.g. regex `/a|b/` in a bug description).
/// Shared across bug-reports, todo and done parsers (all pipe-separated formats).
pub fn split_pipe_respecting_escape(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if next == '|' {
                    chars.next();
                    current.push('|');
                    continue;
                }
            }
            current.push(c);
        } else if c == '|' {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
}

/// Escape a field value for pipe-separated MD:
///   literal `|` → `\|` (so parser can tell it apart from field separator)
///   literal `\n` → `\\n` (preserve newlines in single-line field)
/// Order matters: pipe first, then newline, so `|` in description with newlines works.
pub fn escape_field(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', "\\n")
}

/// Reverse `escape_field` on parse: unescape `\n` then `\|`.
pub fn unescape_field(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\|", "|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_pipe_respecting_escape() {
        let parts = split_pipe_respecting_escape("a | b\\|c | d");
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].trim(), "a");
        assert_eq!(parts[1].trim(), "b|c");
        assert_eq!(parts[2].trim(), "d");
    }

    #[test]
    fn test_read_to_string_lossy_tolerates_invalid_utf8() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("todo.md");
        // Valid ASCII around a lone invalid continuation byte (0xFF) — read_to_string
        // would error ("stream did not contain valid UTF-8"); lossy must not.
        std::fs::write(&path, [b'-', b' ', b'x', 0xFF, b'\n', b'o', b'k']).unwrap();
        let content = read_to_string_lossy(&path).expect("lossy read must not error");
        assert!(content.contains("- x"), "ascii preserved: {:?}", content);
        assert!(content.contains("ok"));
        assert!(
            content.contains('\u{FFFD}'),
            "invalid byte → replacement char"
        );
    }

    #[test]
    fn test_read_to_string_lossy_missing_file_still_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        // IO errors (missing file) still propagate — only encoding is tolerated.
        assert!(read_to_string_lossy(&tmp.path().join("nope.md")).is_err());
    }
}
