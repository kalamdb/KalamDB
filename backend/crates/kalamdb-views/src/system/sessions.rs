//! system.sessions virtual view
//!
//! **Type**: Virtual View (not backed by persistent storage)
//!
//! Provides the current set of active connection sessions tracked by the
//! shared backend session manager.

use std::sync::Arc;

use datafusion::arrow::{
    array::{ArrayRef, BooleanBuilder, Int64Builder, StringBuilder, TimestampMicrosecondBuilder},
    datatypes::SchemaRef,
    record_batch::RecordBatch,
};
use kalamdb_commons::{schemas::TableDefinition, SystemTable};
use kalamdb_macros::table;
use parking_lot::RwLock;

use super::common::{SystemViewProvider, Timestamp};
use crate::view_base::VirtualView;

crate::memoized_view_schema!(sessions_schema, SessionsView);

/// Serializable snapshot of a live connection session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionSessionSnapshot {
    pub session_id: String,
    pub origin: String,
    pub backend_pid: Option<i64>,
    pub authenticated_user_id: Option<String>,
    pub current_schema: Option<String>,
    pub state: String,
    pub transaction_id: Option<String>,
    pub transaction_state: Option<String>,
    pub transaction_has_writes: bool,
    pub client_addr: Option<String>,
    pub opened_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub last_method: Option<String>,
}

/// Active-session snapshot callback type.
pub type SessionsSnapshotCallback = Arc<dyn Fn() -> Vec<ConnectionSessionSnapshot> + Send + Sync>;

#[table(
    name = "sessions",
    view = true,
    comment = "Active connection sessions tracked by KalamDB"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRow {
    /// Server-side PostgreSQL gRPC session identifier
    pub session_id: String,
    /// Connection entry point that opened this session
    pub origin: String,
    /// Parsed PostgreSQL backend PID when session_id follows pg-<pid> or pg-<pid>-<config-hash>
    pub backend_pid: Option<i64>,
    /// Authenticated KalamDB user id for this connection session
    pub authenticated_user_id: Option<String>,
    /// Current schema reported by the connection session
    pub current_schema: Option<String>,
    /// Session state similar to pg_stat_activity semantics
    pub state: String,
    /// Active remote transaction identifier, if any
    pub transaction_id: Option<String>,
    /// Current remote transaction lifecycle state
    pub transaction_state: Option<String>,
    /// Whether the active transaction has performed writes
    pub transaction_has_writes: bool,
    /// Observed client socket address for the gRPC session
    pub client_addr: Option<String>,
    /// Transport used by the session
    pub transport: String,
    /// When the server first observed this session
    pub opened_at: Timestamp,
    /// Most recent RPC activity timestamp for this session
    pub last_seen_at: Timestamp,
    /// Most recent gRPC method observed for this session
    pub last_method: Option<String>,
}

/// Virtual view that snapshots active connection sessions from memory.
pub struct SessionsView {
    snapshot_callback: Arc<RwLock<Option<SessionsSnapshotCallback>>>,
}

impl std::fmt::Debug for SessionsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionsView")
            .field("has_callback", &self.snapshot_callback.read().is_some())
            .finish()
    }
}

impl SessionsView {
    pub fn new() -> Self {
        Self {
            snapshot_callback: Arc::new(RwLock::new(None)),
        }
    }

    pub fn set_snapshot_callback(&self, callback: SessionsSnapshotCallback) {
        *self.snapshot_callback.write() = Some(callback);
    }

    pub fn definition() -> TableDefinition {
        SessionRow::definition()
    }
}

impl VirtualView for SessionsView {
    fn system_table(&self) -> SystemTable {
        SystemTable::Sessions
    }

    fn schema(&self) -> SchemaRef {
        sessions_schema()
    }

