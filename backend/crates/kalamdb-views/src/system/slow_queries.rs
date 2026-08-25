//! system.slow_queries virtual view
//!
//! Exposes recent slow-query JSONL entries without keeping query text in memory
//! or reading unbounded log files on each scan.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
};

use datafusion::arrow::{
    array::{ArrayRef, Float64Builder, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{
    datatypes::KalamDataType,
    schemas::{ColumnDefault, ColumnDefinition, TableDefinition},
};
use kalamdb_system::SystemTable;

use super::common::{system_view_definition, view_provider_with, SystemViewProvider};
use crate::{error::RegistryError, view_base::VirtualView};

crate::memoized_view_schema!(slow_queries_schema, SlowQueriesView);

const SLOW_QUERY_READ_BYTES: u64 = 1024 * 1024;
const SLOW_QUERY_MAX_ROWS: usize = 200;

#[derive(Debug)]
struct SlowQueryLogEntry {
    timestamp:    String,
    timestamp_ms: i64,
    duration_ms:  f64,
    user_id:      String,
    table_type:   String,
    table_name:   Option<String>,
    row_count:    i64,
    query:        String,
}

#[derive(Debug, serde::Deserialize)]
struct RawSlowQueryLogEntry {
    timestamp:     Option<String>,
    timestamp_ms:  Option<i64>,
    duration_ms:   Option<f64>,
    duration_secs: Option<f64>,
    row_count:     Option<i64>,
    user_id:       Option<String>,
    table_type:    Option<String>,
    table_name:    Option<String>,
    query:         Option<String>,
}

impl RawSlowQueryLogEntry {
    fn into_entry(self) -> Option<SlowQueryLogEntry> {
        let timestamp = self.timestamp?;
        let timestamp_ms = self.timestamp_ms?;
        let duration_ms =
            self.duration_ms.or_else(|| self.duration_secs.map(|secs| secs * 1000.0))?;
        if !duration_ms.is_finite() {
            return None;
        }

        Some(SlowQueryLogEntry {
            timestamp,
            timestamp_ms,
            duration_ms,
            user_id: self.user_id?,
            table_type: self.table_type?,
            table_name: self.table_name,
            row_count: self.row_count?,
            query: self.query?,
        })
    }
}

#[derive(Debug)]
pub struct SlowQueriesView {
    logs_path: PathBuf,
}

impl SlowQueriesView {
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
                Some("Slow query timestamp in RFC 3339 format".to_string())),
            ColumnDefinition::new(
                2,
                "timestamp_ms",
                2,
                KalamDataType::BigInt,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Slow query timestamp as epoch milliseconds".to_string())),
            ColumnDefinition::new(
                3,
                "duration_ms",
                3,
                KalamDataType::Double,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Query duration in milliseconds".to_string())),
            ColumnDefinition::new(
                4,
                "user_id",
                4,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("User that executed the query".to_string())),
            ColumnDefinition::new(
                5,
                "table_type",
                5,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Resolved table type for the query".to_string())),
            ColumnDefinition::new(
                6,
                "table_name",
                6,
                KalamDataType::Text,
                true,
                false,
                false,
                ColumnDefault::None,
                Some("Resolved table name when available".to_string())),
            ColumnDefinition::new(
                7,
                "row_count",
                7,
                KalamDataType::BigInt,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Rows returned or affected".to_string())),
            ColumnDefinition::new(
                8,
                "query",
                8,
                KalamDataType::Text,
                false,
                false,
                false,
                ColumnDefault::None,
                Some("Redacted SQL text".to_string())),
        ];

        system_view_definition(
            SystemTable::SlowQueries,
            columns,
            "Recent slow queries from the bounded slow-query JSONL log view")
    }

    pub fn new(logs_path: impl Into<PathBuf>) -> Self {
        Self {
            logs_path: logs_path.into(),
        }
    }

    fn slow_log_path(&self) -> PathBuf {
        let jsonl_path = self.logs_path.join("slow.jsonl");
        if jsonl_path.exists() {
            jsonl_path
        } else {
            self.logs_path.join("slow.log")
        }
    }

    fn read_log_tail(&self) -> Result<String, RegistryError> {
        let path = self.slow_log_path();
        if !path.exists() {
            return Ok(String::new());
        }

        let mut file = File::open(&path).map_err(|error| {
            RegistryError::Other(format!(
                "Failed to open slow query log {}: {}",
                path.display(),
                error
            ))
        })?;
        let file_len = file
            .metadata()
            .map_err(|error| {
                RegistryError::Other(format!(
                    "Failed to stat slow query log {}: {}",
                    path.display(),
                    error
                ))
            })?
            .len();
        let offset = file_len.saturating_sub(SLOW_QUERY_READ_BYTES);
        file.seek(SeekFrom::Start(offset)).map_err(|error| {
            RegistryError::Other(format!(
                "Failed to seek slow query log {}: {}",
                path.display(),
                error
            ))
        })?;

        let mut content = String::new();
        file.read_to_string(&mut content).map_err(|error| {
            RegistryError::Other(format!(
                "Failed to read slow query log {}: {}",
                path.display(),
                error
            ))
        })?;

        if offset == 0 {
            return Ok(content);
        }

        Ok(content.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default())
    }

    fn read_entries(&self) -> Result<Vec<SlowQueryLogEntry>, RegistryError> {
        let content = self.read_log_tail()?;
        let mut entries = Vec::new();
        for line in content.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if let Ok(raw) = serde_json::from_str::<RawSlowQueryLogEntry>(line) {
                if let Some(entry) = raw.into_entry() {
                    if entries.len() == SLOW_QUERY_MAX_ROWS {
                        entries.remove(0);
                    }
                    entries.push(entry);
                }
            }
        }

        Ok(entries)
    }
}

impl VirtualView for SlowQueriesView {
    fn system_table(&self) -> SystemTable {
        SystemTable::SlowQueries
    }

    fn schema(&self) -> SchemaRef {
        slow_queries_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, RegistryError> {
        let entries = self.read_entries()?;

        let mut timestamps = StringBuilder::new();
        let mut timestamp_ms = Int64Builder::new();
        let mut duration_ms = Float64Builder::new();
        let mut user_ids = StringBuilder::new();
        let mut table_types = StringBuilder::new();
        let mut table_names = StringBuilder::new();
        let mut row_counts = Int64Builder::new();
        let mut queries = StringBuilder::new();

        for entry in entries {
            timestamps.append_value(&entry.timestamp);
            timestamp_ms.append_value(entry.timestamp_ms);
            duration_ms.append_value(entry.duration_ms);
            user_ids.append_value(&entry.user_id);
            table_types.append_value(&entry.table_type);
            if let Some(table_name) = &entry.table_name {
                table_names.append_value(table_name);
            } else {
                table_names.append_null();
            }
            row_counts.append_value(entry.row_count);
            queries.append_value(&entry.query);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(timestamps.finish()) as ArrayRef,
                Arc::new(timestamp_ms.finish()) as ArrayRef,
                Arc::new(duration_ms.finish()) as ArrayRef,
                Arc::new(user_ids.finish()) as ArrayRef,
                Arc::new(table_types.finish()) as ArrayRef,
                Arc::new(table_names.finish()) as ArrayRef,
                Arc::new(row_counts.finish()) as ArrayRef,
                Arc::new(queries.finish()) as ArrayRef,
            ])
        .map_err(|error| {
            RegistryError::Other(format!("Failed to build slow_queries batch: {}", error))
        })
    }
}

pub type SlowQueriesTableProvider = SystemViewProvider<SlowQueriesView>;

pub fn create_slow_queries_provider(logs_path: impl Into<PathBuf>) -> SlowQueriesTableProvider {
    view_provider_with(logs_path.into(), SlowQueriesView::new)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_schema() {
        let schema = slow_queries_schema();
        assert_eq!(schema.fields().len(), 8);
        assert_eq!(schema.field(0).name(), "timestamp");
        assert_eq!(schema.field(7).name(), "query");
    }

    #[test]
    fn test_parse_slow_query_jsonl() {
        let dir = tempdir().unwrap();
        let log_file = dir.path().join("slow.jsonl");
        let mut file = std::fs::File::create(&log_file).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2026-05-26T10:30:00.123Z","timestamp_ms":1780000000123,"duration_ms":1250.0,"duration_secs":1.25,"row_count":10,"user_id":"root","table_type":"user","table_name":"events","query":"SELECT * FROM events"}}"#
        )
        .unwrap();

        let view = SlowQueriesView::new(dir.path());
        let entries = view.read_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].duration_ms, 1250.0);
        assert_eq!(entries[0].table_name.as_deref(), Some("events"));
    }
}
