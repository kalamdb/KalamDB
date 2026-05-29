use crate::session::CLISession;

pub(super) fn split_batch_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();

    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_backtick = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;

    while let Some(ch) = chars.next() {
        if in_line_comment {
            current.push(ch);
            if ch == '\n' {
                in_line_comment = false;
            }
            continue;
        }

        if in_block_comment {
            current.push(ch);
            if ch == '*' && chars.peek() == Some(&'/') {
                current.push(chars.next().expect("peeked slash should be available"));
                in_block_comment = false;
            }
            continue;
        }

        if !in_single_quote && !in_double_quote && !in_backtick {
            if ch == '-' && chars.peek() == Some(&'-') && is_sql_line_comment_start(&current, &chars)
            {
                current.push(ch);
                current.push(chars.next().expect("peeked dash should be available"));
                in_line_comment = true;
                continue;
            }

            if ch == '/' && chars.peek() == Some(&'*') {
                current.push(ch);
                current.push(chars.next().expect("peeked star should be available"));
                in_block_comment = true;
                continue;
            }
        }

        match ch {
            '\'' if !in_double_quote && !in_backtick => {
                current.push(ch);
                if in_single_quote && chars.peek() == Some(&'\'') {
                    current.push(chars.next().expect("peeked quote should be available"));
                } else {
                    in_single_quote = !in_single_quote;
                }
            },
            '"' if !in_single_quote && !in_backtick => {
                current.push(ch);
                if in_double_quote && chars.peek() == Some(&'"') {
                    current.push(chars.next().expect("peeked quote should be available"));
                } else {
                    in_double_quote = !in_double_quote;
                }
            },
            '`' if !in_single_quote && !in_double_quote => {
                in_backtick = !in_backtick;
                current.push(ch);
            },
            ';' if !(in_single_quote || in_double_quote || in_backtick) => {
                let stmt = current.trim();
                if !strip_leading_batch_comments(stmt).is_empty() {
                    statements.push(stmt.to_string());
                }
                current.clear();
            },
            _ => current.push(ch),
        }
    }

    let trailing = current.trim();
    if !strip_leading_batch_comments(trailing).is_empty() {
        statements.push(trailing.to_string());
    }

    statements
}

pub(super) fn batch_table_readiness_target(statement: &str) -> Option<String> {
    let trimmed = strip_leading_batch_comments(statement.trim().trim_end_matches(';'));
    if trimmed.is_empty() {
        return None;
    }

    let remainder = if let Some(rest) = CLISession::strip_ascii_prefix(trimmed, "CREATE USER TABLE")
    {
        rest
    } else if let Some(rest) = CLISession::strip_ascii_prefix(trimmed, "CREATE SHARED TABLE") {
        rest
    } else if let Some(rest) = CLISession::strip_ascii_prefix(trimmed, "CREATE STREAM TABLE") {
        rest
    } else if let Some(rest) = CLISession::strip_ascii_prefix(trimmed, "CREATE TABLE") {
        rest
    } else {
        return None;
    };

    let remainder = remainder.trim_start();
    let remainder = if let Some(rest) = CLISession::strip_ascii_prefix(remainder, "IF NOT EXISTS") {
        rest.trim_start()
    } else {
        remainder
    };

    let mut name = String::new();
    let mut in_quote: Option<char> = None;

    for ch in remainder.chars() {
        if let Some(quote) = in_quote {
            name.push(ch);
            if ch == quote {
                in_quote = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' | '`' => {
                in_quote = Some(ch);
                name.push(ch);
            },
            '(' | ';' => break,
            c if c.is_whitespace() => break,
            _ => name.push(ch),
        }
    }

    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn strip_leading_batch_comments(mut value: &str) -> &str {
    loop {
        let trimmed = value.trim_start();
        if let Some(rest) = trimmed.strip_prefix("--") {
            let Some(newline_index) = rest.find('\n') else {
                return "";
            };
            value = &rest[newline_index + 1..];
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("/*") {
            let Some(end_index) = rest.find("*/") else {
                return "";
            };
            value = &rest[end_index + 2..];
            continue;
        }

        return trimmed;
    }
}

fn is_sql_line_comment_start(current: &str, chars: &std::iter::Peekable<std::str::Chars<'_>>) -> bool {
    let prev = current.chars().last();
    let prev_ok = prev.is_none()
        || prev.is_some_and(|c| c.is_whitespace() || matches!(c, ';' | '(' | ')' | ','));
    if !prev_ok {
        return false;
    }

    let mut lookahead = chars.clone();
    let _second_dash = lookahead.next();
    let after_second_dash = lookahead.peek().copied();
    after_second_dash.is_none() || after_second_dash.is_some_and(char::is_whitespace)
}
