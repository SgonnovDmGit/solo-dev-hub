//! v1.8.0 (T-000140): CSV formatter for the deploy report.
//!
//! The frontend flattens each displayed report row into a `DeployReportCsvRow`
//! (8 cells) and passes a `Vec` of them to the backend; this module only
//! formats them, the command layer writes the file. Keeping the formatter here
//! (rather than inline in the command) makes escaping cargo-testable.
//!
//! Delimiter is `;` (not `,`): RU/EU-locale Excel uses the semicolon as its list
//! separator, so a comma-delimited file lands entirely in column A. Fields are
//! still quoted per RFC4180 rules (quote when the field contains the delimiter,
//! a double-quote, or a line break; embedded `"` doubled).

use crate::models::DeployReportCsvRow;

/// Field delimiter. Semicolon for Excel locale compatibility (see module docs).
const DELIM: char = ';';

/// Stable English header keys, in the fixed column order.
const HEADER: [&str; 8] = [
    "repo",
    "environment",
    "domain",
    "branch",
    "image_tag",
    "db_name",
    "secrets_count",
    "updated_at",
];

/// RFC4180-style field escaping: if the field contains the delimiter (`;`), a
/// double-quote, a carriage return, or a newline, wrap it in double quotes and
/// escape embedded `"` as `""`. Fields without those characters are emitted raw
/// (a plain comma no longer forces quoting, since `,` is not the delimiter).
fn escape_csv_field(field: &str) -> String {
    if field.contains(DELIM) || field.contains('"') || field.contains('\r') || field.contains('\n')
    {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Format the deploy-report rows as a semicolon-delimited CSV string.
///
/// - First line = the stable English header.
/// - One line per row, same column order; `secrets_count` rendered as its integer.
/// - Records joined with `\r\n`; no trailing newline after the last record.
/// - Empty input → header-only output (no data rows, no trailing newline).
pub fn deploy_report_to_csv(rows: &[DeployReportCsvRow]) -> String {
    let sep = DELIM.to_string();
    let mut lines: Vec<String> = Vec::with_capacity(rows.len() + 1);

    lines.push(
        HEADER
            .iter()
            .map(|h| escape_csv_field(h))
            .collect::<Vec<_>>()
            .join(&sep),
    );

    for row in rows {
        let cells = [
            escape_csv_field(&row.repo),
            escape_csv_field(&row.environment),
            escape_csv_field(&row.domain),
            escape_csv_field(&row.branch),
            escape_csv_field(&row.image_tag),
            escape_csv_field(&row.db_name),
            escape_csv_field(&row.secrets_count.to_string()),
            escape_csv_field(&row.updated_at),
        ];
        lines.push(cells.join(&sep));
    }

    lines.join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_STR: &str =
        "repo;environment;domain;branch;image_tag;db_name;secrets_count;updated_at";

    fn plain_row() -> DeployReportCsvRow {
        DeployReportCsvRow {
            repo: "web-app".to_string(),
            environment: "prod".to_string(),
            domain: "x.com".to_string(),
            branch: "master".to_string(),
            image_tag: "latest".to_string(),
            db_name: "appdb".to_string(),
            secrets_count: 3,
            updated_at: "2026-07-02T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_header_present_exact_and_ordered() {
        let out = deploy_report_to_csv(&[]);
        assert_eq!(out, HEADER_STR);
        // First line matches exactly (defensive against future trailing rows).
        assert_eq!(out.lines().next().unwrap(), HEADER_STR);
    }

    #[test]
    fn test_empty_input_header_only_no_trailing_newline() {
        let out = deploy_report_to_csv(&[]);
        assert_eq!(out, HEADER_STR);
        assert!(!out.ends_with('\n'));
        assert!(!out.ends_with("\r\n"));
        // Header only → exactly one line, zero data rows.
        assert_eq!(out.split("\r\n").count(), 1);
    }

    #[test]
    fn test_plain_row_emitted_raw_semicolon_separated() {
        let out = deploy_report_to_csv(&[plain_row()]);
        let expected = format!(
            "{}\r\nweb-app;prod;x.com;master;latest;appdb;3;2026-07-02T00:00:00Z",
            HEADER_STR
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn test_field_with_semicolon_is_quoted() {
        let mut row = plain_row();
        row.domain = "a.com; b.com".to_string();
        let out = deploy_report_to_csv(&[row]);
        assert!(out.contains("\"a.com; b.com\""));
    }

    #[test]
    fn test_field_with_comma_stays_raw() {
        // Comma is NOT the delimiter → must not be quoted.
        let mut row = plain_row();
        row.domain = "a.com, b.com".to_string();
        let out = deploy_report_to_csv(&[row]);
        assert!(out.contains(";a.com, b.com;"));
        assert!(!out.contains("\"a.com, b.com\""));
    }

    #[test]
    fn test_field_with_double_quote_is_quoted_and_doubled() {
        let mut row = plain_row();
        row.db_name = "he said \"hi\"".to_string();
        let out = deploy_report_to_csv(&[row]);
        // Embedded " doubled, whole field wrapped.
        assert!(out.contains("\"he said \"\"hi\"\"\""));
    }

    #[test]
    fn test_field_with_newline_is_quoted() {
        let mut row = plain_row();
        row.db_name = "line1\nline2".to_string();
        let out = deploy_report_to_csv(&[row]);
        assert!(out.contains("\"line1\nline2\""));
    }

    #[test]
    fn test_field_with_carriage_return_is_quoted() {
        let mut row = plain_row();
        row.domain = "a\rb".to_string();
        let out = deploy_report_to_csv(&[row]);
        assert!(out.contains("\"a\rb\""));
    }

    #[test]
    fn test_secrets_count_numeric_rendering() {
        let mut zero = plain_row();
        zero.secrets_count = 0;
        let mut twelve = plain_row();
        twelve.secrets_count = 12;
        let out = deploy_report_to_csv(&[zero, twelve]);
        let data_lines: Vec<&str> = out.split("\r\n").skip(1).collect();
        assert!(data_lines[0].ends_with(";0;2026-07-02T00:00:00Z"));
        assert!(data_lines[1].ends_with(";12;2026-07-02T00:00:00Z"));
    }

    #[test]
    fn test_line_count_is_n_plus_one() {
        let rows = vec![plain_row(), plain_row(), plain_row()];
        let out = deploy_report_to_csv(&rows);
        assert_eq!(out.split("\r\n").count(), rows.len() + 1);
    }

    #[test]
    fn test_escape_csv_field_leaves_safe_field_raw() {
        assert_eq!(escape_csv_field("plain"), "plain");
        assert_eq!(escape_csv_field(""), "");
    }
}
