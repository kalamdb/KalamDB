//! Flush Job Executor
//!
//! **Phase 9 (T146)**: JobExecutor implementation for flush operations
//!
//! Handles flushing buffered writes from RocksDB to Parquet files.
//!
//! ## Responsibilities
//! - Flush user table data (UserTableFlushJob)
//! - Flush shared table data (SharedTableFlushJob)
//! - Flush stream table data (StreamTableFlushJob)
//! - Track flush metrics (rows flushed, files created, bytes written)
//!
//! ## Non-Blocking Execution
//! All flush operations are executed via `tokio::task::spawn_blocking` to avoid
//! blocking the async runtime. This is critical because:
//! - RocksDB operations are synchronous and can take 10-100ms+
//! - Parquet file writes involve I/O that can block
//! - High flush concurrency could starve the async executor
//!
//! ## Parameters Format
//! ```json
//! {
//!   "namespace_id": "default",
//!   "table_name": "users",
//!   "table_type": "User",
//!   "flush_threshold": 10000
//! }
//! ```

use std::{future::Future, sync::Arc};

use async_trait::async_trait;
use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_commons::{models::UserId, schemas::TableType, TableId};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    error_extensions::KalamDbResultExt,
    manifest::{CoreFlushScopeHook, ManifestService},
    providers::{SharedTableProvider, UserTableProvider},
    schema_registry::SchemaRegistry,
};
use kalamdb_flush::flush::{
    FlushJobResult, FlushScopeHint, FlushTableMetadata, SharedTableFlushJob, TableFlush,
    UserTableFlushJob,
};
use kalamdb_store::EntityStore;
use kalamdb_system::JobType;
use kalamdb_tables::{SharedTableIndexedStore, UserTableIndexedStore};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::executors::{
    segment_compact::SegmentCompactParams,
    shared_table_cleanup::cleanup_empty_shared_scope_if_needed,
    table_partition::hot_table_partition, JobContext, JobDecision, JobExecutor, JobParams,
};
use crate::AppContextJobsExt;

const MAX_POST_FLUSH_TASKS: usize = 2;

/// Typed parameters for flush operations (T189)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlushParams {
    /// Table identifier (required)
    pub table_id: TableId,
    /// Table type (required)
    pub table_type: TableType,
    /// Flush threshold in rows (optional, defaults to config)
    #[serde(default)]
    pub flush_threshold: Option<u64>,
}

impl JobParams for FlushParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        // TableId and TableType validation is handled by their constructors
        Ok(())
    }
}

enum FlushTarget {
    User(Arc<UserTableIndexedStore>),
    Shared(Arc<SharedTableIndexedStore>),
}

impl FlushTarget {
    fn resolve(
        schema_registry: &SchemaRegistry,
        table_id: &TableId,
        table_type: TableType,
    ) -> Result<Option<Self>, KalamDbError> {
        let provider_arc = match table_type {
            TableType::User | TableType::Shared => {
                schema_registry.get_provider(table_id).ok_or_else(|| {
                    KalamDbError::NotFound(format!(
                        "Table provider not registered for {} (id={})",
                        table_id, table_id
                    ))
                })?
            },
            TableType::Stream | TableType::System => return Ok(None),
        };

        match table_type {
            TableType::User => {
                let provider =
                    provider_arc.as_any().downcast_ref::<UserTableProvider>().ok_or_else(|| {
                        KalamDbError::InvalidOperation(
                            "Cached provider type mismatch for user table".into(),
                        )
                    })?;

                Ok(Some(Self::User(provider.store())))
            },
            TableType::Shared => {
                let provider = provider_arc
                    .as_any()
                    .downcast_ref::<SharedTableProvider>()
                    .ok_or_else(|| {
                        KalamDbError::InvalidOperation(
                            "Cached provider type mismatch for shared table".into(),
                        )
                    })?;

                Ok(Some(Self::Shared(provider.store())))
            },
            TableType::Stream | TableType::System => Ok(None),
        }
    }

