//! One DataFusion provider for user, shared, and stream tables.
//!
//! User and shared scans go through [`VersionedTableEngine`]. Stream scans
//! stay on the append-log provider. TTL, expiry, and hard-delete stay stream
//! capabilities, and explicit stream transactions stay rejected.

mod capability;
mod ops;

use std::any::Any;

use async_trait::async_trait;
use datafusion::{
    arrow::datatypes::SchemaRef,
    catalog::Session,
    common::Statistics,
    datasource::TableProvider,
    error::Result as DataFusionResult,
    logical_expr::{dml::InsertOp, Expr, TableProviderFilterPushDown, TableType},
    physical_plan::ExecutionPlan,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    ids::VersionId,
    models::{rows::Row, UserId},
    TransactionId,
};
use kalamdb_transactions::{StagedMutation, TransactionOverlay};

use crate::{
    error::KalamDbError, shared_tables::SharedTableProvider, stream_tables::StreamTableEngine,
    table_engine::VersionedTableEngine, user_tables::UserTableProvider, utils::TableRowDml,
};

pub use capability::{
    ColdTierMode, MutationMode, RlsMode, ScanEngineKind, ScopeMode, TableCapabilityProfile,
    TransactionMode, VersionSource,
};

enum ProviderKind {
    Versioned(VersionedTableEngine),
    Stream(StreamTableEngine),
}

/// DataFusion provider registered for user, shared, and stream tables.
pub struct KalamTableProvider {
    profile: TableCapabilityProfile,
    kind: ProviderKind,
}

impl std::fmt::Debug for KalamTableProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KalamTableProvider")
            .field("profile", &self.profile)
            .field("scan_engine", &self.profile.scan_engine)
            .finish()
    }
}

impl KalamTableProvider {
    pub(crate) fn from_user(provider: UserTableProvider) -> Self {
        Self {
            profile: TableCapabilityProfile::user(),
            kind: ProviderKind::Versioned(VersionedTableEngine::User(provider)),
        }
    }

    pub(crate) fn from_shared(provider: SharedTableProvider) -> Self {
        Self {
            profile: TableCapabilityProfile::shared(),
            kind: ProviderKind::Versioned(VersionedTableEngine::Shared(provider)),
        }
    }

    pub fn from_stream(provider: StreamTableEngine) -> Self {
        Self {
            profile: TableCapabilityProfile::stream(),
            kind: ProviderKind::Stream(provider),
        }
    }

    pub fn profile(&self) -> TableCapabilityProfile {
        self.profile
    }

    pub(crate) fn as_user(&self) -> Option<&UserTableProvider> {
        match &self.kind {
            ProviderKind::Versioned(VersionedTableEngine::User(provider)) => Some(provider),
            _ => None,
        }
    }

    pub(crate) fn as_shared(&self) -> Option<&SharedTableProvider> {
        match &self.kind {
            ProviderKind::Versioned(VersionedTableEngine::Shared(provider)) => Some(provider),
            _ => None,
        }
    }

    pub fn as_stream(&self) -> Option<&StreamTableEngine> {
        match &self.kind {
            ProviderKind::Stream(provider) => Some(provider),
            _ => None,
        }
    }

