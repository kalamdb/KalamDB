//! Base trait for table providers with unified DML operations
//!
//! This module provides:
//! - BaseTableProvider<K, V> trait for generic table operations
//! - TableProviderCore shared structure for common services
//! - Shared MVCC-oriented scan helpers for user and shared tables
//!
//! **Design Rationale**:
//! - Eliminates most of the historic duplicate code across User/Shared/Stream providers
//! - Generic over storage key (K) and value (V) types
//! - No separate handlers - DML logic implemented directly in providers
//! - Shared core reduces memory overhead (Arc<TableProviderCore> vs per-provider fields)
//! - New planning-only helpers are moving into `kalamdb-datafusion-sources`
//!
//! ## Streaming vs MVCC Constraints
//!
//! **Why full iterator-based streaming is NOT possible for User/Shared tables:**
//!
//! User and Shared tables use MVCC (Multi-Version Concurrency Control) with version
//! resolution. This means:
//!
//! 1. Multiple versions of the same row may exist (each INSERT/UPDATE creates a new _seq)
//! 2. To return the "current" row, we must find MAX(_seq) per primary key
//! 3. Tombstones (_deleted = true) must hide older versions
//!
//! This inherently requires seeing ALL rows before returning ANY results, making
//! true streaming impossible. The flow is:
//!
//! ```text
//! Hot Storage (RocksDB) ─┐
//!                        ├──> Merge ──> Version Resolution ──> Filter Deleted ──> Result
//! Cold Storage (Parquet) ┘       (requires ALL rows to find MAX(_seq) per PK)
//! ```
//!
//! Stream tables ARE streamable** because they:
//! - Are append-only (no updates, no version resolution needed)
//! - Use TTL-based eviction instead of tombstones
//! - Can return rows as they're scanned with early termination on LIMIT
//! - They now use the provider-family-specific exec-backed path in `stream_table_provider.rs`
//!   instead of sharing the MVCC-oriented flow below
//!
//! ## Architecture
//!
//! ```text
//! User/Shared TableProvider::scan()
//!             │
//!             ▼
//! base_scan() ── combines filters, remaps projections, and builds the deferred
//!                MVCC execution plan used by user/shared providers
//!             │
//!             ▼
//! scan_rows() ── extracts scan context, applies fast paths, and calls
//!                scan_kvs_with_context()
//!             │
//!             ▼
//! scan_kvs_with_context() ── provider-specific hot/cold scan implementation:
//!   • User: user-scoped RocksDB prefix + Parquet, MVCC merge
//!   • Shared: full RocksDB + Parquet, MVCC merge
//!             │
//!             ▼
//! resolve_latest_scan_from_futures() ── shared concurrent hot/cold fetch +
//!                                       MVCC winner selection
//!
//! Stream tables bypass this path and build deferred execution descriptors in
//! `stream_table_provider.rs` so the hot-store scan runs at execute time.
//! ```

mod cold_pk;
mod deferred;
mod hot_scan;
mod provider;
mod rows;
mod versions;

pub use cold_pk::{
    ensure_pk_absent_in_cold, ensure_unique_pk_value, find_row_by_pk, pk_exists_batch_in_cold,
    pk_exists_in_cold,
    validate_pk_update,
};
pub(crate) use deferred::scan_diagnostics_enabled;
pub use deferred::{
    extract_pk_equality_literal, filter_uses_deleted_column, is_count_only_projection,
    typed_pk_literal_from_filter, DeferredMvccScanProvider, MvccScanResult,
};
pub use hot_scan::{
    apply_limit, build_count_only_batch, calculate_scan_limit, compute_cold_columns,
    compute_metadata_only_cold_columns,
};
pub(crate) use hot_scan::{
    count_resolved_rows_from_futures, hot_index_seek, resolve_latest_scan_from_futures,
    scan_hot_index_resolved,
};
pub use provider::BaseTableProvider;
pub use rows::{
    build_notification_row, build_stream_notification_row, embedding_columns,
    extract_embedding_vector,
};
pub use versions::{
    direct_insert_versions, snapshot_bound_from_log_index, statement_row_version,
    transaction_snapshot_bound,
};

pub(crate) use crate::utils::parquet::{
    scan_parquet_files_as_batch_async, scan_parquet_files_as_result_async,
    scan_parquet_files_with_stats_async, ParquetScanResult,
};
pub use crate::utils::{
    core::TableProviderCore,
    row_utils::{
        extract_full_user_context, extract_seq_bounds_from_filter, inject_system_columns,
        materialize_scan_rows, resolve_user_scope, rows_to_arrow_batch, system_user_id, ScanRow,
    },
};