    fn log_message(&self) -> &'static str {
        match self {
            FlushTarget::User(_) => "Executing UserTableFlushJob (non-blocking)",
            FlushTarget::Shared(_) => "Executing SharedTableFlushJob (non-blocking)",
        }
    }

    fn has_min_rows(self, min_rows: usize) -> bool {
        match self {
            FlushTarget::User(store) => {
                let partition = store.partition();
                store
                    .backend()
                    .scan(&partition, None, None, Some(min_rows))
                    .map(|iter| iter.count() >= min_rows)
                    .unwrap_or(true)
            },
            FlushTarget::Shared(store) => {
                let partition = store.partition();
                store
                    .backend()
                    .scan(&partition, None, None, Some(min_rows))
                    .map(|iter| iter.count() >= min_rows)
                    .unwrap_or(true)
            },
        }
    }
}

/// Flush Job Executor
///
/// Executes flush operations for buffered table data.
///
/// ## Two-Phase Distributed Execution
///
/// **Phase 1 - Local Work (all nodes)**:
/// Currently a no-op. Future: will delete flushed rows from RocksDB to free memory.
///
/// **Phase 2 - Leader Actions (leader only)**:
/// Full flush: read from RocksDB, write Parquet files to storage, update manifest,
/// delete flushed rows from RocksDB.
pub struct FlushExecutor {
    post_flush_permits: Arc<Semaphore>,
}

impl FlushExecutor {
    /// Create a new FlushExecutor
    pub fn new() -> Self {
        Self {
            post_flush_permits: Arc::new(Semaphore::new(MAX_POST_FLUSH_TASKS)),
        }
    }

    async fn run_blocking_flush<F>(
        task: F,
        error_context: &'static str,
    ) -> Result<FlushJobResult, KalamDbError>
    where
        F: FnOnce() -> kalamdb_flush::Result<FlushJobResult> + Send + 'static,
    {
        tokio::task::spawn_blocking(task)
            .await
            .map_err(|err| KalamDbError::InvalidOperation(format!("Flush task panicked: {}", err)))?
            .map_err(KalamDbError::from)
            .into_kalamdb_error(error_context)
    }

    async fn execute_target_flush(
        target: FlushTarget,
        app_ctx: Arc<AppContext>,
        table_id: Arc<TableId>,
        schema: SchemaRef,
        schema_registry: Arc<SchemaRegistry>,
        manifest_service: Arc<ManifestService>,
    ) -> Result<FlushJobResult, KalamDbError> {
        let cached = schema_registry
            .get(&table_id)
            .ok_or_else(|| KalamDbError::TableNotFound(format!("Table not found: {}", table_id)))?;
        let storage_cached = cached
            .storage_cached(&app_ctx.storage_registry())
            .into_kalamdb_error("Failed to get storage cache for flush")?;
        let metadata = FlushTableMetadata::new(
            cached.schema_version,
            cached.bloom_filter_columns().to_vec(),
            cached.indexed_columns().to_vec(),
        );
        let scan_batch_size = app_ctx.config().flush.flush_batch_size;
        let scope_hook = Arc::new(CoreFlushScopeHook::new(app_ctx));

        match target {
            FlushTarget::User(store) => {
                let flush_job = UserTableFlushJob::new(
                    table_id,
                    store,
                    schema,
                    storage_cached,
                    manifest_service,
                    metadata,
                    scan_batch_size,
                    scope_hook,
                );

                Self::run_blocking_flush(move || flush_job.execute(), "User table flush failed")
                    .await
            },
            FlushTarget::Shared(store) => {
                let flush_job = SharedTableFlushJob::new(
                    table_id,
                    store,
                    schema,
                    storage_cached,
                    manifest_service,
                    metadata,
                    scan_batch_size,
                    scope_hook,
                );

                Self::run_blocking_flush(move || flush_job.execute(), "Shared table flush failed")
                    .await
            },
        }
    }

