use std::sync::Arc;

use datafusion::{arrow::datatypes::SchemaRef, datasource::TableProvider};
use kalamdb_commons::{models::schemas::TableDefinition, StorageId, TableId, UserId};
use kalamdb_store::StorageError;

use crate::{Manifest, ManifestCacheEntry};

// Notification service trait for data change notifications
mod notification_service;
pub use notification_service::NotificationService;

// Topic publisher trait for synchronous CDC publishing from table providers
mod topic_publisher;
pub use topic_publisher::TopicPublisher;

/// Interface for ManifestService implementations used by table providers.
#[async_trait::async_trait]
pub trait ManifestService: Send + Sync {
    /// Read a manifest through the canonical service path.
    ///
    /// Implementations should resolve memory first, then the local RocksDB manifest copy,
    /// then storage manifest.json, hydrating faster layers after lower-layer hits.
    fn get_or_load(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError>;

    /// Async version of get_or_load to avoid blocking the tokio runtime.
    async fn get_or_load_async(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Option<Arc<ManifestCacheEntry>>, StorageError>;

    fn validate_manifest(&self, manifest: &Manifest) -> Result<(), StorageError>;

    fn mark_as_stale(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError>;

    fn rebuild_manifest(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError>;

    fn mark_pending_write(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<(), StorageError>;

    fn ensure_manifest_initialized(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
    ) -> Result<Manifest, StorageError>;

    fn stage_before_flush(
        &self,
        table_id: &TableId,
        user_id: Option<&UserId>,
        manifest: &Manifest,
    ) -> Result<(), StorageError>;

    fn get_manifest_user_ids(&self, table_id: &TableId) -> Result<Vec<UserId>, StorageError>;
}

/// Interface for SchemaRegistry implementations used by table providers.
pub trait SchemaRegistry: Send + Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    fn get_arrow_schema(&self, table_id: &TableId) -> Result<SchemaRef, Self::Error>;

    fn get_table_if_exists(
        &self,
        table_id: &TableId,
    ) -> Result<Option<Arc<TableDefinition>>, Self::Error>;

    fn get_arrow_schema_for_version(
        &self,
        table_id: &TableId,
        schema_version: u32,
    ) -> Result<SchemaRef, Self::Error>;

    fn get_storage_id(&self, table_id: &TableId) -> Result<StorageId, Self::Error>;

    fn get_table_provider(
        &self,
        _table_id: &TableId,
    ) -> Option<Arc<dyn TableProvider + Send + Sync>> {
        None
    }
}

/// Interface for cluster leadership checks used by providers.
#[async_trait::async_trait]
pub trait ClusterCoordinator: Send + Sync {
    async fn is_cluster_mode(&self) -> bool;

    /// Check if this node is the Meta group leader (where all DML data lives).
    async fn is_meta_leader(&self) -> bool;

    /// Get the API address of the Meta group leader.
    async fn meta_leader_addr(&self) -> Option<String>;

    async fn is_leader_for_user(&self, user_id: &UserId) -> bool;

    async fn is_leader_for_shared(&self, table_id: &TableId) -> bool;

    async fn leader_addr_for_user(&self, user_id: &UserId) -> Option<String>;

    async fn leader_addr_for_shared(&self, table_id: &TableId) -> Option<String>;

    /// Production coordinators propose SHARED autocommit DML to the owner group.
    fn replicates_shared_writes(&self) -> bool {
        false
    }

    /// Replicate an autocommit insert. The default refuses so a test double
    /// keeps the local provider path.
    async fn propose_shared_insert(
        &self,
        _table_id: &TableId,
        _actor_user_id: &UserId,
        _rows: Vec<kalamdb_commons::models::rows::Row>,
    ) -> Result<usize, String> {
        Err("shared autocommit insert is not replicated through this coordinator".into())
    }

    /// Replicate an autocommit update. One entry covers every final row.
    async fn propose_shared_update(
        &self,
        _table_id: &TableId,
        _actor_user_id: &UserId,
        _pk_values: Vec<String>,
        _updates: Vec<kalamdb_commons::models::rows::Row>,
    ) -> Result<usize, String> {
        Err("shared autocommit update is not replicated through this coordinator".into())
    }

    /// Replicate an autocommit delete. One entry covers every final row.
    async fn propose_shared_delete(
        &self,
        _table_id: &TableId,
        _actor_user_id: &UserId,
        _pk_values: Vec<String>,
    ) -> Result<usize, String> {
        Err("shared autocommit delete is not replicated through this coordinator".into())
    }
}