    /// A future SQL view over a shared table scans through this provider.
    ///
    /// Read predicates and insert/update `WITH CHECK` stay inside the shared
    /// provider, which calls `SharedTableAuthorization`. There is no second
    /// scan engine.
    pub async fn scan_shared_relation(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<std::sync::Arc<dyn ExecutionPlan>> {
        let Some(shared) = self.as_shared() else {
            return Err(datafusion::error::DataFusionError::Plan(
                "shared view scan requires a shared table".into(),
            ));
        };
        if self.profile.rls_mode != RlsMode::ForcePolicyCheck {
            return Err(datafusion::error::DataFusionError::Plan(
                "shared view scan requires policy checks".into(),
            ));
        }
        shared.scan(state, projection, filters, limit).await
    }

    pub async fn columnar_hot_scan(
        &self,
        user_id: Option<&UserId>,
        snapshot: Option<VersionId>,
        projection: Option<&Vec<usize>>,
        filter: Option<&Expr>,
        limit: Option<usize>,
        keep_deleted: bool,
    ) -> Result<Option<(datafusion::arrow::record_batch::RecordBatch, usize)>, KalamDbError> {
        match &self.kind {
            ProviderKind::Versioned(engine) => {
                engine
                    .columnar_hot_scan(user_id, snapshot, projection, filter, limit, keep_deleted)
                    .await
            },
            ProviderKind::Stream(_) => Ok(None),
        }
    }

    pub async fn sum_hot_covering_integers(
        &self,
        user_id: Option<&UserId>,
        filters: &[Expr],
        value_column: &str,
    ) -> Result<Option<(u64, i128)>, KalamDbError> {
        match &self.kind {
            ProviderKind::Versioned(engine) => {
                engine.sum_hot_covering_integers(user_id, filters, value_column).await
            },
            ProviderKind::Stream(_) => Ok(None),
        }
    }

    pub async fn plan_literal_equality_updates(
        &self,
        transaction_id: &TransactionId,
        user_id: &UserId,
        snapshot_commit_seq: Option<VersionId>,
        overlay: Option<&TransactionOverlay>,
        seek_column: &str,
        seek_value: &ScalarValue,
        assignments: &[(String, ScalarValue)],
    ) -> Result<Option<Vec<StagedMutation>>, KalamDbError> {
        match &self.kind {
            ProviderKind::Versioned(engine) => {
                engine
                    .plan_literal_equality_updates(
                        transaction_id,
                        user_id,
                        snapshot_commit_seq,
                        overlay,
                        seek_column,
                        seek_value,
                        assignments,
                    )
                    .await
            },
            ProviderKind::Stream(_) => Ok(None),
        }
    }

    fn inner(&self) -> &dyn TableRowDml {
        match &self.kind {
            ProviderKind::Versioned(engine) => engine.as_row_dml(),
            ProviderKind::Stream(provider) => provider,
        }
    }
}

/// [`KalamTableProvider`] behind a DataFusion [`TableProvider`].
pub fn as_kalam_table(provider: &dyn TableProvider) -> Option<&KalamTableProvider> {
    (provider as &dyn Any).downcast_ref::<KalamTableProvider>()
}

/// Shared provider stored on [`KalamTableProvider`].
pub(crate) fn as_shared_table(provider: &dyn TableProvider) -> Option<&SharedTableProvider> {
    (provider as &dyn Any)
        .downcast_ref::<KalamTableProvider>()
        .and_then(KalamTableProvider::as_shared)
}

/// Stream engine stored on [`KalamTableProvider`].
pub fn as_stream_table(provider: &dyn TableProvider) -> Option<&StreamTableEngine> {
    (provider as &dyn Any)
        .downcast_ref::<KalamTableProvider>()
        .and_then(KalamTableProvider::as_stream)
}

#[async_trait]
impl TableProvider for KalamTableProvider {
    fn schema(&self) -> SchemaRef {
        self.inner().schema()
    }

    fn table_type(&self) -> TableType {
        self.inner().table_type()
    }

    fn get_column_default(&self, column: &str) -> Option<&Expr> {
        self.inner().get_column_default(column)
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner().supports_filters_pushdown(filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        self.inner().statistics()
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<std::sync::Arc<dyn ExecutionPlan>> {
        self.inner().scan(state, projection, filters, limit).await
    }

    async fn insert_into(
        &self,
        state: &dyn Session,
        input: std::sync::Arc<dyn ExecutionPlan>,
        insert_op: InsertOp,
    ) -> DataFusionResult<std::sync::Arc<dyn ExecutionPlan>> {
        self.inner().insert_into(state, input, insert_op).await
    }

    async fn delete_from(
        &self,
        state: &dyn Session,
        filters: Vec<Expr>,
    ) -> DataFusionResult<std::sync::Arc<dyn ExecutionPlan>> {
        self.inner().delete_from(state, filters).await
    }

    async fn update(
        &self,
        state: &dyn Session,
        assignments: Vec<(String, Expr)>,
        filters: Vec<Expr>,
    ) -> DataFusionResult<std::sync::Arc<dyn ExecutionPlan>> {
        self.inner().update(state, assignments, filters).await
    }
}

#[async_trait]
impl TableRowDml for KalamTableProvider {
    async fn insert_rows(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<usize, KalamDbError> {
        self.inner().insert_rows(user_id, rows, versions).await
    }

    async fn insert_rows_returning(
        &self,
        user_id: &UserId,
        rows: Vec<Row>,
        versions: &[VersionId],
    ) -> Result<Vec<datafusion::scalar::ScalarValue>, KalamDbError> {
        self.inner().insert_rows_returning(user_id, rows, versions).await
    }

    async fn update_row_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        updates: Row,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.inner().update_row_by_pk(user_id, pk_value, updates, version).await
    }

    async fn delete_row_by_pk(
        &self,
        user_id: &UserId,
        pk_value: &str,
        version: VersionId,
    ) -> Result<bool, KalamDbError> {
        self.inner().delete_row_by_pk(user_id, pk_value, version).await
    }
}
