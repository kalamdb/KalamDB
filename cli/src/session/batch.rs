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
        for statement in Self::split_batch_statements(sql) {
            self.execute(&statement).await?;

            if let Some(table) = Self::batch_table_readiness_target(&statement) {
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
