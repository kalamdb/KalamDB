//! Table export/import job executors.
//!
//! Table export is intentionally Parquet-first: it flushes hot RocksDB rows for the selected
//! table scope, then archives committed Parquet segments plus manifest metadata. Table import only
//! accepts this archive format so copied Parquet files are registered through the manifest service.

use std::{collections::HashMap, fs, time::Duration};

use async_trait::async_trait;
use bytes::Bytes;
use kalamdb_commons::{models::UserId, schemas::TableType, JobId, TableId};
use kalamdb_core::{error::KalamDbError, providers::UserTableProvider};
use kalamdb_store::EntityStore;
use kalamdb_system::{
    providers::jobs::models::JobFilter, JobStatus, JobType, Manifest, SegmentStatus,
};
use kalamdb_transfer::{
    import_segment_filename, is_safe_transfer_id, parse_transfer_user_id,
    read_table_export_archive, validate_table_transfer_scope, write_table_export_zip,
    TableExportArchive, TableExportArchiveMetadata, TableExportSegmentData,
};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::{
    executors::{flush::FlushParams, JobContext, JobDecision, JobExecutor, JobParams},
    AppContextJobsExt,
};

const FLUSH_WAIT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const FLUSH_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Typed parameters for single-table export operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableExportParams {
    pub table_id:   TableId,
    pub table_type: TableType,
    #[serde(default)]
    pub user_id:    Option<String>,
    pub export_id:  String,
}

impl JobParams for TableExportParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        validate_table_transfer_scope(self.table_type, self.user_id.as_deref())?;
        if self.export_id.is_empty() {
            return Err(KalamDbError::InvalidOperation("export_id cannot be empty".to_string()));
        }
        if !is_safe_transfer_id(&self.export_id) {
            return Err(KalamDbError::InvalidOperation(
                "export_id contains unsafe characters".to_string(),
            ));
        }
        Ok(())
    }
}

/// Typed parameters for importing a table export archive into one table scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableImportParams {
    pub table_id:   TableId,
    pub table_type: TableType,
    #[serde(default)]
    pub user_id:    Option<String>,
    pub import_id:  String,
}

impl JobParams for TableImportParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        validate_table_transfer_scope(self.table_type, self.user_id.as_deref())?;
        if self.import_id.is_empty() {
            return Err(KalamDbError::InvalidOperation("import_id cannot be empty".to_string()));
        }
        if !is_safe_transfer_id(&self.import_id) {
            return Err(KalamDbError::InvalidOperation(
                "import_id contains unsafe characters".to_string(),
            ));
        }
        Ok(())
    }
}

pub struct TableExportExecutor;

impl TableExportExecutor {
    pub fn new() -> Self {
        Self
    }
}

pub struct TableImportExecutor;

impl TableImportExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl JobExecutor for TableExportExecutor {
    type Params = TableExportParams;

    fn job_type(&self) -> JobType {
        JobType::TableExport
    }

    fn name(&self) -> &'static str {
        "TableExportExecutor"
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        let params = ctx.params();
        let target_user_id = parse_transfer_user_id(params.table_type, params.user_id.as_deref())?;
        let table_label = params.table_id.full_name();

        ctx.log_info(&format!(
            "Starting table export for {} (type={}, user_id={:?}, export_id={})",
            table_label,
            params.table_type.as_str(),
            target_user_id.as_ref().map(UserId::as_str),
            params.export_id
        ));

        let schema_registry = ctx.app_ctx.schema_registry();
        let table_def =
            schema_registry.get_table_if_exists(&params.table_id)?.ok_or_else(|| {
                KalamDbError::TableNotFound(format!("Table not found: {}", table_label))
            })?;

        if table_def.table_type != params.table_type {
            return Err(KalamDbError::InvalidOperation(format!(
                "Table {} has type {}, not {}",
                table_label,
                table_def.table_type.as_str(),
                params.table_type.as_str()
            )));
        }

        maybe_flush_table_scope(ctx, &params.table_id, params.table_type, target_user_id.as_ref())
            .await?;

