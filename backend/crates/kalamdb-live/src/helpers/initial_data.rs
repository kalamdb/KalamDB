// backend/crates/kalamdb-live/src/helpers/initial_data.rs
//
// Initial data fetch for live query subscriptions.
// Provides "changes since timestamp" functionality to populate client state
// before real-time notifications begin.

use std::{collections::BTreeMap, fmt::Write, sync::Arc};

use arrow::{
    array::{Array, Int64Array},
    record_batch::RecordBatch,
};
use datafusion_common::ScalarValue;
use kalamdb_commons::{
    constants::SystemColumnNames,
    ids::SeqId,
    models::{rows::Row, ReadContext, TableId},
    Role, TableType,
};
use once_cell::sync::OnceCell;

use crate::{
    error::{LiveError, LiveResultExt},
    traits::{LiveSchemaLookup, LiveSqlExecutor},
};

/// Options for fetching initial data when subscribing to a live query
#[derive(Debug, Clone)]
pub struct InitialDataOptions {
    /// Fetch changes since this sequence ID (exclusive)
    /// If None, starts from the beginning (or end, depending on strategy)
    pub since_seq: Option<SeqId>,

    /// Fetch changes up to this sequence ID (inclusive)
    /// Used to define the snapshot boundary
    pub until_seq: Option<SeqId>,

    /// Fetch changes after this deterministic commit sequence (exclusive).
    pub since_commit_seq: Option<u64>,

    /// Fetch changes up to this deterministic commit sequence (inclusive).
    pub until_commit_seq: Option<u64>,

    /// Maximum number of rows to return (batch size)
    /// Default: 100
    pub limit: usize,

    /// Include soft-deleted rows (_deleted=true)
    /// Default: false
    pub include_deleted: bool,

    /// Fetch the last N rows (newest first) instead of from the beginning
    /// Default: false
    pub fetch_last: bool,
}

impl Default for InitialDataOptions {
    fn default() -> Self {
        Self {
            since_seq: None,
            until_seq: None,
            since_commit_seq: None,
            until_commit_seq: None,
            limit: 100,
            include_deleted: false,
            fetch_last: false,
        }
    }
}

impl InitialDataOptions {
    /// Create options to fetch changes since a specific sequence ID
    pub fn since(seq: SeqId) -> Self {
        Self {
            since_seq: Some(seq),
            until_seq: None,
            since_commit_seq: None,
            until_commit_seq: None,
            limit: 100,
            include_deleted: false,
            fetch_last: false,
        }
    }

    /// Create options to fetch the last N rows (legacy/simple mode)
    /// Note: This might need adjustment for SeqId-based logic
    pub fn last(limit: usize) -> Self {
        Self {
            since_seq: None,
            until_seq: None,
            since_commit_seq: None,
            until_commit_seq: None,
            limit,
            include_deleted: false,
            fetch_last: true,
        }
    }

    /// Create options for batch-based fetching
    pub fn batch(since_seq: Option<SeqId>, until_seq: Option<SeqId>, batch_size: usize) -> Self {
        Self {
            since_seq,
            until_seq,
            since_commit_seq: None,
            until_commit_seq: None,
            limit: batch_size,
            include_deleted: false,
            fetch_last: false,
        }
    }

    /// Set the maximum number of rows to return
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Include soft-deleted rows in the result
    pub fn with_deleted(mut self) -> Self {
        self.include_deleted = true;
        self
    }

    /// Set deterministic commit-sequence resume bounds.
    pub fn with_commit_range(
        mut self,
        since_commit_seq: Option<u64>,
        until_commit_seq: Option<u64>,
    ) -> Self {
        self.since_commit_seq = since_commit_seq;
        self.until_commit_seq = until_commit_seq;
        self
    }
}

/// Result of an initial data fetch
#[derive(Debug)]
pub struct InitialDataResult {
    /// The fetched rows (as Row objects)
    pub rows: Vec<Row>,

    /// Sequence ID of the last row in the result
    /// Used for pagination (passed as since_seq in next request)
    pub last_seq: Option<SeqId>,

    /// Deterministic commit sequence of the last row in the result.
    pub last_commit_seq: Option<u64>,

    /// Whether there are more rows available in the snapshot range
    pub has_more: bool,

    /// The snapshot boundary used for this fetch
    pub snapshot_end_seq: Option<SeqId>,

    /// Deterministic snapshot boundary used for this fetch.
    pub snapshot_end_commit_seq: Option<u64>,
}

/// Service for fetching initial data when subscribing to live queries
pub struct InitialDataFetcher {
    schema_lookup: Arc<dyn LiveSchemaLookup>,
    sql_executor: Arc<OnceCell<Arc<dyn LiveSqlExecutor>>>,
}

