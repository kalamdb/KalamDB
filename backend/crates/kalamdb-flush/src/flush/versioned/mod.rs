//! Scope-aware flush over the existing user and shared jobs.
//!
//! Segment encoding stays in those jobs. This type only chooses the scope.

mod shared;
mod user;

use std::sync::Arc;

use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_filestore::StorageCached;
use kalamdb_tables::{SharedTableIndexedStore, UserTableIndexedStore};

use super::base::{FlushJobResult, FlushScopeHook, FlushTableMetadata, TableFlush};
use crate::{FlushError, ManifestService};

pub use shared::SharedTableFlushJob;
pub use user::UserTableFlushJob;

pub enum VersionedTableFlushJob {
    User(UserTableFlushJob),
    Shared(SharedTableFlushJob),
}

impl VersionedTableFlushJob {
    pub fn user(
        table_id: Arc<kalamdb_commons::models::TableId>,
        store: Arc<UserTableIndexedStore>,
        schema: SchemaRef,
        storage_cached: Arc<StorageCached>,
        manifest_service: Arc<ManifestService>,
        metadata: FlushTableMetadata,
        scan_batch_size: usize,
        scope_hook: Arc<dyn FlushScopeHook>,
    ) -> Self {
        Self::User(UserTableFlushJob::new(
            table_id,
            store,
            schema,
            storage_cached,
            manifest_service,
            metadata,
            scan_batch_size,
            scope_hook,
        ))
    }

    pub fn shared(
        table_id: Arc<kalamdb_commons::models::TableId>,
        store: Arc<SharedTableIndexedStore>,
        schema: SchemaRef,
        storage_cached: Arc<StorageCached>,
        manifest_service: Arc<ManifestService>,
        metadata: FlushTableMetadata,
        scan_batch_size: usize,
        scope_hook: Arc<dyn FlushScopeHook>,
    ) -> Self {
        Self::Shared(SharedTableFlushJob::new(
            table_id,
            store,
            schema,
            storage_cached,
            manifest_service,
            metadata,
            scan_batch_size,
            scope_hook,
        ))
    }

    pub fn retain_latest_hot(self, retain: bool) -> Self {
        match self {
            Self::User(job) => Self::User(job.retain_latest_hot(retain)),
            Self::Shared(job) => Self::Shared(job.retain_latest_hot(retain)),
        }
    }

    pub fn execute(&self) -> Result<FlushJobResult, FlushError> {
        match self {
            Self::User(job) => job.execute(),
            Self::Shared(job) => job.execute(),
        }
    }
}
