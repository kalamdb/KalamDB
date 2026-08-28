//! Base flush trait and common utilities for table flush operations
//!
//! This module provides a common interface and implementation for flushing table data
//! from RocksDB to Parquet files. It eliminates code duplication across different table
//! types (shared, user, stream) by providing:
//!
//! - `TableFlush` trait: Common interface for all flush operations
//! - `FlushJobResult`: Standardized result type with metrics
//! - `FlushExecutor`: Template method pattern for job tracking and error handling
//!
//! ## Architecture
//!
//! ```text
//! TableFlush trait (defines interface)
//!      ↓
//! FlushExecutor (template method - common workflow)
//!      ↓
//! Implementations (users.rs, shared.rs, streams.rs)
//! ```

use std::sync::Arc;

use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_commons::{
    schemas::{TableCompression, TableType},
    TableId, UserId,
};
use kalamdb_filestore::StorageCached;
use serde::{Deserialize, Serialize};

use crate::Result;

/// Scope touched by a flush operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushScopeHint {
    Shared,
    User(UserId),
}

/// Metadata for user table flush operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UserTableFlushMetadata {
    /// Number of unique users whose data was flushed
    pub users_count: usize,

    /// Error messages for users that failed to flush (partial failures)
    pub errors: Vec<String>,
}

/// Metadata for shared table flush operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SharedTableFlushMetadata {
    /// Additional context (reserved for future use)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Flush metadata for different table types
///
/// Type-safe alternative to JsonValue for flush operation metadata.
/// Each table type has its own strongly-typed metadata structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "table_type", rename_all = "snake_case")]
pub enum FlushMetadata {
    /// User table flush metadata (partitioned by user_id)
    UserTable(UserTableFlushMetadata),

    /// Shared table flush metadata (single Parquet file)
    SharedTable(SharedTableFlushMetadata),
}

impl FlushMetadata {
    /// Create metadata for user table flush
    pub fn user_table(users_count: usize, errors: Vec<String>) -> Self {
        FlushMetadata::UserTable(UserTableFlushMetadata {
            users_count,
            errors,
        })
    }

    /// Create metadata for shared table flush
    pub fn shared_table() -> Self {
        FlushMetadata::SharedTable(SharedTableFlushMetadata::default())
    }

    /// Get users_count if this is UserTable metadata
    pub fn users_count(&self) -> Option<usize> {
        match self {
            FlushMetadata::UserTable(meta) => Some(meta.users_count),
            _ => None,
        }
    }

    /// Get errors if this is UserTable metadata
    pub fn errors(&self) -> Option<&[String]> {
        match self {
            FlushMetadata::UserTable(meta) => Some(&meta.errors),
            _ => None,
        }
    }
}

/// Result of a flush job execution
///
/// Contains metrics and output files (no Job record - that's JobsManager's responsibility)
#[derive(Debug, Clone)]
pub struct FlushJobResult {
    /// Total rows flushed from RocksDB to Parquet
    pub rows_flushed: usize,

    /// Parquet files written (relative paths)
    pub parquet_files: Vec<String>,

    /// Table scopes that wrote new Parquet files.
    pub scope_hints: Vec<FlushScopeHint>,

    /// Type-safe metadata specific to table type
    pub metadata: FlushMetadata,
}

/// Cached table metadata needed by the flush writer.
#[derive(Debug, Clone)]
pub struct FlushTableMetadata {
    pub schema_version:       u32,
    pub bloom_filter_columns: Vec<String>,
    pub indexed_columns:      Vec<(u64, String)>,
    pub compression:          TableCompression,
}

impl FlushTableMetadata {
    pub fn new(
        schema_version: u32,
        bloom_filter_columns: Vec<String>,
        indexed_columns: Vec<(u64, String)>,
        compression: TableCompression,
    ) -> Self {
        Self {
            schema_version,
            bloom_filter_columns,
            indexed_columns,
            compression,
        }
    }
}