#[derive(Debug, Clone, Copy)]
struct TableCapabilities {
    has_commit_seq: bool,
    has_deleted: bool,
}

const BLOCKING_MATERIALIZATION_ROW_THRESHOLD: usize = 4_096;

impl InitialDataFetcher {
    /// Create a new initial data fetcher.
    ///
    /// The SQL executor is set later via `set_sql_executor` because of
    /// bootstrap ordering (LiveQueryManager is created before SqlExecutor).
    pub fn new(schema_lookup: Arc<dyn LiveSchemaLookup>) -> Self {
        Self {
            schema_lookup,
            sql_executor: Arc::new(OnceCell::new()),
        }
    }

    /// Wire the SQL executor (called once during bootstrap).
    pub fn set_sql_executor(&self, executor: Arc<dyn LiveSqlExecutor>) {
        if self.sql_executor.set(executor).is_err() {
            log::warn!("LiveSqlExecutor already initialized in InitialDataFetcher");
        }
    }

    fn sql_executor(&self) -> Result<&Arc<dyn LiveSqlExecutor>, LiveError> {
        self.sql_executor
            .get()
            .ok_or_else(|| LiveError::InvalidOperation("SQL executor not initialized".into()))
    }

    /// Fetch initial data for a table
    ///
    /// # Arguments
    /// * `table_id` - Table identifier with namespace and table name
    /// * `table_type` - User or Shared table
    /// * `options` - Options for the fetch (timestamp, limit, etc.)
    /// * `where_clause` - Optional WHERE clause string to include in the query
    /// * `projections` - Optional column projections (None = SELECT *, all columns)
    ///
    /// # Returns
    /// InitialDataResult with rows and metadata
    pub async fn fetch_initial_data(
        &self,
        live_id: &kalamdb_commons::models::LiveQueryId,
        role: Role,
        table_id: &TableId,
        table_type: TableType,
        options: InitialDataOptions,
        where_clause: Option<&str>,
        projections: Option<&[String]>,
    ) -> Result<InitialDataResult, LiveError> {
        let limit = options.limit;
        if limit == 0 {
            return Ok(InitialDataResult {
                rows: Vec::new(),
                last_seq: None,
                last_commit_seq: None,
                has_more: false,
                snapshot_end_seq: None,
                snapshot_end_commit_seq: None,
            });
        }

        // Extract user_id from LiveId for RLS
        let user_id = live_id.user_id().clone();

        // Execute via trait — handles user scoping and RLS internally
        let table_name = table_id.full_name(); // "namespace.table"

        // Build SELECT clause: either specific columns or *
        // Always include _seq column for pagination, even if not in projections
        let table_capabilities = self.table_capabilities(table_id)?;
        let has_commit_seq = table_capabilities.has_commit_seq;
        let select_clause = if let Some(cols) = projections {
            // Ensure system resume columns are always included for pagination tracking.
            let mut columns = cols.to_vec();
            if !columns.iter().any(|c| c == SystemColumnNames::SEQ) {
                columns.push(SystemColumnNames::SEQ.to_string());
            }
            if has_commit_seq && !columns.iter().any(|c| c == SystemColumnNames::COMMIT_SEQ) {
                columns.push(SystemColumnNames::COMMIT_SEQ.to_string());
            }
            columns.join(", ")
        } else {
            "*".to_string()
        };

        let mut sql = format!("SELECT {} FROM {}", select_clause, table_name);

        let where_clauses =
            self.build_where_clauses(table_type, &options, where_clause, table_capabilities);

        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }

        // Add ORDER BY — use write! to avoid intermediate format! allocations
        if has_commit_seq && options.since_commit_seq.is_some() {
            let direction = if options.fetch_last { "DESC" } else { "ASC" };
            let _ = write!(
                sql,
                " ORDER BY {} {}, {} {}",
                SystemColumnNames::COMMIT_SEQ,
                direction,
                SystemColumnNames::SEQ,
                direction
            );
        } else if options.fetch_last {
            let _ = write!(sql, " ORDER BY {} DESC", SystemColumnNames::SEQ);
        } else {
            let _ = write!(sql, " ORDER BY {} ASC", SystemColumnNames::SEQ);
        }

        // Add LIMIT (fetch limit + 1 to check has_more)
        let _ = write!(sql, " LIMIT {}", limit + 1);

        let batches = self
            .sql_executor()?
            .execute_for_batches(&sql, user_id, role, ReadContext::Internal)
            .await?;

        let mut rows_with_seq =
            materialize_initial_rows(batches, has_commit_seq, limit + 1).await?;
        sort_initial_rows(&mut rows_with_seq, has_commit_seq, &options);

        // Determine has_more and slice to limit
        let total_fetched = rows_with_seq.len();
        let over_limit = total_fetched > limit;
        let has_more = !options.fetch_last && over_limit;