        let storage_id = schema_registry.get_storage_id(&params.table_id)?;
        let storage_cached = ctx
            .app_ctx
            .storage_registry()
            .get_cached(&storage_id)
            .map_err(|error| KalamDbError::InvalidOperation(error.to_string()))?
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!("Storage '{}' not found", storage_id))
            })?;

        let manifest_entry = ctx
            .app_ctx
            .manifest_service()
            .get_or_load(&params.table_id, target_user_id.as_ref())
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("Failed to load manifest: {}", error))
            })?;

        let mut export_manifest = manifest_entry
            .map(|entry| entry.manifest.clone())
            .unwrap_or_else(|| Manifest::new(params.table_id.clone(), target_user_id.clone()));
        export_manifest
            .segments
            .retain(|segment| segment.status == SegmentStatus::Committed);

        let mut segments = Vec::with_capacity(export_manifest.segments.len());

        for segment in &export_manifest.segments {
            let get_result = match storage_cached
                .get(params.table_type, &params.table_id, target_user_id.as_ref(), &segment.path)
                .await
            {
                Ok(result) => result,
                Err(error) => {
                    let wrapped = KalamDbError::InvalidOperation(format!(
                        "Failed to read Parquet segment '{}' for {}: {}",
                        segment.path, table_label, error
                    ));
                    if is_missing_parquet_file_error(&wrapped) {
                        ctx.log_warn(&format!(
                            "Skipping missing Parquet segment '{}' for {} during export: {}",
                            segment.path, table_label, error
                        ));
                        continue;
                    }
                    return Err(wrapped);
                },
            };

            segments.push(TableExportSegmentData {
                source_path: segment.path.clone(),
                data:        get_result.data.to_vec(),
            });
        }

        let metadata = TableExportArchiveMetadata::new(
            params.table_id.clone(),
            params.table_type,
            target_user_id.clone(),
            table_def.as_ref().clone(),
            export_manifest,
        );

        let zip = write_table_export_zip(metadata, segments)?;
        let zip_size = zip.bytes.len();

        let exports_dir = ctx.app_ctx.config().storage.exports_dir().join("tables");
        fs::create_dir_all(&exports_dir).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create table exports directory '{}': {}",
                exports_dir.display(),
                error
            ))
        })?;
        let zip_path = exports_dir.join(format!("{}.zip", params.export_id));
        fs::write(&zip_path, &zip.bytes).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to write table export ZIP '{}': {}",
                zip_path.display(),
                error
            ))
        })?;

        ctx.log_info(&format!(
            "Table export complete: {} Parquet files, {} raw bytes, ZIP {} bytes written to {}",
            zip.file_count,
            zip.raw_bytes,
            zip_size,
            zip_path.display()
        ));

        Ok(JobDecision::Completed {
            message: Some(format!(
                "Exported {} Parquet files ({} bytes) to {}.zip",
                zip.file_count, zip_size, params.export_id
            )),
        })
    }

    async fn execute_leader(
        &self,
        ctx: &JobContext<Self::Params>,
    ) -> Result<JobDecision, KalamDbError> {
        self.execute(ctx).await
    }
}

#[async_trait]
impl JobExecutor for TableImportExecutor {
    type Params = TableImportParams;

    fn job_type(&self) -> JobType {
        JobType::TableImport
    }

    fn name(&self) -> &'static str {
        "TableImportExecutor"
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        let params = ctx.params();
        let target_user_id = parse_transfer_user_id(params.table_type, params.user_id.as_deref())?;
        let table_label = params.table_id.full_name();

        ctx.log_info(&format!(
            "Starting table import into {} (type={}, user_id={:?}, import_id={})",
            table_label,
            params.table_type.as_str(),
            target_user_id.as_ref().map(UserId::as_str),
            params.import_id
        ));

        let schema_registry = ctx.app_ctx.schema_registry();
        let table_def =
            schema_registry.get_table_if_exists(&params.table_id)?.ok_or_else(|| {
                KalamDbError::TableNotFound(format!("Table not found: {}", table_label))
            })?;

        if table_def.table_type != params.table_type {
            return Err(KalamDbError::InvalidOperation(format!(
                "Table {} has type {}, not {}",
                table_label,
                table_def.table_type.as_str(),
                params.table_type.as_str()
            )));
        }

