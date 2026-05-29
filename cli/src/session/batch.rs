use std::{
    path::Path,
    time::{Duration, Instant},
};

use kalam_client::KalamLinkError;

use super::CLISession;
use crate::error::{CLIError, Result};

mod parser;

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
        let statements = Self::coalesce_explicit_transaction_blocks(Self::split_batch_statements(sql))?;
        for statement in statements {
            let dispatch = Self::strip_leading_batch_comments(&statement);
            if dispatch.is_empty() {
                continue;
            }

            self.execute_input(dispatch).await?;

            if let Some(table) = Self::batch_table_readiness_target(dispatch) {
                self.wait_for_batch_table_ready(&table).await?;
            }
        }
        Ok(())
    }

    pub(in crate::session) fn split_batch_statements(sql: &str) -> Vec<String> {
        parser::split_batch_statements(sql)
    }

    pub(in crate::session) fn batch_table_readiness_target(statement: &str) -> Option<String> {
        parser::batch_table_readiness_target(statement)
    }

    fn coalesce_explicit_transaction_blocks(statements: Vec<String>) -> Result<Vec<String>> {
        let mut merged = Vec::with_capacity(statements.len());
        let mut in_tx = false;
        let mut tx_buffer = String::new();

        for statement in statements {
            if !in_tx {
                if Self::is_explicit_transaction_begin(&statement) {
                    in_tx = true;
                    tx_buffer.push_str(statement.trim());
                    tx_buffer.push_str(";\n");
                } else {
                    merged.push(statement);
                }
                continue;
            }

            tx_buffer.push_str(statement.trim());
            tx_buffer.push_str(";\n");

            if Self::is_explicit_transaction_end(&statement) {
                merged.push(tx_buffer.trim_end().to_string());
                tx_buffer.clear();
                in_tx = false;
            }
        }

        if in_tx {
            return Err(CLIError::ParseError(
                "SQL file contains an explicit transaction block without COMMIT/ROLLBACK"
                    .to_string(),
            ));
        }

        Ok(merged)
    }

    fn is_explicit_transaction_begin(statement: &str) -> bool {
        let trimmed = Self::strip_leading_batch_comments(statement);
        Self::strip_ascii_prefix(trimmed, "BEGIN").is_some()
            || Self::strip_ascii_prefix(trimmed, "START TRANSACTION").is_some()
    }

    fn is_explicit_transaction_end(statement: &str) -> bool {
        let trimmed = Self::strip_leading_batch_comments(statement);
        Self::strip_ascii_prefix(trimmed, "COMMIT").is_some()
            || Self::strip_ascii_prefix(trimmed, "ROLLBACK").is_some()
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
    fn split_batch_statements_skips_comment_only_trailing_content() {
        let statements = CLISession::split_batch_statements(
            "CREATE TABLE demo.t (id BIGINT PRIMARY KEY);\n-- trailing comment only\n-- another comment\n",
        );

        assert_eq!(statements.len(), 1);
        assert_eq!(statements[0], "CREATE TABLE demo.t (id BIGINT PRIMARY KEY)");
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

    #[test]
    fn coalesce_transaction_block_merges_begin_commit() {
        let statements = vec![
            "CREATE TABLE demo.t (id BIGINT PRIMARY KEY)".to_string(),
            "BEGIN".to_string(),
            "INSERT INTO demo.t (id) VALUES (1)".to_string(),
            "UPDATE demo.t SET id = 2 WHERE id = 1".to_string(),
            "COMMIT".to_string(),
            "SELECT * FROM demo.t".to_string(),
        ];

        let merged = CLISession::coalesce_explicit_transaction_blocks(statements).unwrap();
        assert_eq!(merged.len(), 3);
        assert!(merged[1].starts_with("BEGIN;"));
        assert!(merged[1].contains("INSERT INTO demo.t"));
        assert!(merged[1].contains("COMMIT;"));
    }

    #[test]
    fn coalesce_transaction_block_requires_end_marker() {
        let statements = vec![
            "BEGIN".to_string(),
            "INSERT INTO demo.t (id) VALUES (1)".to_string(),
        ];

        let err = CLISession::coalesce_explicit_transaction_blocks(statements).unwrap_err();
        assert!(
            err.to_string()
                .contains("explicit transaction block without COMMIT/ROLLBACK")
        );
    }

    #[test]
    fn split_batch_keeps_export_and_create_separate() {
        let sql = "-- Export table\nexport demo.t --output /tmp/demo.zip;\n\n-- Create target\nCREATE TABLE demo_copy (id BIGINT PRIMARY KEY);\n";
        let statements = CLISession::split_batch_statements(sql);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("export demo.t --output /tmp/demo.zip"));
        assert!(statements[1].contains("CREATE TABLE demo_copy"));
    }
}
