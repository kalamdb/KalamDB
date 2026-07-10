//! Shared helpers for full-database backup and restore jobs.
//!
//! Backup and restore mutate the same on-disk storage trees (RocksDB staging,
//! Parquet files, snapshots, streams). They must not overlap each other or
//! in-flight flush / segment-compaction jobs that write those trees.

use std::{sync::LazyLock, time::Duration};

use kalamdb_core::error::KalamDbError;
use kalamdb_system::{providers::jobs::models::JobFilter, JobStatus, JobType};
use tokio::{sync::Mutex, time::sleep};

use crate::{
    executors::{JobContext, JobParams},
    AppContextJobsExt,
};

static DATABASE_TRANSFER_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Maximum time to wait for storage-writing jobs before backup/restore.
const STORAGE_QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const STORAGE_QUIESCENCE_POLL: Duration = Duration::from_millis(500);

/// Jobs that write Parquet / manifest data and must finish before transfer.
const STORAGE_QUIESCENCE_JOB_TYPES: [JobType; 2] = [JobType::Flush, JobType::SegmentCompact];

fn is_active_job_status(status: JobStatus) -> bool {
    matches!(
        status,
        JobStatus::New | JobStatus::Queued | JobStatus::Running | JobStatus::Retrying
    )
}

/// Serialize full-database backup/restore against each other.
pub(crate) async fn acquire_database_transfer_lock() -> tokio::sync::MutexGuard<'static, ()> {
    DATABASE_TRANSFER_LOCK.lock().await
}

/// Wait until no flush or segment-compaction jobs are active.
///
/// Skipped when the job manager is not initialized (unit tests with a minimal
/// AppContext).
pub(crate) async fn wait_for_storage_quiescence(
    ctx: &JobContext<impl JobParams>,
) -> Result<(), KalamDbError> {
    if !ctx.app_ctx.is_job_manager_initialized() {
        return Ok(());
    }

    let job_manager = ctx.app_ctx.job_manager();
    let deadline = tokio::time::Instant::now() + STORAGE_QUIESCENCE_TIMEOUT;

    loop {
        let mut blocking_jobs = Vec::new();

        for job_type in STORAGE_QUIESCENCE_JOB_TYPES {
            let filter = JobFilter {
                job_type: Some(job_type),
                ..Default::default()
            };
            let jobs = job_manager.list_jobs(filter).await?;
            for job in jobs {
                if is_active_job_status(job.status) {
                    blocking_jobs.push(format!("{} ({:?})", job.job_id, job.status));
                }
            }
        }

        if blocking_jobs.is_empty() {
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(KalamDbError::InvalidOperation(format!(
                "Timed out waiting for storage jobs to finish before database transfer: {}",
                blocking_jobs.join(", ")
            )));
        }

        ctx.log_debug(&format!(
            "Waiting for {} active storage job(s) before database transfer: {}",
            blocking_jobs.len(),
            blocking_jobs.join(", ")
        ));
        sleep(STORAGE_QUIESCENCE_POLL).await;
    }
}
