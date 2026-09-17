//! system.server_logs virtual view
//!
//! **Type**: Virtual View (not backed by persistent storage)
//!
//! Provides read access to the latest JSON Lines from `server.jsonl`.
//! Requires `format = "json"` in logging configuration.
//!
//! **DataFusion Pattern**: Implements VirtualView trait for consistent view behavior
//! - Tails via [`crate::jsonl_tail`] from EOF in small chunks (no full-file read)
//! - Returns at most the most recent parsed lines; a 1GB log stays on disk
//! - No memory consumption when idle
//!
//! **Schema Caching**: Memoized via `OnceLock`
//! **Schema**: TableDefinition provides consistent metadata for views

use std::{path::PathBuf, sync::Arc};

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefault, ColumnDefinition, TableDefinition},
};
use kalamdb_system::SystemTable;

use super::common::{system_view_definition, view_provider_with, SystemViewProvider};
use crate::{
    error::RegistryError,
    jsonl_tail::{parse_json_log_line, read_jsonl_tail, JsonlTailLimits},
    view_base::VirtualView,
};

crate::memoized_view_schema!(server_logs_schema, ServerLogsView);

const LOG_TAIL: JsonlTailLimits = JsonlTailLimits::DEFAULT;
#[cfg(test)]
const SERVER_LOG_MAX_ROWS: usize = LOG_TAIL.max_rows;
#[cfg(test)]
const SERVER_LOG_MAX_LINE_BYTES: usize = LOG_TAIL.max_line_bytes;

/// Parsed server log entry exposed through `system.server_logs`.
#[derive(Debug)]
struct JsonLogEntry {
    timestamp: String,
    level:     String,
    thread:    Option<String>,
    target:    Option<String>,
    line:      Option<i64>,
    message:   String,
}

/// Raw JSON log entry structure.
///
/// `tracing-subscriber` JSON output nests event fields under `fields`, while
/// older/manual log lines may already expose `message` at the root.
#[derive(Debug, serde::Deserialize)]
struct RawJsonLogEntry {
    timestamp: Option<String>,
    level:     Option<String>,
    #[serde(alias = "threadName")]
    thread:    Option<String>,
    target:    Option<String>,
    line:      Option<i64>,
    message:   Option<String>,
    fields:    Option<RawJsonLogFields>,
}

#[derive(Debug, serde::Deserialize)]
struct RawJsonLogFields {
    message: Option<String>,
}

impl RawJsonLogEntry {
    fn into_entry(self) -> Option<JsonLogEntry> {
        Some(JsonLogEntry {
            timestamp: self.timestamp?,
            level:     self.level?,
            thread:    self.thread,
            target:    self.target,
            line:      self.line,
            message:   self.message.or_else(|| self.fields.and_then(|fields| fields.message))?,
        })
    }
}

/// ServerLogsView - Reads server log files dynamically
#[derive(Debug)]
pub struct ServerLogsView {
    logs_path: PathBuf,
}

impl ServerLogsView {
    /// Get the TableDefinition for system.server_logs view
    ///
    /// Schema:
    /// - timestamp TEXT NOT NULL (log timestamp)
    /// - level TEXT NOT NULL (log level: DEBUG, INFO, WARN, ERROR)
    /// - thread TEXT (nullable - thread name)
    /// - target TEXT (nullable - module/target name)
    /// - line BIGINT (nullable - source line number)
    /// - message TEXT NOT NULL (log message content)
    pub fn definition() -> TableDefinition {
        let columns = vec![
            ColumnDefinition::new(
                1,
                "timestamp",
                1,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Log entry timestamp (ISO 8601 format)".to_string()),
            ),
            ColumnDefinition::new(
                2,
                "level",
                2,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Log level (DEBUG, INFO, WARN, ERROR)".to_string()),
            ),
            ColumnDefinition::new(
                3,
                "thread",
                3,
                KalamDataType::Text,
                true,
                false,
                false,
                ColumnDefault::None,
                Some("Thread name that generated the log".to_string()),
            ),
            ColumnDefinition::new(
                4,
                "target",
                4,
                KalamDataType::Text,
                true,
                false,
                false,
                ColumnDefault::None,
                Some("Module or target that generated the log".to_string()),
            ),
            ColumnDefinition::new(
                5,
                "line",
                5,
                KalamDataType::BigInt,
                true,
                false,
                false,
                ColumnDefault::None,
                Some("Source code line number".to_string()),
            ),
            ColumnDefinition::new(
                6,
                "message",
                6,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Log message content".to_string()),
            ),
        ];

        system_view_definition(
            SystemTable::ServerLogs,
            columns,
            "Latest JSON server log lines from the file tail (read-only view)",
        )
    }