    fn try_spawn_post_flush_task<F>(&self, task_name: &'static str, table_id: &TableId, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let permit = match Arc::clone(&self.post_flush_permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                log::trace!(
                    "Skipping {} for {}; post-flush maintenance is saturated",
                    task_name,
                    table_id
                );
                return;
            },
        };

        tokio::task::spawn(async move {
            let _permit = permit;
            future.await;
        });
    }

    fn segment_compaction_idempotency_key(
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<&UserId>,
    ) -> String {
        match (table_type, user_id) {
            (TableType::User, Some(user_id)) => {
                format!("SC:{}:user:{}", table_id, user_id.as_str())
            },
            (TableType::Shared, None) => format!("SC:{}:shared", table_id),
            _ => format!("SC:{}:{:?}", table_id, table_type),
        }
    }

    fn maybe_spawn_segment_compaction(
        &self,
        app_ctx: Arc<AppContext>,
        table_id: &TableId,
        table_type: TableType,
        scope_hints: &[FlushScopeHint],
    ) {
        if !app_ctx.config().flush.compaction.enabled || scope_hints.is_empty() {
            return;
        }

        let params_by_scope = scope_hints
            .iter()
            .filter_map(|scope_hint| match (table_type, scope_hint) {
                (TableType::Shared, FlushScopeHint::Shared) => Some(SegmentCompactParams {
                    table_id: table_id.clone(),
                    table_type,
                    user_id: None,
                }),
                (TableType::User, FlushScopeHint::User(user_id)) => Some(SegmentCompactParams {
                    table_id: table_id.clone(),
                    table_type,
                    user_id: Some(user_id.clone()),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();

        if params_by_scope.is_empty() {
            return;
        }

        let compact_app_ctx = app_ctx.clone();
        let compact_table_id = table_id.clone();

        // This enqueue path is lightweight and should not be dropped behind
        // heavier post-flush maintenance such as RocksDB partition compaction.
        tokio::task::spawn(async move {
            for params in params_by_scope {
                let idempotency_key = Self::segment_compaction_idempotency_key(
                    &compact_table_id,
                    table_type,
                    params.user_id.as_ref(),
                );

                let should_enqueue = match kalamdb_core::manifest::preview_small_segment_compaction(
                    &compact_app_ctx,
                    &compact_table_id,
                    table_type,
                    params.user_id.as_ref(),
                )
                .await
                {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(error) => {
                        log::warn!(
                            "Post-flush segment compaction preview failed for {}: {}",
                            compact_table_id,
                            error
                        );
                        false
                    },
                };

                if !should_enqueue {
                    continue;
                }

                let job_manager = compact_app_ctx.job_manager();
                match job_manager
                    .create_job_typed(JobType::SegmentCompact, params, Some(idempotency_key), None)
                    .await
                {
                    Ok(job_id) => log::trace!(
                        "Queued segment compaction job {} for {}",
                        job_id,
                        compact_table_id
                    ),
                    Err(KalamDbError::IdempotentConflict(_)) => {
                        log::trace!("Segment compaction already queued for {}", compact_table_id)
                    },
                    Err(error) => log::warn!(
                        "Failed to enqueue segment compaction for {}: {}",
                        compact_table_id,
                        error
                    ),
                }
            }
        });
    }

    /// Internal flush implementation used by both execute() and execute_leader()
    async fn do_flush(&self, ctx: &JobContext<FlushParams>) -> Result<JobDecision, KalamDbError> {
        // Parameters already validated in JobContext - type-safe access
        let params = ctx.params();
        let table_id = params.table_id.clone();
        let table_type = params.table_type;

        log::trace!("[{}] Flushing {} (type: {:?})", ctx.job_id, table_id, table_type);

        // Get dependencies from AppContext
        let app_ctx = &ctx.app_ctx;
        let schema_registry = app_ctx.schema_registry();

        // Check if table exists before attempting to flush
        // If table doesn't exist (e.g., dropped after flush job was queued), skip the job
        let table_def = match schema_registry.get_table_if_exists(&table_id)? {
            Some(def) => def,
            None => {
                ctx.log_info(&format!("Table {} no longer exists, skipping flush job", table_id));
                return Ok(JobDecision::Skipped {
                    message: format!("Table {} not found (table may have been dropped)", table_id),
                });
            },
        };

        // Verify table type matches expected type (safety check)
        if table_def.table_type != table_type {
            ctx.log_warn(&format!(
                "Table {} type mismatch: expected {:?}, found {:?}, skipping flush",
                table_id, table_type, table_def.table_type
            ));
            return Ok(JobDecision::Skipped {
                message: format!(
                    "Table {} type mismatch (expected {:?}, found {:?})",
                    table_id, table_type, table_def.table_type
                ),
            });
        }

        let target = match FlushTarget::resolve(&schema_registry, &table_id, table_type)? {
            Some(target) => target,
            None => match table_type {
                TableType::Stream => {
                    ctx.log_trace("Stream table flush not yet implemented");
                    return Ok(JobDecision::Completed {
                        message: Some(format!(
                            "Stream flush skipped (not implemented) for {}",
                            table_id
                        )),
                    });
                },
                TableType::System => {
                    return Err(KalamDbError::InvalidOperation(
                        "Cannot flush SYSTEM tables".to_string(),
                    ));
                },
                TableType::User | TableType::Shared => unreachable!(),
            },
        };

        // Get current Arrow schema from the registry (already includes system columns)
        let schema = schema_registry
            .get_arrow_schema(&table_id)
            .into_kalamdb_error(&format!("Arrow schema not found for {}", table_id))?;

        ctx.log_trace(target.log_message());

        let table_id_arc = Arc::new(table_id.clone());
        let result = Self::execute_target_flush(
            target,
            app_ctx.clone(),
            table_id_arc,
            schema,
            schema_registry.clone(),
            app_ctx.manifest_service(),
        )
        .await?;

        log::debug!(
            "[{}] Flush operation completed: {} rows flushed, {} files created",
            ctx.job_id,
            result.rows_flushed,
            result.parquet_files.len()
        );

        // Fire-and-forget: check if the shared table scope is empty and clean
        // up cold segments if so.  Also non-blocking to avoid stalling.
        if matches!(table_type, TableType::Shared) {
            let cleanup_app_ctx = app_ctx.clone();
            let cleanup_table_id = table_id.clone();
            let cleanup_job_id = ctx.job_id.clone();
            self.try_spawn_post_flush_task("post-flush shared cleanup", &table_id, async move {
                // Build a minimal JobContext just for the helper
                let params = FlushParams {
                    table_id: cleanup_table_id.clone(),
                    table_type: TableType::Shared,
                    flush_threshold: None,
                };
                let ctx =
                    crate::executors::JobContext::new(cleanup_app_ctx, cleanup_job_id, params);
                if let Err(e) = cleanup_empty_shared_scope_if_needed(&ctx, &cleanup_table_id).await
                {
                    log::warn!("Post-flush shared scope cleanup failed (non-critical): {}", e);
                }
            });
        }

        // Fire-and-forget: compact RocksDB partition after flush to reclaim
        // space from tombstones.  Compaction is an optimisation, not a
        // correctness requirement, so we must not block the job from being
        // marked "completed".  With max_background_jobs=2, synchronous
        // compaction under concurrent flush load was the root cause of
        // 90-120 s stalls observed in smoke tests.
        if let Some(partition) = hot_table_partition(table_type, &table_id) {
            let compact_table_id = table_id.clone();
            let compact_backend = app_ctx.storage_backend();
            self.try_spawn_post_flush_task("post-flush compaction", &table_id, async move {
                match tokio::task::spawn_blocking(move || {
                    compact_backend.compact_partition(&partition)
                })
                .await
                {
                    Ok(Ok(())) => {
                        log::trace!("Post-flush compaction completed for {}", compact_table_id);
                    },
                    Ok(Err(err)) => {
                        log::warn!("Post-flush compaction failed (non-critical): {}", err);
                    },
                    Err(err) => {
                        log::warn!("Post-flush compaction task panicked: {}", err);
                    },
                }
            });
        }

        self.maybe_spawn_segment_compaction(
            app_ctx.clone(),
            &table_id,
            table_type,
            &result.scope_hints,
        );

        Ok(JobDecision::Completed {
            message: Some(format!(
                "Flushed {} successfully ({} rows, {} files)",
                table_id,
                result.rows_flushed,
                result.parquet_files.len()
            )),
        })
    }
}

#[async_trait]
impl JobExecutor for FlushExecutor {
    type Params = FlushParams;

    fn job_type(&self) -> JobType {
        JobType::Flush
    }

    fn name(&self) -> &'static str {
        "FlushExecutor"
    }

    /// Pre-validate: skip flush job creation when the table has insufficient data in RocksDB.
    ///
    /// When `flush_threshold` is set, checks that the table has at least that many rows
    /// before proceeding. Otherwise, just checks for any data at all.
    /// This avoids creating unnecessary jobs (and the associated Raft overhead) for
    /// tables that have already been flushed or that have no buffered writes.
    async fn pre_validate(
        &self,
        app_ctx: &Arc<AppContext>,
        params: &Self::Params,
    ) -> Result<bool, KalamDbError> {
        let schema_registry = app_ctx.schema_registry();

        // If the table no longer exists, skip
        let table_def = match schema_registry.get_table_if_exists(&params.table_id)? {
            Some(def) => def,
            None => return Ok(false),
        };

        if table_def.table_type != params.table_type {
            return Ok(false);
        }

        // Minimum rows needed: flush_threshold or 1 (just check for any data)
        let min_rows = params.flush_threshold.unwrap_or(1).max(1) as usize;

        let target =
            match FlushTarget::resolve(&schema_registry, &params.table_id, table_def.table_type) {
                Ok(Some(target)) => target,
                Ok(None) => return Ok(true),
                Err(err) => {
                    log::trace!(
                        "Flush pre-validation skipped {} because target resolution failed: {}",
                        params.table_id,
                        err
                    );
                    return Ok(false);
                },
            };

        tokio::task::spawn_blocking(move || target.has_min_rows(min_rows))
            .await
            .map_err(|err| {
                KalamDbError::InvalidOperation(format!(
                    "Flush pre-validation task panicked: {}",
                    err
                ))
            })
    }

    /// Legacy single-phase execute - delegates to do_flush for backward compatibility
    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        self.do_flush(ctx).await
    }

    /// Local work phase - currently no-op on followers
    ///
    /// In future: will compact RocksDB to free memory without writing Parquet files.
    /// The actual RocksDB delete happens in the leader phase after Parquet is written.
    async fn execute_local(
        &self,
        ctx: &JobContext<Self::Params>,
    ) -> Result<JobDecision, KalamDbError> {
        ctx.log_trace("Flush local phase - preparing for leader actions");
        // No-op for now: followers don't do anything in local phase
        // The actual flush (including RocksDB deletion) happens in leader phase
        Ok(JobDecision::Completed {
            message: Some("Local phase completed".to_string()),
        })
    }

    /// Leader-only phase - full flush implementation
    ///
    /// Performs the complete flush operation:
    /// 1. Read buffered data from RocksDB
    /// 2. Write Parquet files to storage (local or S3)
    /// 3. Update manifest with new segment metadata
    /// 4. Delete flushed rows from RocksDB
    /// 5. Compact RocksDB to reclaim space
    async fn execute_leader(
        &self,
        ctx: &JobContext<Self::Params>,
    ) -> Result<JobDecision, KalamDbError> {
        ctx.log_trace("Flush leader phase - executing full flush");
        self.do_flush(ctx).await
    }
}

impl Default for FlushExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::NamespaceId;

    use super::*;

    #[test]
    fn test_executor_properties() {
        let executor = FlushExecutor::new();
        assert_eq!(executor.job_type(), JobType::Flush);
        assert_eq!(executor.name(), "FlushExecutor");
    }

    #[test]
    fn test_flush_params_validate() {
        let params = FlushParams {
            table_id: TableId::new(
                NamespaceId::default(),
                kalamdb_commons::TableName::new("users"),
            ),
            table_type: TableType::User,
            flush_threshold: Some(10000),
        };

        assert!(params.validate().is_ok());
    }

    #[test]
    fn segment_compaction_idempotency_keys_are_scope_specific() {
        let table_id =
            TableId::new(NamespaceId::new("app"), kalamdb_commons::TableName::new("events"));
        let user_a = UserId::from("user-a");
        let user_b = UserId::from("user-b");

        assert_eq!(
            FlushExecutor::segment_compaction_idempotency_key(&table_id, TableType::Shared, None),
            format!("SC:{}:shared", table_id)
        );
        assert_eq!(
            FlushExecutor::segment_compaction_idempotency_key(
                &table_id,
                TableType::User,
                Some(&user_a)
            ),
            format!("SC:{}:user:user-a", table_id)
        );
        assert_ne!(
            FlushExecutor::segment_compaction_idempotency_key(
                &table_id,
                TableType::User,
                Some(&user_a)
            ),
            FlushExecutor::segment_compaction_idempotency_key(
                &table_id,
                TableType::User,
                Some(&user_b)
            )
        );
    }
}