        // Truncate in-place instead of collecting into a new Vec
        if over_limit {
            rows_with_seq.truncate(limit);
        }

        // If we fetched last rows (DESC), we need to reverse them to return in chronological order
        if options.fetch_last {
            rows_with_seq.reverse();
        }

        // Determine snapshot boundary
        let last_seq = rows_with_seq.last().map(|(seq, _, _)| *seq);
        let last_commit_seq = rows_with_seq.last().and_then(|(_, commit_seq, _)| *commit_seq);
        let snapshot_end_seq = options.until_seq.or(last_seq);
        let snapshot_end_commit_seq = options.until_commit_seq.or(last_commit_seq);

        let rows: Vec<Row> = rows_with_seq.into_iter().map(|(_, _, row)| row).collect();

        Ok(InitialDataResult {
            rows,
            last_seq,
            last_commit_seq,
            has_more,
            snapshot_end_seq,
            snapshot_end_commit_seq,
        })
    }

    /// Compute snapshot end sequence for a subscription.
    ///
    /// Compute the snapshot boundary from rows already materialized on this node.
    ///
    /// The boundary deliberately uses local `MAX(_seq)` instead of a wall-clock
    /// Snowflake upper bound. On a follower, the wall-clock bound can include
    /// leader commits that have not applied locally yet; using the local max keeps
    /// the initial snapshot and buffered notification gate aligned with this
    /// replica's actual storage state.
    pub async fn compute_snapshot_end_seq(
        &self,
        live_id: &kalamdb_commons::models::LiveQueryId,
        role: Role,
        table_id: &TableId,
        table_type: TableType,
        options: &InitialDataOptions,
        where_clause: Option<&str>,
    ) -> Result<Option<SeqId>, LiveError> {
        let table_capabilities = self.table_capabilities(table_id)?;
        if table_capabilities.has_commit_seq {
            return Ok(None);
        }

        self.compute_snapshot_end_seq_sql_fallback(
            live_id,
            role,
            table_id,
            table_type,
            options,
            where_clause,
            table_capabilities,
        )
        .await
    }

    /// Compute the deterministic commit-sequence snapshot boundary for tables
    /// that expose `_commit_seq`.
    pub async fn compute_snapshot_end_commit_seq(
        &self,
        live_id: &kalamdb_commons::models::LiveQueryId,
        role: Role,
        table_id: &TableId,
        table_type: TableType,
        options: &InitialDataOptions,
        where_clause: Option<&str>,
    ) -> Result<Option<u64>, LiveError> {
        let table_capabilities = self.table_capabilities(table_id)?;
        if !table_capabilities.has_commit_seq {
            return Ok(None);
        }

        let user_id = live_id.user_id().clone();
        let table_name = table_id.full_name();
        let mut sql = format!(
            "SELECT MAX({}) AS max_commit_seq FROM {}",
            SystemColumnNames::COMMIT_SEQ,
            table_name
        );

        let where_clauses =
            self.build_where_clauses(table_type, options, where_clause, table_capabilities);
        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }

        let batches = self
            .sql_executor()?
            .execute_for_batches(&sql, user_id, role, ReadContext::Internal)
            .await?;

        if batches.is_empty() || batches[0].num_rows() == 0 {
            return Ok(None);
        }

        let batch = &batches[0];
        let value = ScalarValue::try_from_array(batch.column(0), 0)
            .into_serialization_error("Failed to convert max_commit_seq")?;

        match value {
            ScalarValue::UInt64(Some(commit_seq)) => Ok(Some(commit_seq)),
            ScalarValue::Int64(Some(commit_seq)) if commit_seq >= 0 => Ok(Some(commit_seq as u64)),
            ScalarValue::Null | ScalarValue::UInt64(None) | ScalarValue::Int64(None) => Ok(None),
            _ => Err(LiveError::Other("max_commit_seq column is not an integer".to_string())),
        }
    }

    async fn compute_snapshot_end_seq_sql_fallback(
        &self,
        live_id: &kalamdb_commons::models::LiveQueryId,
        role: Role,
        table_id: &TableId,
        table_type: TableType,
        options: &InitialDataOptions,
        where_clause: Option<&str>,
        table_capabilities: TableCapabilities,
    ) -> Result<Option<SeqId>, LiveError> {
        let user_id = live_id.user_id().clone();

        let table_name = table_id.full_name();
        let mut sql =
            format!("SELECT MAX({}) AS max_seq FROM {}", SystemColumnNames::SEQ, table_name);

        let where_clauses =
            self.build_where_clauses(table_type, options, where_clause, table_capabilities);
        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }

        let batches = self
            .sql_executor()?
            .execute_for_batches(&sql, user_id, role, ReadContext::Internal)
            .await?;

        if batches.is_empty() || batches[0].num_rows() == 0 {
            return Ok(None);
        }

        let batch = &batches[0];
        let array = batch
            .column(0)
            .as_any().downcast_ref::<Int64Array>()
            .ok_or_else(|| LiveError::Other("max_seq column is not Int64".to_string()))?;

        if array.is_null(0) {
            return Ok(None);
        }

        Ok(Some(SeqId::from(array.value(0))))
    }

    fn build_where_clauses(
        &self,
        table_type: TableType,
        options: &InitialDataOptions,
        where_clause: Option<&str>,
        table_capabilities: TableCapabilities,
    ) -> Vec<String> {
        let mut where_clauses = Vec::new();

        if table_capabilities.has_commit_seq {
            match (options.since_commit_seq, options.since_seq) {
                (Some(since_commit), Some(since_seq)) => where_clauses.push(format!(
                    "({commit_col} > {since_commit} OR ({commit_col} = {since_commit} AND \
                     {seq_col} > {since_seq}))",
                    commit_col = SystemColumnNames::COMMIT_SEQ,
                    seq_col = SystemColumnNames::SEQ,
                    since_seq = since_seq.as_i64()
                )),
                (Some(since_commit), None) => where_clauses.push(format!(
                    "{} > {}",
                    SystemColumnNames::COMMIT_SEQ,
                    since_commit
                )),
                (None, Some(since_seq)) => where_clauses.push(format!(
                    "{} > {}",
                    SystemColumnNames::SEQ,
                    since_seq.as_i64()
                )),
                (None, None) => {},
            }

            if let Some(until_commit_seq) = options.until_commit_seq {
                where_clauses.push(format!(
                    "{} <= {}",
                    SystemColumnNames::COMMIT_SEQ,
                    until_commit_seq
                ));
            } else if let Some(until_seq) = options.until_seq {
                where_clauses.push(format!("{} <= {}", SystemColumnNames::SEQ, until_seq.as_i64()));
            }
        } else {
            if let Some(since) = options.since_seq {
                where_clauses.push(format!("{} > {}", SystemColumnNames::SEQ, since.as_i64()));
            }
            if let Some(until) = options.until_seq {
                where_clauses.push(format!("{} <= {}", SystemColumnNames::SEQ, until.as_i64()));
            }
        }

        if !options.include_deleted
            && matches!(table_type, TableType::User | TableType::Shared)
            && table_capabilities.has_deleted
        {
            where_clauses.push(format!("{} = false", SystemColumnNames::DELETED));
        }

        if let Some(where_sql) = where_clause {
            where_clauses.push(where_sql.to_string());
        }

        where_clauses
    }

    fn table_capabilities(&self, table_id: &TableId) -> Result<TableCapabilities, LiveError> {
        let schema = self.schema_lookup.get_arrow_schema(table_id)?;
        Ok(TableCapabilities {
            has_commit_seq: schema.field_with_name(SystemColumnNames::COMMIT_SEQ).is_ok(),
            has_deleted: schema.field_with_name(SystemColumnNames::DELETED).is_ok(),
        })
    }
}