    /// Create a new server logs view
    pub fn new(logs_path: impl Into<PathBuf>) -> Self {
        Self {
            logs_path: logs_path.into(),
        }
    }

    fn log_file_path(&self) -> PathBuf {
        let jsonl_path = self.logs_path.join("server.jsonl");
        if jsonl_path.exists() {
            jsonl_path
        } else {
            self.logs_path.join("server.log")
        }
    }

    /// Read the newest parsed JSON lines by walking backward from EOF.
    fn read_log_entries(&self) -> Result<Vec<JsonLogEntry>, RegistryError> {
        read_jsonl_tail(&self.log_file_path(), LOG_TAIL, parse_server_log_line)
    }
}

fn parse_server_log_line(bytes: &[u8]) -> Option<JsonLogEntry> {
    parse_json_log_line(bytes, LOG_TAIL.max_line_bytes, RawJsonLogEntry::into_entry)
}

impl VirtualView for ServerLogsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::ServerLogs
    }

    fn schema(&self) -> SchemaRef {
        server_logs_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let entries = self.read_log_entries()?;
        let row_count = entries.len();

        let mut timestamps = StringBuilder::with_capacity(row_count, row_count * 32);
        let mut levels = StringBuilder::with_capacity(row_count, row_count * 8);
        let mut threads = StringBuilder::with_capacity(row_count, row_count * 16);
        let mut targets = StringBuilder::with_capacity(row_count, row_count * 24);
        let mut lines = Int64Builder::with_capacity(row_count);
        let mut messages = StringBuilder::with_capacity(row_count, row_count * 64);

        for entry in entries {
            timestamps.append_value(&entry.timestamp);
            levels.append_value(&entry.level);

            if let Some(thread) = &entry.thread {
                threads.append_value(thread);
            } else {
                threads.append_null();
            }

            if let Some(target) = &entry.target {
                targets.append_value(target);
            } else {
                targets.append_null();
            }

            if let Some(line) = entry.line {
                lines.append_value(line);
            } else {
                lines.append_null();
            }

            messages.append_value(&entry.message);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(timestamps.finish()) as ArrayRef,
                Arc::new(levels.finish()) as ArrayRef,
                Arc::new(threads.finish()) as ArrayRef,
                Arc::new(targets.finish()) as ArrayRef,
                Arc::new(lines.finish()) as ArrayRef,
                Arc::new(messages.finish()) as ArrayRef,
            ],
        )
        .map_err(|e| RegistryError::Other(format!("Failed to build server_logs batch: {}", e)))
    }
}

pub type ServerLogsTableProvider = SystemViewProvider<ServerLogsView>;

