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
    let spans = top_level_line_spans(sql);
    let start = spans
        .iter()
        .find(|span| span.trimmed_line.eq_ignore_ascii_case(start_marker))
        .map(|span| span.line_end)?;
    let end = spans
        .iter()
        .filter(|span| span.line_start >= start)
        .find(|span| span.trimmed_line.eq_ignore_ascii_case(end_marker))
        .map(|span| span.line_start)
        .unwrap_or(sql.len());
    Some(sql[start..end].trim())
}

#[derive(Debug, Clone, Copy)]
struct TopLevelLineSpan<'a> {
    line_start: usize,
    line_end: usize,
    trimmed_line: &'a str,
}

/// Collect lines whose start position is at top-level SQL state (outside single quotes,
/// double quotes, dollar-quoted strings `$$` / `$tag$`, and `/* ... */` block comments).
fn top_level_line_spans(sql: &str) -> Vec<TopLevelLineSpan<'_>> {
    #[derive(Clone, PartialEq, Eq)]
    enum LexState<'a> {
        Normal,
        SingleQuote,
        DoubleQuote,
        BlockComment(usize),
        DollarQuote(&'a str),
    }

    let bytes = sql.as_bytes();
    let mut state = LexState::Normal;
    let mut spans = Vec::new();
    let mut offset = 0usize;

    for segment in sql.split_inclusive('\n') {
        let line_start = offset;
        let line_end = offset + segment.len();
        let line = segment.trim_end_matches(['\r', '\n']);

        if state == LexState::Normal {
            spans.push(TopLevelLineSpan {
                line_start,
                line_end,
                trimmed_line: line.trim(),
            });
        }

        // Advance lex state through this line segment
        let mut i = line_start;
        while i < line_end {
            match state {
                LexState::Normal => {
                    if i + 1 < line_end && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                        // Line comment runs until end of this line segment
                        break;
                    }
                    if i + 1 < line_end && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        state = LexState::BlockComment(1);
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\'' {
                        state = LexState::SingleQuote;
                        i += 1;
                        continue;
                    }
                    if bytes[i] == b'"' {
                        state = LexState::DoubleQuote;
                        i += 1;
                        continue;
                    }
                    if bytes[i] == b'$' {
                        if let Some(tag) = match_dollar_quote_tag(&sql[i..line_end]) {
                            state = LexState::DollarQuote(tag);
                            i += tag.len();
                            continue;
                        }
                    }
                    i += 1;
                },
                LexState::SingleQuote => {
                    if bytes[i] == b'\'' {
                        if i + 1 < line_end && bytes[i + 1] == b'\'' {
                            i += 2;
                            continue;
                        }
                        state = LexState::Normal;
                    }
                    i += 1;
                },
                LexState::DoubleQuote => {
                    if bytes[i] == b'"' {
                        if i + 1 < line_end && bytes[i + 1] == b'"' {
                            i += 2;
                            continue;
                        }
                        state = LexState::Normal;
                    }
                    i += 1;
                },
                LexState::BlockComment(depth) => {
                    if i + 1 < line_end && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        state = LexState::BlockComment(depth + 1);
                        i += 2;
                        continue;
                    }
                    if i + 1 < line_end && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        if depth <= 1 {
                            state = LexState::Normal;
                        } else {
                            state = LexState::BlockComment(depth - 1);
                        }
                        i += 2;
                        continue;
                    }
                    i += 1;
                },
                LexState::DollarQuote(tag) => {
                    if sql[i..line_end].starts_with(tag) {
                        state = LexState::Normal;
                        i += tag.len();
                        continue;
                    }
                    i += 1;
                },
            }
        }

        offset = line_end;
    }

    spans
}

fn match_dollar_quote_tag(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'$') {
        return None;
    }
    if bytes.get(1) == Some(&b'$') {
        return Some(&s[..2]);
    }
    let first = *bytes.get(1)?;
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return None;
    }
    let mut idx = 2usize;
    while let Some(&b) = bytes.get(idx) {
        if b == b'$' {
            return Some(&s[..=idx]);
        }
        if !(b.is_ascii_alphanumeric() || b == b'_') {
            return None;
        }
        idx += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MIGRATION: &str = "-- Migration: draft\n-- Updated: 2026-06-08T17:45:18Z\n\n-- \
                                    UP\nCREATE TABLE users (id INTEGER);\n\n-- DOWN\nDROP TABLE \
                                    users;";

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

    #[test]
    fn extract_up_section_ignores_markers_inside_dollar_quotes_strings_and_block_comments() {
        let migration = r#"-- Migration: routine
-- UP
CREATE OR REPLACE FUNCTION app.demo()
RETURNS TEXT
LANGUAGE SQL
SECURITY INVOKER
AS $body$
-- DOWN
SELECT '-- DOWN' AS marker;
$body$;
/*
-- DOWN
*/
INSERT INTO notes (body) VALUES ('
-- DOWN
still inside string');
-- DOWN
DROP FUNCTION app.demo;
"#;
        let up = extract_up_section(migration);
        assert!(up.contains("CREATE OR REPLACE FUNCTION app.demo()"), "{up}");
        assert!(up.contains("INSERT INTO notes"), "{up}");
        assert!(!up.contains("DROP FUNCTION app.demo"), "{up}");
    }
}