async fn materialize_initial_rows(
    batches: Vec<RecordBatch>,
    has_commit_seq: bool,
    capacity_hint: usize,
) -> Result<Vec<(SeqId, Option<u64>, Row)>, LiveError> {
    let row_count = batches.iter().map(|batch| batch.num_rows()).sum::<usize>();
    if row_count <= BLOCKING_MATERIALIZATION_ROW_THRESHOLD {
        return materialize_initial_rows_sync(batches, has_commit_seq, capacity_hint);
    }

    tokio::task::spawn_blocking(move || {
        materialize_initial_rows_sync(batches, has_commit_seq, capacity_hint)
    })
    .await
    .map_err(|err| LiveError::Other(format!("initial data materialization task failed: {err}")))?
}

fn materialize_initial_rows_sync(
    batches: Vec<RecordBatch>,
    has_commit_seq: bool,
    capacity_hint: usize,
) -> Result<Vec<(SeqId, Option<u64>, Row)>, LiveError> {
    let mut rows_with_seq = Vec::with_capacity(capacity_hint);

    for batch in batches {
        let schema = batch.schema();
        let seq_col_idx = schema.index_of(SystemColumnNames::SEQ).map_err(|_| {
            LiveError::Other(format!("Result missing {} column", SystemColumnNames::SEQ))
        })?;

        let seq_col = batch.column(seq_col_idx);
        let seq_array = seq_col.as_any().downcast_ref::<Int64Array>().ok_or_else(|| {
            LiveError::Other(format!("{} column is not Int64", SystemColumnNames::SEQ))
        })?;
        let commit_seq_array = if has_commit_seq {
            let commit_idx = schema.index_of(SystemColumnNames::COMMIT_SEQ).map_err(|_| {
                LiveError::Other(format!("Result missing {} column", SystemColumnNames::COMMIT_SEQ))
            })?;
            Some(batch.column(commit_idx))
        } else {
            None
        };

        let num_rows = batch.num_rows();
        let num_cols = batch.num_columns();

        for row_idx in 0..num_rows {
            let mut row_map = BTreeMap::new();
            for col_idx in 0..num_cols {
                let col_name = schema.field(col_idx).name();
                if col_name == SystemColumnNames::COMMIT_SEQ {
                    continue;
                }
                let col_array = batch.column(col_idx);
                let value = ScalarValue::try_from_array(col_array, row_idx)
                    .into_serialization_error("Failed to convert to ScalarValue")?;
                row_map.insert(col_name.clone(), value);
            }

            let seq_id = SeqId::from(seq_array.value(row_idx));
            let commit_seq = commit_seq_array
                .as_ref()
                .and_then(|array| ScalarValue::try_from_array(array, row_idx).ok())
                .and_then(|value| match value {
                    ScalarValue::UInt64(Some(commit_seq)) => Some(commit_seq),
                    ScalarValue::Int64(Some(commit_seq)) if commit_seq >= 0 => {
                        Some(commit_seq as u64)
                    },
                    _ => None,
                });
            rows_with_seq.push((seq_id, commit_seq, Row::new(row_map)));
        }
    }

    Ok(rows_with_seq)
}

