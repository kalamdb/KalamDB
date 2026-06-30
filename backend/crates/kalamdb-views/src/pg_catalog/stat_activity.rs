use std::sync::{Arc, OnceLock};

use datafusion::arrow::{
    array::{ArrayRef, Int64Builder, StringBuilder, TimestampMicrosecondBuilder},
    datatypes::{DataType, Field, Schema, SchemaRef, TimeUnit},
    record_batch::RecordBatch,
};
use kalamdb_commons::{Role, SystemTable};

use crate::{error::RegistryError, pg_catalog::PgCatalogView, sessions::SessionsSnapshotCallback};

fn schema() -> SchemaRef {
    static SCHEMA: OnceLock<SchemaRef> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            Arc::new(Schema::new(vec![
                Field::new("pid", DataType::Int64, true),
                Field::new("datname", DataType::Utf8, false),
                Field::new("usename", DataType::Utf8, true),
                Field::new("client_addr", DataType::Utf8, true),
                Field::new("state", DataType::Utf8, false),
                Field::new(
                    "backend_start",
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    false,
                ),
                Field::new("xact_start", DataType::Timestamp(TimeUnit::Microsecond, None), true),
                Field::new("query", DataType::Utf8, true),
                Field::new("backend_type", DataType::Utf8, false),
            ]))
        })
        .clone()
}

#[derive(Clone)]
pub struct PgStatActivityView {
    snapshot_callback: SessionsSnapshotCallback,
}

impl std::fmt::Debug for PgStatActivityView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgStatActivityView").finish_non_exhaustive()
    }
}

impl PgStatActivityView {
    pub fn new(snapshot_callback: SessionsSnapshotCallback) -> Self {
        Self { snapshot_callback }
    }
}

impl PgCatalogView for PgStatActivityView {
    fn name(&self) -> &'static str {
        "pg_stat_activity"
    }

    fn schema(&self) -> SchemaRef {
        schema()
    }

    fn required_system_table(&self) -> Option<SystemTable> {
        Some(SystemTable::Sessions)
    }

    fn compute_batch(&self, _role: Role) -> Result<RecordBatch, RegistryError> {
        let mut pids = Int64Builder::new();
        let mut datnames = StringBuilder::new();
        let mut users = StringBuilder::new();
        let mut client_addrs = StringBuilder::new();
        let mut states = StringBuilder::new();
        let mut backend_starts = TimestampMicrosecondBuilder::new();
        let mut xact_starts = TimestampMicrosecondBuilder::new();
        let mut queries = StringBuilder::new();
        let mut backend_types = StringBuilder::new();

        for session in (self.snapshot_callback)() {
            if let Some(pid) = session.backend_pid {
                pids.append_value(pid);
            } else {
                pids.append_null();
            }
            datnames.append_value(session.current_schema.as_deref().unwrap_or("kalam"));
            if let Some(user_id) = session.authenticated_user_id {
                users.append_value(user_id);
            } else {
                users.append_null();
            }
            if let Some(client_addr) = session.client_addr {
                client_addrs.append_value(client_addr);
            } else {
                client_addrs.append_null();
            }
            states.append_value(session.state);
            backend_starts.append_value(session.opened_at_ms.saturating_mul(1000));
            if session.transaction_id.is_some() {
                xact_starts.append_value(session.last_seen_at_ms.saturating_mul(1000));
            } else {
                xact_starts.append_null();
            }
            if let Some(last_method) = session.last_method {
                queries.append_value(last_method);
            } else {
                queries.append_null();
            }
            backend_types.append_value(session.origin);
        }

        RecordBatch::try_new(
            self.schema(),
            vec![
                Arc::new(pids.finish()) as ArrayRef,
                Arc::new(datnames.finish()) as ArrayRef,
                Arc::new(users.finish()) as ArrayRef,
                Arc::new(client_addrs.finish()) as ArrayRef,
                Arc::new(states.finish()) as ArrayRef,
                Arc::new(backend_starts.finish()) as ArrayRef,
                Arc::new(xact_starts.finish()) as ArrayRef,
                Arc::new(queries.finish()) as ArrayRef,
                Arc::new(backend_types.finish()) as ArrayRef,
            ],
        )
        .map_err(|error| RegistryError::Other(format!("failed to build pg_stat_activity: {error}")))
    }
}
