//! Shared `-- UP` / `-- DOWN` section parsing for migration SQL files.

pub(crate) fn extract_up_section(sql: &str) -> String {
    extract_up_section_borrowed(sql).to_string()
}

pub(crate) fn extract_up_section_borrowed(sql: &str) -> &str {
    extract_between_markers(sql, "-- UP", "-- DOWN").unwrap_or_else(|| sql.trim())
}

pub(crate) fn extract_between_markers<'a>(
    sql: &'a str,
    start_marker: &str,
    end_marker: &str,
) -> Option<&'a str> {
    let start = find_marker_line_end(sql, start_marker)?;
    let rest = &sql[start..];
    let end = find_marker_line_start(rest, end_marker).unwrap_or(rest.len());
    Some(rest[..end].trim())
}

fn find_marker_line_end(sql: &str, marker: &str) -> Option<usize> {
    let mut offset = 0usize;
    for segment in sql.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        if line.trim().eq_ignore_ascii_case(marker) {
            return Some(offset + segment.len());
        }
        offset += segment.len();
    }
    if sql[offset..].trim().eq_ignore_ascii_case(marker) {
        return Some(sql.len());
    }
    None
}

fn find_marker_line_start(sql: &str, marker: &str) -> Option<usize> {
    let mut offset = 0usize;
    for segment in sql.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        if line.trim().eq_ignore_ascii_case(marker) {
            return Some(offset);
        }
        offset += segment.len();
    }
    if sql[offset..].trim().eq_ignore_ascii_case(marker) {
        return Some(offset);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MIGRATION: &str = "-- Migration: draft\n-- Updated: 2026-06-08T17:45:18Z\n\n-- UP\nCREATE TABLE users (id INTEGER);\n\n-- DOWN\nDROP TABLE users;";

    #[test]
    fn extract_up_section_ignores_updated_header() {
        assert_eq!(extract_up_section(SAMPLE_MIGRATION), "CREATE TABLE users (id INTEGER);");
    }

    #[test]
    fn extract_up_section_borrowed_matches_owned_extraction() {
        assert_eq!(
            extract_up_section_borrowed(SAMPLE_MIGRATION),
            "CREATE TABLE users (id INTEGER);"
        );
    }
}
