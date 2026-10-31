//! system.transactions virtual view
//!
//! **Type**: Virtual View (not backed by persistent storage)
//!
//! Provides the current set of active explicit transactions tracked by the
//! in-memory transaction coordinator.

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{schemas::TableDefinition, SystemTable};
use kalamdb_macros::table;
use parking_lot::RwLock;

use super::common::SystemViewProvider;
use crate::view_base::VirtualView;

crate::memoized_view_schema!(transactions_schema, TransactionsView);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionSnapshot {
    pub transaction_id: String,
    pub owner_id: String,
    pub origin: String,
    pub state: String,
    pub age_ms: i64,
    pub idle_ms: i64,
    pub write_count: i64,
    pub write_bytes: i64,
    pub touched_tables_count: i64,
    pub snapshot_commit_seq: i64,
}

/// Active-transaction snapshot callback type.
pub type TransactionsSnapshotCallback = Arc<dyn Fn() -> Vec<TransactionSnapshot> + Send + Sync>;

#[table(
    name = "transactions",
    view = true,
    comment = "Active explicit transactions across pg RPC, SQL batch, and internal origins"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionRow {
    /// Canonical explicit transaction identifier
    pub transaction_id: String,
    /// Human-readable execution owner identifier
    pub owner_id: String,
    /// Transaction origin surface
    pub origin: String,
    /// Current transaction lifecycle state
    pub state: String,
    /// Transaction age in milliseconds
    pub age_ms: i64,
    /// Milliseconds since the transaction last performed work
    pub idle_ms: i64,
    /// Number of staged mutations currently buffered
    pub write_count: i64,
    /// Approximate in-memory size of the staged write set
    pub write_bytes: i64,
    /// Number of tables referenced by the transaction
    pub touched_tables_count: i64,
    /// Committed snapshot boundary captured at BEGIN
    pub snapshot_commit_seq: i64,
}

/// Virtual view that snapshots active explicit transactions from memory.
pub struct TransactionsView {
    snapshot_callback: Arc<RwLock<Option<TransactionsSnapshotCallback>>>,
}

impl std::fmt::Debug for TransactionsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransactionsView")
            .field("has_callback", &self.snapshot_callback.read().is_some())
            .finish()
    }
}

impl TransactionsView {
    pub fn new() -> Self {
        Self {
            snapshot_callback: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_snapshot_callback(&self, callback: TransactionsSnapshotCallback) {
        *self.snapshot_callback.write() = Some(callback);
    }

    pub fn definition() -> TableDefinition {
        TransactionRow::definition()
    }
}

impl VirtualView for TransactionsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::Transactions
    }

    fn schema(&self) -> SchemaRef {
        transactions_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, crate::error::RegistryError> {
        let mut snapshot = self
            .snapshot_callback
            .read()
            .as_ref()
            .map(|callback| callback())
            .unwrap_or_default();

        snapshot.sort_by(|left, right| {
            left.origin
                .cmp(&right.origin)
                .then_with(|| left.transaction_id.cmp(&right.transaction_id))
        });

        let mut transaction_id_builder = StringBuilder::new();
        let mut owner_id_builder = StringBuilder::new();
        let mut origin_builder = StringBuilder::new();
        let mut state_builder = StringBuilder::new();
        let mut age_ms_builder = Int64Builder::new();
        let mut idle_ms_builder = Int64Builder::new();
        let mut write_count_builder = Int64Builder::new();
        let mut write_bytes_builder = Int64Builder::new();
        let mut touched_tables_count_builder = Int64Builder::new();
        let mut snapshot_commit_seq_builder = Int64Builder::new();

        for transaction in snapshot {
            transaction_id_builder.append_value(&transaction.transaction_id);
            owner_id_builder.append_value(&transaction.owner_id);
            origin_builder.append_value(&transaction.origin);
            state_builder.append_value(&transaction.state);
            age_ms_builder.append_value(transaction.age_ms);
            idle_ms_builder.append_value(transaction.idle_ms);
            write_count_builder.append_value(transaction.write_count);
            write_bytes_builder.append_value(transaction.write_bytes);
            touched_tables_count_builder.append_value(transaction.touched_tables_count);
            snapshot_commit_seq_builder.append_value(transaction.snapshot_commit_seq);
        }

        let columns: Vec<ArrayRef> = vec![
            Arc::new(transaction_id_builder.finish()),
            Arc::new(owner_id_builder.finish()),
            Arc::new(origin_builder.finish()),
            Arc::new(state_builder.finish()),
            Arc::new(age_ms_builder.finish()),
            Arc::new(idle_ms_builder.finish()),
            Arc::new(write_count_builder.finish()),
            Arc::new(write_bytes_builder.finish()),
            Arc::new(touched_tables_count_builder.finish()),
            Arc::new(snapshot_commit_seq_builder.finish()),
        ];

        RecordBatch::try_new(transactions_schema(), columns)
            .map_err(|error| crate::error::RegistryError::Other(error.to_string()))
    }
}

pub type TransactionsTableProvider = SystemViewProvider<TransactionsView>;