pub fn create_server_logs_provider(logs_path: impl Into<PathBuf>) -> ServerLogsTableProvider {
    view_provider_with(logs_path.into(), ServerLogsView::new)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use datafusion::{
        execution::{memory_pool::GreedyMemoryPool, runtime_env::RuntimeEnvBuilder},
        prelude::{SessionConfig, SessionContext},
    };
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_schema() {
        let schema = server_logs_schema();
        assert_eq!(schema.fields().len(), 6);
        assert_eq!(schema.field(0).name(), "timestamp");
        assert_eq!(schema.field(1).name(), "level");
        assert_eq!(schema.field(2).name(), "thread");
        assert_eq!(schema.field(3).name(), "target");
        assert_eq!(schema.field(4).name(), "line");
        assert_eq!(schema.field(5).name(), "message");
    }

    #[test]
    fn test_parse_json_logs() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("server.log");

        // Write some JSON log entries
        let mut file = std::fs::File::create(&log_file).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:30:00.123+00:00","level":"INFO","thread":"main","target":"kalamdb_server","line":42,"message":"Server started"}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:30:01.456+00:00","level":"DEBUG","thread":"worker-1","target":"kalamdb_core","line":100,"message":"Processing query"}}"#
        )
        .unwrap();

        let view = ServerLogsView::new(dir.path());
        let entries = view.read_log_entries().unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, "INFO");
        assert_eq!(entries[0].message, "Server started");
        assert_eq!(entries[1].level, "DEBUG");
    }

    #[test]
    fn test_parse_tracing_json_logs() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("server.jsonl");

        let mut file = std::fs::File::create(&log_file).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:30:00.123Z","level":"INFO","fields":{{"message":"Server started"}},"target":"kalamdb_server","threadName":"main"}}"#
        )
        .unwrap();

        let view = ServerLogsView::new(dir.path());
        let entries = view.read_log_entries().unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, "INFO");
        assert_eq!(entries[0].message, "Server started");
        assert_eq!(entries[0].thread.as_deref(), Some("main"));
    }

    #[test]
    fn test_empty_log_file() {
        let dir = tempdir().unwrap();
        // Don't create the log file

        let view = ServerLogsView::new(dir.path());
        let entries = view.read_log_entries().unwrap();

        assert!(entries.is_empty());
    }

    #[test]
    fn test_compute_batch() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("server.jsonl");

        let mut file = std::fs::File::create(&log_file).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:30:00Z","level":"INFO","message":"Test"}}"#
        )
        .unwrap();

        let view = ServerLogsView::new(dir.path());
        let batch = view.compute_batch().unwrap();

        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 6);
    }

    #[test]
    fn compute_batch_keeps_only_the_latest_max_rows() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("server.jsonl");
        let mut file = std::fs::File::create(&log_file).unwrap();
        let total = SERVER_LOG_MAX_ROWS + 40;
        for index in 0..total {
            writeln!(
                file,
                r#"{{"timestamp":"2024-01-15T10:30:00.{index:03}Z","level":"INFO","message":"row-{index}"}}"#
            )
            .unwrap();
        }

        let view = ServerLogsView::new(dir.path());
        let entries = view.read_log_entries().unwrap();
        assert_eq!(entries.len(), SERVER_LOG_MAX_ROWS);
        assert_eq!(entries[0].message, format!("row-{}", total - SERVER_LOG_MAX_ROWS));
        assert_eq!(entries[SERVER_LOG_MAX_ROWS - 1].message, format!("row-{}", total - 1));
    }

    #[test]
    fn compute_batch_reads_only_the_file_tail() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("server.jsonl");
        let mut file = std::fs::File::create(&log_file).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:00:00Z","level":"INFO","message":"old-line"}}"#
        )
        .unwrap();
        for index in 0..SERVER_LOG_MAX_ROWS {
            writeln!(
                file,
                r#"{{"timestamp":"2024-01-15T10:30:00.{index:03}Z","level":"INFO","message":"pad-{index}"}}"#
            )
            .unwrap();
        }
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T11:00:00Z","level":"INFO","message":"new-line"}}"#
        )
        .unwrap();

        let view = ServerLogsView::new(dir.path());
        let entries = view.read_log_entries().unwrap();
        assert!(entries.iter().all(|entry| entry.message != "old-line"));
        assert!(entries.iter().any(|entry| entry.message == "new-line"));
        assert_eq!(entries.len(), SERVER_LOG_MAX_ROWS);
    }

    #[test]
    fn skips_oversized_lines_and_keeps_neighbors() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("server.jsonl");
        let mut file = std::fs::File::create(&log_file).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:00:00Z","level":"INFO","message":"before"}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:00:01Z","level":"INFO","message":"{}"}}"#,
            "x".repeat(SERVER_LOG_MAX_LINE_BYTES)
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2024-01-15T10:00:02Z","level":"INFO","message":"after"}}"#
        )
        .unwrap();

        let view = ServerLogsView::new(dir.path());
        let entries = view.read_log_entries().unwrap();
        let messages: Vec<&str> = entries.iter().map(|entry| entry.message.as_str()).collect();
        assert_eq!(messages, ["before", "after"]);
    }

    #[test]
    fn order_by_limit_fits_small_memory_pool() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("server.jsonl");
        let mut file = std::fs::File::create(&log_file).unwrap();
        let payload = "x".repeat(4 * 1024);
        for index in 0..200 {
            writeln!(
                file,
                r#"{{"timestamp":"2024-01-15T10:30:{:02}.000Z","level":"INFO","message":"{payload}-{index}"}}"#,
                index % 60
            )
            .unwrap();
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let memory_pool = Arc::new(GreedyMemoryPool::new(8 * 1024 * 1024));
            let runtime_env = RuntimeEnvBuilder::new()
                .with_memory_pool(memory_pool)
                .build_arc()
                .expect("runtime env");
            let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime_env);
            ctx.register_table("server_logs", Arc::new(create_server_logs_provider(dir.path())))
                .expect("register table");
            let batches = ctx
                .sql(
                    "SELECT timestamp, level, message FROM server_logs ORDER BY timestamp DESC \
                     LIMIT 50",
                )
                .await
                .expect("plan")
                .collect()
                .await
                .expect("topk should fit in the 8MB pool");
            let rows: usize = batches.iter().map(|batch| batch.num_rows()).sum();
            assert!(rows > 0, "expected at least one log row from the tail");
            assert!(rows <= 50, "limit 50 should not return {rows} rows");
        });
    }
}
