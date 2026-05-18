use std::{
    path::Path,
    time::{Duration, Instant},
};

use kalam_client::KalamLinkError;

use super::CLISession;
use crate::error::{CLIError, Result};

impl CLISession {
    /// Execute SQL statements from a file using the same batch importer as `--file`.
    pub async fn execute_file(&mut self, path: &Path) -> Result<()> {
        let sql = tokio::fs::read_to_string(path).await.map_err(|e| {
            CLIError::FileError(format!("Failed to read {}: {}", path.display(), e))
        })?;

        self.execute_batch(&sql).await
    }

    /// Execute multiple SQL statements from a script.
    pub async fn execute_batch(&mut self, sql: &str) -> Result<()> {
        for statement in Self::split_batch_statements(sql) {
            self.execute(&statement).await?;

            if let Some(table) = Self::batch_table_readiness_target(&statement) {
                self.wait_for_batch_table_ready(&table).await?;
            }
        }
        Ok(())
    }

    pub(in crate::session) fn split_batch_statements(sql: &str) -> Vec<String> {
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
                if ch == '-' && chars.peek() == Some(&'-') {
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
                    if !stmt.is_empty() {
                        statements.push(stmt.to_string());
                    }
                    current.clear();
                },
                _ => current.push(ch),
            }
        }

        let trailing = current.trim();
        if !trailing.is_empty() {
            statements.push(trailing.to_string());
        }

        statements
    }

    pub(in crate::session) fn batch_table_readiness_target(statement: &str) -> Option<String> {
        let trimmed = Self::strip_leading_batch_comments(statement.trim().trim_end_matches(';'));
        if trimmed.is_empty() {
            return None;
        }

        let remainder = if let Some(rest) = Self::strip_ascii_prefix(trimmed, "CREATE USER TABLE") {
            rest
        } else if let Some(rest) = Self::strip_ascii_prefix(trimmed, "CREATE SHARED TABLE") {
            rest
        } else if let Some(rest) = Self::strip_ascii_prefix(trimmed, "CREATE STREAM TABLE") {
            rest
        } else if let Some(rest) = Self::strip_ascii_prefix(trimmed, "CREATE TABLE") {
            rest
        } else {
            return None;
        };

        let remainder = remainder.trim_start();
        let remainder = if let Some(rest) = Self::strip_ascii_prefix(remainder, "IF NOT EXISTS") {
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

    async fn wait_for_batch_table_ready(&mut self, table: &str) -> Result<()> {
        let started = Instant::now();
        let timeout = Duration::from_secs(5);
        let probe_sql = format!("SELECT * FROM {} LIMIT 1", table);
        let mut last_error = None;

        while started.elapsed() < timeout {
            match self.execute_query_response(&probe_sql).await {
                Ok(_) => return Ok(()),
                Err(err) => {
                    last_error = Some(err.to_string());
                    tokio::time::sleep(Duration::from_millis(50)).await;
                },
            }
        }

        Err(CLIError::LinkError(KalamLinkError::TimeoutError(format!(
            "Timed out waiting for table {} to become queryable after DDL. Last error: {}",
            table,
            last_error.unwrap_or_else(|| "<none>".to_string())
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::CLISession;

    #[test]
    fn split_batch_statements_preserves_comments_and_literals() {
        let statements = CLISession::split_batch_statements(
            "-- leading comment; not a statement\nCREATE TABLE demo.t (id BIGINT PRIMARY KEY, msg TEXT);\n/* block ; comment */\nEXECUTE AS USER 'alice' (INSERT INTO demo.t (id, msg) VALUES (1, 'hello; -- still literal'));\nSELECT * FROM demo.t;",
        );

        assert_eq!(statements.len(), 3);
        assert!(statements[0].contains("CREATE TABLE demo.t"));
        assert!(statements[1].starts_with("/* block ; comment */\nEXECUTE AS USER 'alice'"));
        assert!(statements[1].contains("hello; -- still literal"));
        assert_eq!(statements[2], "SELECT * FROM demo.t");
    }

    #[test]
    fn readiness_target_detects_create_table_forms() {
        assert_eq!(
            CLISession::batch_table_readiness_target(
                "-- schema setup\nCREATE USER TABLE IF NOT EXISTS demo.activity_feed (id BIGINT PRIMARY KEY)",
            ),
            Some("demo.activity_feed".to_string())
        );
        assert_eq!(
            CLISession::batch_table_readiness_target(
                "CREATE SHARED TABLE \"demo\".\"activity feed\" (id BIGINT PRIMARY KEY)",
            ),
            Some("\"demo\".\"activity feed\"".to_string())
        );
        assert_eq!(
            CLISession::batch_table_readiness_target(
                "EXECUTE AS USER 'demo-user' (CREATE TABLE demo.t (id BIGINT PRIMARY KEY))",
            ),
            None
        );
    }
}