        let imports_dir = ctx.app_ctx.config().storage.exports_dir().join("imports");
        let zip_path = imports_dir.join(format!("{}.zip", params.import_id));
        let zip_bytes = fs::read(&zip_path).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to read staged import ZIP '{}': {}",
                zip_path.display(),
                error
            ))
        })?;

        let TableExportArchive { metadata, files } = read_table_export_archive(zip_bytes)?;

        if metadata.source_table_type != params.table_type {
            return Err(KalamDbError::InvalidOperation(format!(
                "Exported table type {} cannot be imported into {} table {}",
                metadata.source_table_type.as_str(),
                params.table_type.as_str(),
                table_label
            )));
        }

        if metadata.table_definition.columns != table_def.columns {
            return Err(KalamDbError::InvalidOperation(format!(
                "Exported schema does not match target table {}",
                table_label
            )));
        }

        let source_segments_by_path = metadata
            .manifest
            .segments
            .iter()
            .filter(|segment| segment.status == SegmentStatus::Committed)
            .map(|segment| (segment.path.clone(), segment.clone()))
            .collect::<HashMap<_, _>>();

        let storage_id = schema_registry.get_storage_id(&params.table_id)?;
        let storage_cached = ctx
            .app_ctx
            .storage_registry()
            .get_cached(&storage_id)
            .map_err(|error| KalamDbError::InvalidOperation(error.to_string()))?
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!("Storage '{}' not found", storage_id))
            })?;

        let mut imported_segments = Vec::new();
        let mut total_bytes = 0usize;

        for (index, file) in files.iter().enumerate() {
            let data = &file.data;

            let target_filename =
                import_segment_filename(&params.import_id, index, &file.metadata.source_path);
            storage_cached
                .put(
                    params.table_type,
                    &params.table_id,
                    target_user_id.as_ref(),
                    &target_filename,
                    Bytes::from(data.clone()),
                )
                .await
                .map_err(|error| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to store imported Parquet segment '{}' for {}: {}",
                        target_filename, table_label, error
                    ))
                })?;

            let mut segment = source_segments_by_path
                .get(&file.metadata.source_path)
                .cloned()
                .ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "Export metadata has no committed manifest segment for '{}'",
                        file.metadata.source_path
                    ))
                })?;
            segment.id =
                format!("import-{}-{:05}-{}", params.import_id, index, uuid::Uuid::new_v4());
            segment.path = target_filename;
            segment.schema_version = table_def.schema_version;
            segment.size_bytes = data.len() as u64;
            segment.status = SegmentStatus::Committed;
            imported_segments.push(segment);
            total_bytes += data.len();
        }

        if imported_segments.is_empty() {
            return Ok(JobDecision::Completed {
                message: Some(format!(
                    "No Parquet files found in import archive for {}",
                    table_label
                )),
            });
        }

        let manifest_service = ctx.app_ctx.manifest_service();
        let mut target_manifest = manifest_service
            .ensure_manifest_initialized(&params.table_id, target_user_id.as_ref())
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!("Failed to load target manifest: {}", error))
            })?;
        target_manifest.table_id = params.table_id.clone();
        target_manifest.user_id = target_user_id.clone();
        for segment in imported_segments {
            target_manifest.add_segment(segment);
        }
        manifest_service
            .persist_manifest(&params.table_id, target_user_id.as_ref(), &target_manifest)
            .map_err(|error| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to persist target manifest: {}",
                    error
                ))
            })?;

        if let Err(error) = fs::remove_file(&zip_path) {
            ctx.log_warn(&format!(
                "Imported data successfully, but failed to remove staged ZIP '{}': {}",
                zip_path.display(),
                error
            ));
        }

        Ok(JobDecision::Completed {
            message: Some(format!(
                "Imported {} Parquet files ({} bytes) into {}",
                files.len(),
                total_bytes,
                table_label
            )),
        })
    }

    async fn execute_leader(
        &self,
        ctx: &JobContext<Self::Params>,
    ) -> Result<JobDecision, KalamDbError> {
        self.execute(ctx).await
    }
}