    fn compute_batch(&self) -> Result<RecordBatch, crate::error::RegistryError> {
        let mut snapshot = self
            .snapshot_callback
            .read()
            .as_ref()
            .map(|callback| callback())
            .unwrap_or_default();

        snapshot.sort_by(|left, right| {
            right
                .last_seen_at_ms
                .cmp(&left.last_seen_at_ms)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });

        let mut session_ids = StringBuilder::new();
        let mut origins = StringBuilder::new();
        let mut backend_pids = Int64Builder::new();
        let mut authenticated_user_ids = StringBuilder::new();
        let mut current_schemas = StringBuilder::new();
        let mut states = StringBuilder::new();
        let mut transaction_ids = StringBuilder::new();
        let mut transaction_states = StringBuilder::new();
        let mut transaction_has_writes = BooleanBuilder::new();
        let mut client_addrs = StringBuilder::new();
        let mut transports = StringBuilder::new();
        let mut opened_ats = TimestampMicrosecondBuilder::new();
        let mut last_seen_ats = TimestampMicrosecondBuilder::new();
        let mut last_methods = StringBuilder::new();

        for session in snapshot {
            session_ids.append_value(&session.session_id);
            origins.append_value(&session.origin);

            if let Some(backend_pid) = session.backend_pid {
                backend_pids.append_value(backend_pid);
            } else {
                backend_pids.append_null();
            }

            if let Some(authenticated_user_id) = session.authenticated_user_id.as_deref() {
                authenticated_user_ids.append_value(authenticated_user_id);
            } else {
                authenticated_user_ids.append_null();
            }

            if let Some(current_schema) = session.current_schema.as_deref() {
                current_schemas.append_value(current_schema);
            } else {
                current_schemas.append_null();
            }

            states.append_value(&session.state);

            if let Some(transaction_id) = session.transaction_id.as_deref() {
                transaction_ids.append_value(transaction_id);
            } else {
                transaction_ids.append_null();
            }

            if let Some(transaction_state) = session.transaction_state.as_deref() {
                transaction_states.append_value(transaction_state);
            } else {
                transaction_states.append_null();
            }

            transaction_has_writes.append_value(session.transaction_has_writes);

            if let Some(client_addr) = session.client_addr.as_deref() {
                client_addrs.append_value(client_addr);
            } else {
                client_addrs.append_null();
            }

            transports.append_value(&session.origin);
            opened_ats.append_value(session.opened_at_ms * 1000);
            last_seen_ats.append_value(session.last_seen_at_ms * 1000);

            if let Some(last_method) = session.last_method.as_deref() {
                last_methods.append_value(last_method);
            } else {
                last_methods.append_null();
            }
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(session_ids.finish()) as ArrayRef,
                Arc::new(origins.finish()) as ArrayRef,
                Arc::new(backend_pids.finish()) as ArrayRef,
                Arc::new(authenticated_user_ids.finish()) as ArrayRef,
                Arc::new(current_schemas.finish()) as ArrayRef,
                Arc::new(states.finish()) as ArrayRef,
                Arc::new(transaction_ids.finish()) as ArrayRef,
                Arc::new(transaction_states.finish()) as ArrayRef,
                Arc::new(transaction_has_writes.finish()) as ArrayRef,
                Arc::new(client_addrs.finish()) as ArrayRef,
                Arc::new(transports.finish()) as ArrayRef,
                Arc::new(opened_ats.finish()) as ArrayRef,
                Arc::new(last_seen_ats.finish()) as ArrayRef,
                Arc::new(last_methods.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| {
            crate::error::RegistryError::Other(format!(
                "Failed to build system.sessions batch: {}",
                error
            ))
        })
    }
}

pub type SessionsTableProvider = SystemViewProvider<SessionsView>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_view_definition_has_expected_name() {
        let definition = SessionsView::definition();
        assert_eq!(definition.table_name.as_str(), "sessions");
        assert_eq!(definition.columns.len(), 14);
    }

    #[test]
    fn sessions_view_compute_batch_uses_snapshot_callback() {
        let view = SessionsView::new();
        let callback: SessionsSnapshotCallback = Arc::new(|| {
            vec![ConnectionSessionSnapshot {
                session_id: "pg-321-deadbeef".to_string(),
                origin: "extension_bridge".to_string(),
                backend_pid: Some(321),
                authenticated_user_id: Some("bridge-user".to_string()),
                current_schema: Some("app".to_string()),
                state: "idle in transaction".to_string(),
                transaction_id: Some("tx-pg-321-0".to_string()),
                transaction_state: Some("active".to_string()),
                transaction_has_writes: true,
                client_addr: Some("127.0.0.1:40000".to_string()),
                opened_at_ms: 1,
                last_seen_at_ms: 2,
                last_method: Some("ExecuteQuery".to_string()),
            }]
        });
        view.set_snapshot_callback(callback);

        let batch = view.compute_batch().expect("compute batch");
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 14);
    }
}