/// Higher-level side effects that run after a scope is durably flushed.
pub trait FlushScopeHook: Send + Sync {
    fn after_scope_write(
        &self,
        table_type: TableType,
        table_id: &TableId,
        user_id: Option<&UserId>,
        schema: &SchemaRef,
        storage_cached: &Arc<StorageCached>,
    ) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopFlushScopeHook;

impl FlushScopeHook for NoopFlushScopeHook {
    fn after_scope_write(
        &self,
        _table_type: TableType,
        _table_id: &TableId,
        _user_id: Option<&UserId>,
        _schema: &SchemaRef,
        _storage_cached: &Arc<StorageCached>,
    ) -> Result<()> {
        Ok(())
    }
}

/// Statistics from version resolution during flush
#[derive(Debug, Clone, Default)]
pub struct FlushDedupStats {
    /// Total rows scanned from hot storage (before dedup)
    pub rows_before_dedup:   usize,
    /// Unique rows after version resolution
    pub rows_after_dedup:    usize,
    /// Number of soft-deleted (tombstone) rows encountered
    pub deleted_count:       usize,
    /// Number of tombstones filtered out in final output
    pub tombstones_filtered: usize,
}

impl FlushDedupStats {
    /// Calculate deduplication ratio as percentage
    pub fn dedup_ratio(&self) -> f64 {
        if self.rows_before_dedup > 0 {
            (self.rows_before_dedup - self.rows_after_dedup) as f64 / self.rows_before_dedup as f64
                * 100.0
        } else {
            0.0
        }
    }

    /// Log summary statistics
    /// `table_ref` should be in format "namespace:table" (from TableId Display)
    pub fn log_summary(&self, table_ref: &str) {
        log::debug!(
            "📊 [FLUSH DEDUP] Scanned {} total rows from hot storage (table={})",
            self.rows_before_dedup,
            table_ref
        );
        log::debug!(
            "📊 [FLUSH DEDUP] Version resolution complete: {} rows → {} unique (dedup: {:.1}%, \
             deleted: {})",
            self.rows_before_dedup,
            self.rows_after_dedup,
            self.dedup_ratio(),
            self.deleted_count
        );
        log::debug!(
            "📊 [FLUSH DEDUP] Final: {} rows to flush ({} tombstones filtered)",
            self.rows_after_dedup - self.tombstones_filtered,
            self.tombstones_filtered
        );
    }
}

/// Base trait for table flush operations
///
/// Implement this trait for each table type (shared, user, stream).
/// Called by FlushExecutor after JobsManager creates the Job record.
///
/// ## Snapshot Consistency
///
/// All flush operations use RocksDB snapshots via `scan_iter()`:
/// - ✅ ACID guarantees: Flush sees consistent point-in-time view
/// - ✅ No locking: Writes continue during flush
/// - ✅ Isolation: Concurrent flushes don't interfere
pub trait TableFlush: Send + Sync {
    /// Execute the flush job
    ///
    /// Implement table-specific logic:
    /// - Scan rows from RocksDB (use `scan_iter()` for snapshot consistency)
    /// - Convert rows to Arrow RecordBatch
    /// - Write Parquet file(s) (use `ParquetWriter`)
    ///
    /// # Returns
    ///
    /// `FlushJobResult` with metrics (rows flushed, files created, metadata)
    ///
    /// # Errors
    ///
    /// Returns `FlushError` if flush fails (I/O error, invalid data, etc.)
    fn execute(&self) -> Result<FlushJobResult>;

    /// Get table identifier for logging
    ///
    /// Should return `namespace.table_name` format
    fn table_identifier(&self) -> String;
}

/// Common configuration for flush jobs
///
/// Both SharedTableFlushJob and UserTableFlushJob share these constants.
pub mod config {
    /// Number of rows to process per batch during scan
    pub const BATCH_SIZE: usize = 10000;

    /// Number of hot-storage keys to remove in one indexed-store delete batch.
    pub const DELETE_BATCH_SIZE: usize = 4096;
}

/// Common helper functions for flush operations
pub mod helpers {
    use std::collections::{HashMap, HashSet};

    use datafusion::{
        arrow::{datatypes::SchemaRef, record_batch::RecordBatch},
        scalar::ScalarValue,
    };
    use kalamdb_commons::{
        constants::SystemColumnNames, conversions::arrow_json_conversion::json_rows_to_arrow_batch,
        ids::SeqId, models::rows::Row, next_storage_key_bytes, pk_bucket_key_from_row, PkBucketKey,
    };
    use kalamdb_tables::{SharedTableRow, UserTableRow};