async fn maybe_flush_table_scope(
    ctx: &JobContext<TableExportParams>,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<&UserId>,
) -> Result<(), KalamDbError> {
    let has_user_hot_rows = if table_type == TableType::User {
        user_id.map(|uid| user_table_has_hot_rows(ctx, table_id, uid)).unwrap_or(true)
    } else {
        true
    };

    if !has_user_hot_rows {
        ctx.log_debug("Selected user scope has no RocksDB rows; copying existing Parquet data");
        return Ok(());
    }

    let job_manager = ctx.app_ctx.job_manager();
    let flush_params = FlushParams {
        table_id: table_id.clone(),
        table_type,
        flush_threshold: None,
    };
    let flush_key = format!("table-export-flush:{}:{}", table_id, table_type.as_str());

    let mut flush_job: Option<JobId> = None;
    match job_manager
        .create_job_typed(JobType::Flush, flush_params, Some(flush_key.clone()), None)
        .await
    {
        Ok(job_id) => {
            ctx.log_debug(&format!("Submitted flush job {} for {}", job_id, table_id.full_name()));
            flush_job = Some(job_id);
        },
        Err(KalamDbError::IdempotentConflict(_)) => {
            ctx.log_debug(
                "A matching flush job is already active; proceeding after existing data is \
                 available",
            );
            flush_job = find_active_flush_job(&job_manager, &flush_key).await?;
        },
        Err(KalamDbError::Other(ref message))
            if message.contains("pre-validation returned false") =>
        {
            ctx.log_debug("No hot rows need flushing before table export");
        },
        Err(error) => return Err(error),
    }

    if let Some(job_id) = flush_job {
        wait_for_flush_job(ctx, &job_id).await?;
    }

    Ok(())
}

async fn find_active_flush_job(
    job_manager: &crate::jobs_manager::JobsManager,
    idempotency_key: &str,
) -> Result<Option<JobId>, KalamDbError> {
    let filter = JobFilter {
        job_type: Some(JobType::Flush),
        idempotency_key: Some(idempotency_key.to_string()),
        ..Default::default()
    };

    Ok(job_manager
        .list_jobs(filter)
        .await?
        .into_iter()
        .find(|job| {
            matches!(
                job.status,
                JobStatus::New | JobStatus::Queued | JobStatus::Running | JobStatus::Retrying
            )
        })
        .map(|job| job.job_id))
}

async fn wait_for_flush_job(
    ctx: &JobContext<TableExportParams>,
    job_id: &JobId,
) -> Result<(), KalamDbError> {
    let job_manager = ctx.app_ctx.job_manager();
    let deadline = tokio::time::Instant::now() + FLUSH_WAIT_TIMEOUT;

    loop {
        if tokio::time::Instant::now() >= deadline {
            ctx.log_warn(&format!(
                "Timed out waiting for flush job {}; proceeding with available Parquet data",
                job_id
            ));
            return Ok(());
        }

        match job_manager.get_job(job_id).await? {
            Some(job) => {
                if matches!(
                    job.status,
                    JobStatus::Completed
                        | JobStatus::Failed
                        | JobStatus::Cancelled
                        | JobStatus::Skipped
                ) {
                    if job.status != JobStatus::Completed && job.status != JobStatus::Skipped {
                        ctx.log_warn(&format!(
                            "Flush job {} finished with status {}; export will copy available data",
                            job_id, job.status
                        ));
                    }
                    return Ok(());
                }
            },
            None => return Ok(()),
        }

        sleep(FLUSH_POLL_INTERVAL).await;
    }
}

fn user_table_has_hot_rows(
    ctx: &JobContext<TableExportParams>,
    table_id: &TableId,
    user_id: &UserId,
) -> bool {
    let Some(provider_arc) = ctx.app_ctx.schema_registry().get_provider(table_id) else {
        return false;
    };
    let Some(provider) =
        (provider_arc.as_ref() as &dyn std::any::Any).downcast_ref::<UserTableProvider>()
    else {
        return true;
    };

    let store = provider.store();
    let user_prefix = kalamdb_commons::ids::UserTableRowId::user_prefix(user_id);
    let partition = store.partition();
    store
        .backend()
        .scan(&partition, Some(&user_prefix), None, Some(1))
        .map(|mut iter| iter.next().is_some())
        .unwrap_or(true)
}

impl Default for TableExportExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for TableImportExecutor {
    fn default() -> Self {
        Self::new()
    }
}

fn is_missing_parquet_file_error(err: &KalamDbError) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("not found")
        && (msg.contains("object at location")
            || msg.contains("no such file or directory")
            || msg.contains("failed to open parquet stream")
            || msg.contains("/batch-"))
}
