//! Job Cleanup Executor
//!
//! **Phase 9**: JobExecutor implementation for cleaning up old job history
//!
//! Handles retention of system.jobs / system.job_nodes to prevent infinite growth.
//!
//! ## Responsibilities
//! - Delete completed/failed/cancelled/skipped jobs older than retention period
//! - Track cleanup metrics
//!
//! ## Parameters Format
//! ```json
//! {
//!   "retention_days": 30
//! }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_core::{app_context::AppContext, error::KalamDbError};
use kalamdb_system::JobType;
use serde::{Deserialize, Serialize};

use crate::executors::{JobContext, JobDecision, JobExecutor, JobParams};

/// Typed parameters for job cleanup operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobCleanupParams {
    /// Retention period in days (required, must be > 0)
    pub retention_days: i64,
}

impl JobParams for JobCleanupParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        if self.retention_days <= 0 {
            return Err(KalamDbError::InvalidOperation(
                "retention_days must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}

/// Job Cleanup Executor
///
/// Executes cleanup operations for system.jobs table.
pub struct JobCleanupExecutor;

impl JobCleanupExecutor {
    /// Create a new JobCleanupExecutor
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl JobExecutor for JobCleanupExecutor {
    type Params = JobCleanupParams;

    fn job_type(&self) -> JobType {
        JobType::JobCleanup
    }

    fn name(&self) -> &'static str {
        "JobCleanupExecutor"
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        ctx.log_info("Starting job history cleanup operation");

        let params = ctx.params();
        let retention_days = params.retention_days;

        ctx.log_info(&format!("Cleaning up jobs older than {} days", retention_days));

        // Offload sync RocksDB scan+delete operations to blocking thread
        let app_ctx = ctx.app_ctx.clone();
        let (deleted_count, deleted_nodes) = tokio::task::spawn_blocking(move || {
            let jobs_provider = app_ctx.system_tables().jobs();
            let job_nodes_provider = app_ctx.system_tables().job_nodes();

            let deleted_count = jobs_provider.cleanup_old_jobs(retention_days).map_err(|e| {
                KalamDbError::ExecutionError(format!("Failed to cleanup old jobs: {}", e))
            })?;

            let deleted_nodes =
                job_nodes_provider.cleanup_old_job_nodes(retention_days).map_err(|e| {
                    KalamDbError::ExecutionError(format!("Failed to cleanup old job_nodes: {}", e))
                })?;

            Ok::<_, KalamDbError>((deleted_count, deleted_nodes))
        })
        .await
        .map_err(|e| KalamDbError::ExecutionError(format!("Task join error: {}", e)))??;

        let message = format!(
            "Job history cleanup completed - {} jobs and {} job_nodes deleted (retention: {} days)",
            deleted_count, deleted_nodes, retention_days
        );

        ctx.log_info(&message);

        Ok(JobDecision::Completed {
            message: Some(message),
        })
    }

    async fn pre_validate(
        &self,
        app_ctx: &Arc<AppContext>,
        params: &Self::Params,
    ) -> Result<bool, KalamDbError> {
        params.validate()?;
        let jobs = app_ctx.system_tables().jobs();
        let retention_days = params.retention_days;
        tokio::task::spawn_blocking(move || {
            jobs.has_expired_terminal_jobs(retention_days).map_err(|e| {
                KalamDbError::ExecutionError(format!("Failed to scan job history: {}", e))
            })
        })
        .await
        .map_err(|e| KalamDbError::ExecutionError(format!("Task join error: {}", e)))?
    }

    async fn cancel(&self, ctx: &JobContext<Self::Params>) -> Result<(), KalamDbError> {
        ctx.log_warn("Job cleanup cancellation requested");
        Ok(())
    }
}

impl Default for JobCleanupExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{JobId, NodeId};
    use kalamdb_core::test_helpers::test_app_context_simple;
    use kalamdb_system::{providers::jobs::models::Job, JobStatus};

    use super::*;

    fn terminal_job(job_id: &str, age_days: i64) -> Job {
        let now = chrono::Utc::now().timestamp_millis();
        let ts = now - age_days * 24 * 60 * 60 * 1000;
        Job {
            job_id:          JobId::new(job_id),
            job_type:        JobType::Flush,
            status:          JobStatus::Completed,
            leader_status:   None,
            parameters:      None,
            message:         Some("done".to_string()),
            exception_trace: None,
            idempotency_key: None,
            retry_count:     0,
            max_retries:     3,
            memory_used:     None,
            cpu_used:        None,
            created_at:      ts,
            updated_at:      ts,
            started_at:      Some(ts),
            finished_at:     Some(ts + 1),
            node_id:         NodeId::from(1u64),
            leader_node_id:  None,
            queue:           None,
            priority:        None,
        }
    }

    #[tokio::test]
    async fn pre_validate_skips_when_no_expired_history() {
        let app_ctx = test_app_context_simple();
        let executor = JobCleanupExecutor::new();
        let params = JobCleanupParams { retention_days: 7 };

        let should_run = executor.pre_validate(&app_ctx, &params).await.unwrap();
        assert!(!should_run);
    }

    #[tokio::test]
    async fn pre_validate_detects_expired_history() {
        let app_ctx = test_app_context_simple();
        app_ctx
            .system_tables()
            .jobs()
            .create_job(terminal_job("old_flush", 10))
            .unwrap();

        let executor = JobCleanupExecutor::new();
        let params = JobCleanupParams { retention_days: 7 };
        let should_run = executor.pre_validate(&app_ctx, &params).await.unwrap();
        assert!(should_run);
    }

    #[tokio::test]
    async fn execute_deletes_expired_terminal_jobs() {
        let app_ctx = test_app_context_simple();
        let jobs = app_ctx.system_tables().jobs();
        jobs.create_job(terminal_job("old_flush", 10)).unwrap();
        jobs.create_job(terminal_job("recent_flush", 1)).unwrap();

        let ctx = JobContext::new(
            app_ctx.clone(),
            "JC_test".to_string(),
            JobCleanupParams { retention_days: 7 },
        );
        let decision = JobCleanupExecutor::new().execute(&ctx).await.unwrap();
        assert!(matches!(decision, JobDecision::Completed { .. }));
        assert!(jobs.get_job_by_id(&JobId::new("old_flush")).unwrap().is_none());
        assert!(jobs.get_job_by_id(&JobId::new("recent_flush")).unwrap().is_some());
    }
}