    use super::{FlushDedupStats, FlushTableMetadata};
    use crate::{error::FlushResultExt, Result};

    pub type LatestVersions<R> = HashMap<PkBucketKey, (Vec<u8>, R, i64)>;

    /// Row behavior shared by user/shared flush version resolution.
    pub trait FlushVersionRow {
        fn seq_i64(&self) -> i64;
        fn is_deleted(&self) -> bool;
        fn fields(&self) -> &Row;
        fn into_fields(self) -> Row;
    }

    impl FlushVersionRow for UserTableRow {
        #[inline]
        fn seq_i64(&self) -> i64 {
            self._seq.as_i64()
        }

        #[inline]
        fn is_deleted(&self) -> bool {
            self._deleted
        }

        #[inline]
        fn fields(&self) -> &Row {
            &self.fields
        }

        #[inline]
        fn into_fields(self) -> Row {
            self.fields
        }
    }

    impl FlushVersionRow for SharedTableRow {
        #[inline]
        fn seq_i64(&self) -> i64 {
            self._seq.as_i64()
        }

        #[inline]
        fn is_deleted(&self) -> bool {
            self._deleted
        }

        #[inline]
        fn fields(&self) -> &Row {
            &self.fields
        }

        #[inline]
        fn into_fields(self) -> Row {
            self.fields
        }
    }

    /// Extract primary key field name from Arrow schema
    ///
    /// Returns the first non-system column (doesn't start with '_') or "id" as fallback.
    pub fn extract_pk_field_name(schema: &SchemaRef) -> String {
        schema
            .fields()
            .iter()
            .find(|f| !f.name().starts_with('_'))
            .map(|f| f.name().clone())
            .unwrap_or_else(|| "id".to_string())
    }

    /// Configured row scan batch size, clamped to a valid minimum.
    #[inline]
    pub fn normalize_scan_batch_size(scan_batch_size: usize) -> usize {
        scan_batch_size.max(1)
    }

    /// Update cursor for next batch (append null byte to skip current key)
    pub fn advance_cursor(last_key: &[u8]) -> Vec<u8> {
        next_storage_key_bytes(last_key)
    }

    /// Calculate deduplication ratio
    pub fn calculate_dedup_ratio(before: usize, after: usize) -> f64 {
        if before > 0 {
            (before - after) as f64 / before as f64 * 100.0
        } else {
            0.0
        }
    }

    /// Convert rows with _seq and _deleted system columns to Arrow RecordBatch
    ///
    /// This is the common pattern used by both user and shared table flush.
    /// Adds _seq and _deleted columns to each row before conversion.
    pub fn rows_to_arrow_batch(schema: &SchemaRef, rows: &[(Vec<u8>, Row)]) -> Result<RecordBatch> {
        // Avoid cloning: collect references, then build columnar arrays directly.
        // The JSON conversion function requires owned Rows (it consumes the BTreeMap),
        // so we must clone — but we skip cloning the key bytes.
        let arrow_rows: Vec<Row> = rows.iter().map(|(_, row)| row.clone()).collect();
        json_rows_to_arrow_batch(schema, arrow_rows).into_flush_error("Failed to build RecordBatch")
    }

    /// Convert owned rows (consuming the key bytes) to Arrow RecordBatch.
    ///
    /// More efficient than `rows_to_arrow_batch` when you can pass ownership,
    /// avoiding one Row clone per row.
    pub fn rows_into_arrow_batch(
        schema: &SchemaRef,
        rows: Vec<(Vec<u8>, Row)>,
    ) -> Result<RecordBatch> {
        let arrow_rows: Vec<Row> = rows.into_iter().map(|(_, row)| row).collect();
        json_rows_to_arrow_batch(schema, arrow_rows).into_flush_error("Failed to build RecordBatch")
    }

    /// Get schema version from cached table data
    pub fn get_schema_version(
        metadata: &FlushTableMetadata,
        _table_id: &kalamdb_commons::models::TableId,
    ) -> u32 {
        metadata.schema_version
    }

