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
}