fn sort_initial_rows(
    rows_with_seq: &mut [(SeqId, Option<u64>, Row)],
    has_commit_seq: bool,
    options: &InitialDataOptions,
) {
    if has_commit_seq && options.since_commit_seq.is_some() {
        if options.fetch_last {
            rows_with_seq.sort_unstable_by(|left, right| {
                right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0))
            });
        } else {
            rows_with_seq.sort_unstable_by(|left, right| {
                left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0))
            });
        }
    } else if options.fetch_last {
        rows_with_seq.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    } else {
        rows_with_seq.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc as StdArc,
    };

    use arrow::record_batch::RecordBatch;
    use arrow::{
        array::{Int64Array, UInt64Array},
        datatypes::{DataType, Field, Schema},
    };
    use async_trait::async_trait;
    use kalamdb_commons::{
        models::{LiveQueryId, NamespaceId, TableName, UserId},
        schemas::TableDefinition,
    };
    use parking_lot::Mutex;

    use super::*;

    struct EmptySchemaLookup;

    impl LiveSchemaLookup for EmptySchemaLookup {
        fn get_table_definition(&self, _table_id: &TableId) -> Option<Arc<TableDefinition>> {
            None
        }

        fn get_arrow_schema(&self, _table_id: &TableId) -> Result<Arc<Schema>, LiveError> {
            Ok(Arc::new(Schema::empty()))
        }
    }

    struct CommitSeqSchemaLookup;

    impl LiveSchemaLookup for CommitSeqSchemaLookup {
        fn get_table_definition(&self, _table_id: &TableId) -> Option<Arc<TableDefinition>> {
            None
        }

        fn get_arrow_schema(&self, _table_id: &TableId) -> Result<Arc<Schema>, LiveError> {
            Ok(Arc::new(Schema::new(vec![
                Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
                Field::new(SystemColumnNames::COMMIT_SEQ, DataType::UInt64, false),
            ])))
        }
    }

    struct MaxSeqExecutor {
        seen_sql: Mutex<Option<String>>,
    }

    #[async_trait]
    impl LiveSqlExecutor for MaxSeqExecutor {
        async fn execute_for_batches(
            &self,
            sql: &str,
            _user_id: kalamdb_commons::models::UserId,
            _role: Role,
            _read_context: ReadContext,
        ) -> Result<Vec<RecordBatch>, LiveError> {
            *self.seen_sql.lock() = Some(sql.to_string());
            let schema = Arc::new(Schema::new(vec![Field::new("max_seq", DataType::Int64, true)]));
            let batch = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(vec![42]))])
                .map_err(|err| LiveError::Other(err.to_string()))?;
            Ok(vec![batch])
        }
    }

    struct CountingExecutor {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl LiveSqlExecutor for CountingExecutor {
        async fn execute_for_batches(
            &self,
            _sql: &str,
            _user_id: kalamdb_commons::models::UserId,
            _role: Role,
            _read_context: ReadContext,
        ) -> Result<Vec<RecordBatch>, LiveError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        }
    }

    struct CountingSchemaLookup {
        schema: Arc<Schema>,
        calls: StdArc<AtomicUsize>,
    }

    impl LiveSchemaLookup for CountingSchemaLookup {
        fn get_table_definition(&self, _table_id: &TableId) -> Option<Arc<TableDefinition>> {
            None
        }

        fn get_arrow_schema(&self, _table_id: &TableId) -> Result<Arc<Schema>, LiveError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Arc::clone(&self.schema))
        }
    }

    struct StaticBatchExecutor {
        batches: Vec<RecordBatch>,
    }

    #[async_trait]
    impl LiveSqlExecutor for StaticBatchExecutor {
        async fn execute_for_batches(
            &self,
            _sql: &str,
            _user_id: kalamdb_commons::models::UserId,
            _role: Role,
            _read_context: ReadContext,
        ) -> Result<Vec<RecordBatch>, LiveError> {
            Ok(self.batches.clone())
        }
    }

    struct CaptureFetchExecutor {
        seen_sql: Mutex<Vec<String>>,
        batches: Vec<RecordBatch>,
    }

    #[async_trait]
    impl LiveSqlExecutor for CaptureFetchExecutor {
        async fn execute_for_batches(
            &self,
            sql: &str,
            _user_id: kalamdb_commons::models::UserId,
            _role: Role,
            _read_context: ReadContext,
        ) -> Result<Vec<RecordBatch>, LiveError> {
            self.seen_sql.lock().push(sql.to_string());
            Ok(self.batches.clone())
        }
    }

    #[test]
    fn test_initial_data_options_default() {
        let options = InitialDataOptions::default();
        assert_eq!(options.since_seq, None);
        assert_eq!(options.limit, 100);
        assert!(!options.include_deleted);
        assert!(!options.fetch_last);
    }

    #[test]
    fn test_initial_data_options_since() {
        let seq = SeqId::new(12345);
        let options = InitialDataOptions::since(seq);
        assert_eq!(options.since_seq, Some(seq));
        assert_eq!(options.limit, 100);
        assert!(!options.include_deleted);
        assert!(!options.fetch_last);
    }

    #[test]
    fn test_initial_data_options_last() {
        let options = InitialDataOptions::last(50);
        assert_eq!(options.since_seq, None);
        assert_eq!(options.limit, 50);
        assert!(!options.include_deleted);
        assert!(options.fetch_last);
    }

    #[test]
    fn test_initial_data_options_builder() {
        let seq = SeqId::new(12345);
        let options = InitialDataOptions::since(seq).with_limit(200).with_deleted();

        assert_eq!(options.since_seq, Some(seq));
        assert_eq!(options.limit, 200);
        assert!(options.include_deleted);
    }

    fn test_row(id: &str) -> Row {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Utf8(Some(id.to_string())));
        Row::new(values)
    }

    #[test]
    fn sort_initial_rows_orders_seq_before_truncation() {
        let options = InitialDataOptions::batch(Some(SeqId::from(8)), Some(SeqId::from(30)), 3);
        let mut rows = vec![
            (SeqId::from(10), None, test_row("seed-10")),
            (SeqId::from(11), None, test_row("seed-11")),
            (SeqId::from(12), None, test_row("seed-12")),
            (SeqId::from(9), None, test_row("seed-9")),
        ];

        sort_initial_rows(&mut rows, false, &options);
        rows.truncate(options.limit);

        let ids: Vec<_> = rows
            .iter()
            .map(|(_, _, row)| match row.values.get("id") {
                Some(ScalarValue::Utf8(Some(id))) => id.as_str(),
                _ => "",
            })
            .collect();

        assert_eq!(ids, vec!["seed-9", "seed-10", "seed-11"]);
    }

    #[test]
    fn sort_initial_rows_orders_commit_seq_window() {
        let options = InitialDataOptions::batch(Some(SeqId::from(8)), Some(SeqId::from(30)), 3)
            .with_commit_range(Some(4), Some(9));
        let mut rows = vec![
            (SeqId::from(12), Some(6), test_row("c6-s12")),
            (SeqId::from(10), Some(5), test_row("c5-s10")),
            (SeqId::from(11), Some(5), test_row("c5-s11")),
            (SeqId::from(9), Some(5), test_row("c5-s9")),
        ];

        sort_initial_rows(&mut rows, true, &options);
        rows.truncate(options.limit);

        let ids: Vec<_> = rows
            .iter()
            .map(|(_, _, row)| match row.values.get("id") {
                Some(ScalarValue::Utf8(Some(id))) => id.as_str(),
                _ => "",
            })
            .collect();

        assert_eq!(ids, vec!["c5-s9", "c5-s10", "c5-s11"]);
    }

    #[tokio::test]
    async fn snapshot_boundary_uses_local_max_seq_query() {
        let fetcher = InitialDataFetcher::new(Arc::new(EmptySchemaLookup));
        let executor = Arc::new(MaxSeqExecutor {
            seen_sql: Mutex::new(None),
        });
        fetcher.set_sql_executor(executor.clone());

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );
        let options = InitialDataOptions::default().with_deleted();

        let boundary = fetcher
            .compute_snapshot_end_seq(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                &options,
                None,
            )
            .await
            .expect("snapshot boundary");

        assert_eq!(boundary, Some(SeqId::from(42)));
        assert_eq!(
            executor.seen_sql.lock().as_deref(),
            Some("SELECT MAX(_seq) AS max_seq FROM app.items")
        );
    }

    #[tokio::test]
    async fn snapshot_seq_boundary_skips_sql_when_commit_seq_is_available() {
        let fetcher = InitialDataFetcher::new(Arc::new(CommitSeqSchemaLookup));
        let executor = Arc::new(CountingExecutor {
            calls: AtomicUsize::new(0),
        });
        fetcher.set_sql_executor(executor.clone());

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let boundary = fetcher
            .compute_snapshot_end_seq(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                &InitialDataOptions::default(),
                None,
            )
            .await
            .expect("snapshot boundary check");

        assert_eq!(boundary, None);
        assert_eq!(executor.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn fetch_initial_data_reads_schema_once_for_system_column_capabilities() {
        let schema_calls = StdArc::new(AtomicUsize::new(0));
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
            Field::new(SystemColumnNames::COMMIT_SEQ, DataType::UInt64, false),
            Field::new(SystemColumnNames::DELETED, DataType::Boolean, false),
        ]));
        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: Arc::clone(&schema_calls),
        }));
        fetcher.set_sql_executor(Arc::new(CountingExecutor {
            calls: AtomicUsize::new(0),
        }));

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::batch(None, Some(SeqId::from(10)), 100),
                None,
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        assert!(result.rows.is_empty());
        assert_eq!(schema_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn fetch_initial_data_builds_seq_window_sql() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
        ]));
        let executor = Arc::new(CaptureFetchExecutor {
            seen_sql: Mutex::new(Vec::new()),
            batches: Vec::new(),
        });
        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: StdArc::new(AtomicUsize::new(0)),
        }));
        fetcher.set_sql_executor(executor.clone());

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::batch(Some(SeqId::from(10)), Some(SeqId::from(40)), 2),
                Some("id > 0"),
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        assert!(result.rows.is_empty());
        assert_eq!(
            executor.seen_sql.lock().as_slice(),
            ["SELECT id, _seq FROM app.items WHERE _seq > 10 AND _seq <= 40 AND id > 0 ORDER BY _seq ASC LIMIT 3"],
        );
    }

    #[tokio::test]
    async fn fetch_initial_data_builds_commit_seq_resume_sql() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
            Field::new(SystemColumnNames::COMMIT_SEQ, DataType::UInt64, false),
        ]));
        let executor = Arc::new(CaptureFetchExecutor {
            seen_sql: Mutex::new(Vec::new()),
            batches: Vec::new(),
        });
        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: StdArc::new(AtomicUsize::new(0)),
        }));
        fetcher.set_sql_executor(executor.clone());

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::batch(Some(SeqId::from(10)), None, 2)
                    .with_commit_range(Some(7), Some(9)),
                None,
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        assert!(result.rows.is_empty());
        assert_eq!(
            executor.seen_sql.lock().as_slice(),
            ["SELECT id, _seq, _commit_seq FROM app.items WHERE (_commit_seq > 7 OR (_commit_seq = 7 AND _seq > 10)) AND _commit_seq <= 9 ORDER BY _commit_seq ASC, _seq ASC LIMIT 3"],
        );
    }

    #[tokio::test]
    async fn materialize_initial_rows_extracts_resume_columns_and_rows() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
            Field::new(SystemColumnNames::COMMIT_SEQ, DataType::UInt64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![10, 20])),
                Arc::new(Int64Array::from(vec![100, 200])),
                Arc::new(UInt64Array::from(vec![7, 8])),
            ],
        )
        .expect("record batch");

        let rows = materialize_initial_rows(vec![batch], true, 3).await.expect("materialized rows");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, SeqId::from(100));
        assert_eq!(rows[0].1, Some(7));
        assert_eq!(rows[1].0, SeqId::from(200));
        assert_eq!(rows[1].1, Some(8));
        assert!(rows[0].2.values.contains_key("id"));
        assert!(!rows[0].2.values.contains_key(SystemColumnNames::COMMIT_SEQ));
    }

    #[tokio::test]
    async fn fetch_initial_data_returns_rows_in_seq_order() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![2, 1])),
                Arc::new(Int64Array::from(vec![20, 10])),
            ],
        )
        .expect("record batch");

        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: StdArc::new(AtomicUsize::new(0)),
        }));
        fetcher.set_sql_executor(Arc::new(StaticBatchExecutor {
            batches: vec![batch],
        }));

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::batch(None, Some(SeqId::from(20)), 10),
                None,
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        let ids: Vec<i64> = result
            .rows
            .iter()
            .map(|row| match row.values.get("id") {
                Some(ScalarValue::Int64(Some(value))) => *value,
                other => panic!("unexpected id value: {other:?}"),
            })
            .collect();

        assert_eq!(ids, vec![1, 2]);
    }

    #[tokio::test]
    async fn fetch_initial_data_exact_batch_boundary_is_ready() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(Int64Array::from(vec![10, 20])),
            ],
        )
        .expect("record batch");

        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: StdArc::new(AtomicUsize::new(0)),
        }));
        fetcher.set_sql_executor(Arc::new(StaticBatchExecutor {
            batches: vec![batch],
        }));

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::batch(None, Some(SeqId::from(20)), 2),
                None,
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        let ids: Vec<i64> = result
            .rows
            .iter()
            .map(|row| match row.values.get("id") {
                Some(ScalarValue::Int64(Some(value))) => *value,
                other => panic!("unexpected id value: {other:?}"),
            })
            .collect();

        assert_eq!(ids, vec![1, 2]);
        assert!(!result.has_more);
        assert_eq!(result.last_seq, Some(SeqId::from(20)));
        assert_eq!(result.snapshot_end_seq, Some(SeqId::from(20)));
    }

    #[tokio::test]
    async fn fetch_initial_data_resume_batch_preserves_cursor_state() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![2, 3, 4])),
                Arc::new(Int64Array::from(vec![20, 30, 40])),
            ],
        )
        .expect("record batch");

        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: StdArc::new(AtomicUsize::new(0)),
        }));
        fetcher.set_sql_executor(Arc::new(StaticBatchExecutor {
            batches: vec![batch],
        }));

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::batch(Some(SeqId::from(10)), Some(SeqId::from(40)), 2),
                None,
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        let ids: Vec<i64> = result
            .rows
            .iter()
            .map(|row| match row.values.get("id") {
                Some(ScalarValue::Int64(Some(value))) => *value,
                other => panic!("unexpected id value: {other:?}"),
            })
            .collect();

        assert_eq!(ids, vec![2, 3]);
        assert!(result.has_more);
        assert_eq!(result.last_seq, Some(SeqId::from(30)));
        assert_eq!(result.snapshot_end_seq, Some(SeqId::from(40)));
    }

    #[tokio::test]
    async fn fetch_last_rows_does_not_paginate_older_history() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![6, 5, 4, 3, 2, 1])),
                Arc::new(Int64Array::from(vec![60, 50, 40, 30, 20, 10])),
            ],
        )
        .expect("record batch");

        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: StdArc::new(AtomicUsize::new(0)),
        }));
        fetcher.set_sql_executor(Arc::new(StaticBatchExecutor {
            batches: vec![batch],
        }));

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::last(5),
                None,
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        let ids: Vec<i64> = result
            .rows
            .iter()
            .map(|row| match row.values.get("id") {
                Some(ScalarValue::Int64(Some(value))) => *value,
                other => panic!("unexpected id value: {other:?}"),
            })
            .collect();

        assert_eq!(ids, vec![2, 3, 4, 5, 6]);
        assert!(!result.has_more);
        assert_eq!(result.last_seq, Some(SeqId::from(60)));
        assert_eq!(result.snapshot_end_seq, Some(SeqId::from(60)));
    }

    #[tokio::test]
    async fn fetch_last_rows_returns_all_when_row_count_is_below_limit() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(SystemColumnNames::SEQ, DataType::Int64, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![2, 1])),
                Arc::new(Int64Array::from(vec![20, 10])),
            ],
        )
        .expect("record batch");

        let fetcher = InitialDataFetcher::new(Arc::new(CountingSchemaLookup {
            schema,
            calls: StdArc::new(AtomicUsize::new(0)),
        }));
        fetcher.set_sql_executor(Arc::new(StaticBatchExecutor {
            batches: vec![batch],
        }));

        let table_id = TableId::new(NamespaceId::from("app"), TableName::from("items"));
        let live_id = LiveQueryId::new(
            UserId::new("u1"),
            kalamdb_commons::models::ConnectionId::new("c1"),
            "sub1".to_string(),
        );

        let result = fetcher
            .fetch_initial_data(
                &live_id,
                Role::User,
                &table_id,
                TableType::User,
                InitialDataOptions::last(5),
                None,
                Some(&["id".to_string()]),
            )
            .await
            .expect("initial data fetch");

        let ids: Vec<i64> = result
            .rows
            .iter()
            .map(|row| match row.values.get("id") {
                Some(ScalarValue::Int64(Some(value))) => *value,
                other => panic!("unexpected id value: {other:?}"),
            })
            .collect();

        assert_eq!(ids, vec![1, 2]);
        assert!(!result.has_more);
        assert_eq!(result.last_seq, Some(SeqId::from(20)));
        assert_eq!(result.snapshot_end_seq, Some(SeqId::from(20)));
    }
}