    /// Track the latest hot-storage version for a primary key.
    #[inline]
    pub fn track_latest_version<R: FlushVersionRow>(
        latest_versions: &mut LatestVersions<R>,
        key_bytes: Vec<u8>,
        row: R,
        pk_field: &str,
        stats: &mut FlushDedupStats,
    ) {
        let seq = row.seq_i64();
        let pk_key = pk_bucket_key_from_row(row.fields(), pk_field, SeqId::from_i64(seq));

        if row.is_deleted() {
            stats.deleted_count += 1;
        }

        match latest_versions.entry(pk_key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if seq > entry.get().2 {
                    entry.insert((key_bytes, row, seq));
                }
            },
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((key_bytes, row, seq));
            },
        }
    }

    /// Add system columns (_seq, _deleted) to a Row
    pub fn add_system_columns(mut row: Row, seq: i64, deleted: bool) -> Row {
        row.values
            .insert(SystemColumnNames::SEQ.to_string(), ScalarValue::Int64(Some(seq)));
        row.values
            .insert(SystemColumnNames::DELETED.to_string(), ScalarValue::Boolean(Some(deleted)));
        row
    }

    /// Split latest hot versions into flushable rows and tombstones that must stay hot.
    ///
    /// Tombstones are intentionally not written to Parquet, but the latest tombstone key must
    /// remain in hot storage so it can continue masking older flushed segments until compaction
    /// reconciles cold storage.
    pub fn rows_and_tombstones_from_versions<R: FlushVersionRow>(
        latest_versions: LatestVersions<R>,
        stats: &mut FlushDedupStats,
    ) -> (Vec<(Vec<u8>, Row)>, HashSet<Vec<u8>>) {
        let mut rows = Vec::with_capacity(latest_versions.len());
        let mut preserved_tombstone_keys = HashSet::new();

        for (_pk_value, (key_bytes, row, _seq)) in latest_versions {
            if row.is_deleted() {
                stats.tombstones_filtered += 1;
                preserved_tombstone_keys.insert(key_bytes);
                continue;
            }

            let seq = row.seq_i64();
            rows.push((key_bytes, add_system_columns(row.into_fields(), seq, false)));
        }

        (rows, preserved_tombstone_keys)
    }

    /// Remove keys that must remain hot, preserving ownership of all other cleanup keys.
    pub fn cleanup_keys_excluding_preserved(
        keys: Vec<Vec<u8>>,
        preserved_keys: &HashSet<Vec<u8>>,
    ) -> Vec<Vec<u8>> {
        if preserved_keys.is_empty() {
            return keys;
        }

        let mut cleanup_keys = Vec::with_capacity(keys.len().saturating_sub(preserved_keys.len()));
        for key in keys {
            if !preserved_keys.contains(&key) {
                cleanup_keys.push(key);
            }
        }
        cleanup_keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FlushError;

    struct MockFlushJob {
        should_fail: bool,
        rows_count:  usize,
    }

    impl TableFlush for MockFlushJob {
        fn execute(&self) -> Result<FlushJobResult> {
            if self.should_fail {
                return Err(FlushError::Other("Mock failure".to_string()));
            }

            Ok(FlushJobResult {
                rows_flushed:  self.rows_count,
                parquet_files: vec!["batch-123.parquet".to_string()],
                scope_hints:   vec![FlushScopeHint::Shared],
                metadata:      FlushMetadata::shared_table(),
            })
        }

        fn table_identifier(&self) -> String {
            "test_namespace.test_table".to_string()
        }
    }

    #[test]
    fn test_flush_execution_success() {
        let job = MockFlushJob {
            should_fail: false,
            rows_count:  100,
        };

        let result = job.execute();
        assert!(result.is_ok());

        let result = result.unwrap();
        assert_eq!(result.rows_flushed, 100);
        assert_eq!(result.parquet_files.len(), 1);
    }

    #[test]
    fn test_flush_execution_failure() {
        let job = MockFlushJob {
            should_fail: true,
            rows_count:  0,
        };

        let result = job.execute();
        assert!(result.is_err());
    }
}
