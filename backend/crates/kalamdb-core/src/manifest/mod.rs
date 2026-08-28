//! Manifest adapter module.
//!
//! The manifest state machine, flush jobs, and tail compaction implementation live in
//! `kalamdb-flush`. This module keeps the historical `kalamdb_core::manifest` interface and wires
//! core-only context such as schema lookup into that crate.

use std::sync::Arc;

use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_commons::{schemas::TableType, TableId, UserId};
use kalamdb_filestore::StorageCached;
pub use kalamdb_flush::{
    compaction::{
        select_trailing_small_segments, SmallSegmentCompactionContext,
        SmallSegmentCompactionResult, SmallSegmentCompactionSelection,
    },
    flush, FlushDedupStats, FlushJobResult, FlushManifestHelper, FlushMetadata, FlushScopeHint,
    FlushScopeHook, FlushTableMetadata, ManifestService, NoopFlushScopeHook, SharedTableFlushJob,
    SharedTableFlushMetadata, TableFlush, UserTableFlushJob, UserTableFlushMetadata,
};
use kalamdb_flush::{FlushError, Result as FlushResult};
pub use kalamdb_tables::manifest::{
    ensure_manifest_ready, load_row_from_parquet_by_seq, manifest_helpers, planner,
    ManifestAccessPlanner, RowGroupSelection,
};

use crate::{
    app_context::AppContext,
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    schema_registry::CachedTableData,
    vector::{flush_shared_scope_vectors, flush_user_scope_vectors},
};

pub struct CoreFlushScopeHook {
    app_ctx: Arc<AppContext>,
}

impl CoreFlushScopeHook {
    pub fn new(app_ctx: Arc<AppContext>) -> Self {
        Self { app_ctx }
    }
}

impl FlushScopeHook for CoreFlushScopeHook {
    fn after_scope_write(
        &self,
        table_type: TableType,
        table_id: &TableId,
        user_id: Option<&UserId>,
        schema: &SchemaRef,
        storage_cached: &Arc<StorageCached>,
    ) -> FlushResult<()> {
        if let Some(sql_executor) = self.app_ctx.try_sql_executor() {
            sql_executor.clear_plan_cache();
        }

        match (table_type, user_id) {
            (TableType::User, Some(user_id)) => {
                flush_user_scope_vectors(&self.app_ctx, table_id, user_id, schema, storage_cached)
            },
            (TableType::Shared, None) => {
                flush_shared_scope_vectors(&self.app_ctx, table_id, schema, storage_cached)
            },
            (table_type, user_id) => Err(KalamDbError::InvalidOperation(format!(
                "invalid flush scope: table_type={:?}, user_id={:?}",
                table_type,
                user_id.map(UserId::as_str)
            ))),
        }
        .map_err(|error| FlushError::Other(error.to_string()))
    }
}

pub async fn preview_small_segment_compaction(
    app_ctx: &Arc<AppContext>,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
) -> Result<Option<SmallSegmentCompactionSelection>, KalamDbError> {
    kalamdb_flush::compaction::preview_small_segment_compaction(
        &app_ctx.manifest_service(),
        &app_ctx.config().flush.compaction,
        table_id,
        table_type,
        user_id,
    )
    .map_err(KalamDbError::from)
}

pub async fn compact_small_segments(
    app_ctx: &Arc<AppContext>,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
) -> Result<Option<SmallSegmentCompactionResult>, KalamDbError> {
    let Some(selection) =
        preview_small_segment_compaction(app_ctx, table_id, table_type, user_id).await?
    else {
        return Ok(None);
    };
    let context =
        small_segment_compaction_context(app_ctx, table_id, table_type, selection.schema_version)?;

    kalamdb_flush::compaction::compact_small_segments(&context, table_id, table_type, user_id)
        .await
        .map_err(KalamDbError::from)
}

fn small_segment_compaction_context(
    app_ctx: &Arc<AppContext>,
    table_id: &TableId,
    table_type: TableType,
    schema_version: u32,
) -> Result<SmallSegmentCompactionContext, KalamDbError> {
    let schema_registry = app_ctx.schema_registry();
    let table_def = schema_registry
        .get_table_if_exists(table_id)?
        .ok_or_else(|| KalamDbError::TableNotFound(format!("Table not found: {}", table_id)))?;
    if table_def.table_type != table_type {
        return Err(KalamDbError::InvalidOperation(format!(
            "Table type mismatch for {} during small-segment compaction: expected {:?}, found {:?}",
            table_id, table_type, table_def.table_type
        )));
    }

    let cached = if let Some(cached) = schema_registry.get(table_id) {
        if cached.schema_version == schema_version {
            cached.as_ref().clone()
        } else {
            let versioned_table = app_ctx
                .system_tables()
                .tables()
                .get_version(table_id, schema_version)
                .into_kalamdb_error("Failed to load versioned schema for small-segment compaction")?
                .ok_or_else(|| {
                    KalamDbError::Other(format!(
                        "Schema version {} not found for table {}",
                        schema_version, table_id
                    ))
                })?;
            CachedTableData::new(Arc::new(versioned_table))
        }
    } else {
        let versioned_table = app_ctx
            .system_tables()
            .tables()
            .get_version(table_id, schema_version)
            .into_kalamdb_error("Failed to load versioned schema for small-segment compaction")?
            .ok_or_else(|| {
                KalamDbError::Other(format!(
                    "Schema version {} not found for table {}",
                    schema_version, table_id
                ))
            })?;
        CachedTableData::new(Arc::new(versioned_table))
    };

    let primary_key_field = cached
        .table
        .columns
        .iter()
        .find(|column| column.is_primary_key)
        .map(|column| column.column_name.clone())
        .ok_or_else(|| {
            KalamDbError::InvalidOperation(format!(
                "Table {} has no primary key for small-segment compaction",
                table_id
            ))
        })?;
    let storage_cached = cached
        .storage_cached(&app_ctx.storage_registry())
        .into_kalamdb_error("Failed to resolve storage for small-segment compaction")?;

    Ok(SmallSegmentCompactionContext::new(
        app_ctx.manifest_service(),
        storage_cached,
        app_ctx.config().flush.compaction.clone(),
        schema_version,
        cached.arrow_schema()?,
        primary_key_field,
        cached.bloom_filter_columns().to_vec(),
        cached.indexed_columns().to_vec(),
        cached.table.table_options.parquet_compression(),
    ))
}
