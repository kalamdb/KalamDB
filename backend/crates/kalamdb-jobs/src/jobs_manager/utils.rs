use chrono::Utc;
use kalamdb_commons::{JobId, NodeId};
use kalamdb_core::{error::KalamDbError, error_extensions::KalamDbResultExt};
use kalamdb_raft::{commands::MetaCommand, NodeStatus};
use kalamdb_system::{providers::jobs::models::JobFilter, JobStatus, JobType};
use log::Level;

use super::types::JobsManager;

/// Lazy-formatting version of log_job_event.
/// Avoids String allocation when the log level is disabled.
macro_rules! log_job {
    ($self:expr, $job_id:expr, $level:expr, $($arg:tt)+) => {
        if log::log_enabled!($level) {
            $self.log_job_event($job_id, &$level, &format!($($arg)+));
        }
    };
}
pub(crate) use log_job;

impl JobsManager {
    /// Generate typed JobId with prefix
    ///
    /// Prefixes come from `JobType::short_prefix()`.
    pub(crate) fn generate_job_id(&self, job_type: &JobType) -> JobId {
        let prefix = job_type.short_prefix();

        // Generate UUID for uniqueness
        let uuid = uuid::Uuid::new_v4().to_string().replace("-", "");
        JobId::new(format!("{}-{}", prefix, &uuid[..12]))
    }

    /// Get active cluster node IDs for job fan-out.
    ///
    /// In cluster mode, returns nodes with status Active.
    /// In standalone mode (or if no nodes are reported), returns only this node.
    pub(crate) fn active_cluster_node_ids(&self) -> Vec<NodeId> {
        let app_ctx = self.get_attached_app_context();
        let cluster_info = app_ctx.executor().get_cluster_info();

        if !cluster_info.is_cluster_mode || cluster_info.nodes.is_empty() {
            return vec![self.node_id];
        }

        let mut nodes: Vec<NodeId> = cluster_info
            .nodes
            .iter()
            .filter(|n| !matches!(n.status, NodeStatus::Offline))
            .map(|n| n.node_id)
            .collect();

        if nodes.is_empty() {
            nodes.push(cluster_info.current_node_id);
        }

        nodes
    }

    /// Log job event to jobs.log file
    ///
    /// All log lines are prefixed with [JobId] for easy filtering.
    ///
    /// # Arguments
    /// * `job_id` - Job ID for log prefix
    /// * `level` - Log level (info, warn, error)
    /// * `message` - Log message
    pub(crate) fn log_job_event(&self, job_id: &JobId, level: &Level, message: &str) {
        // TODO: Implement dedicated jobs.log file appender (T137)
        // For now, use standard logging with [JobId] prefix
        match level {
            Level::Error => log::error!("[{}] {}", job_id.as_str(), message),
            Level::Warn => log::warn!("[{}] {}", job_id.as_str(), message),
            Level::Info => log::info!("[{}] {}", job_id.as_str(), message),
            Level::Debug => log::debug!("[{}] {}", job_id.as_str(), message),
            Level::Trace => log::trace!("[{}] {}", job_id.as_str(), message),
        }
    }

    /// Recover incomplete jobs on startup.
    ///
    /// Leader-only jobs (backup, restore, export, …) complete their job_node
    /// immediately and keep `system.jobs.status = Running` until leader actions
    /// finish. If the process dies in that window, job_node recovery finds
    /// nothing and the job would otherwise stay Running forever.
    pub(crate) async fn recover_incomplete_jobs(&self) -> Result<(), KalamDbError> {
        let app_ctx = self.get_attached_app_context();

        if app_ctx.executor().is_cluster_mode() {
            let statuses = vec![JobStatus::Running, JobStatus::Retrying];
            let job_nodes = self
                .job_nodes_provider
                .list_for_node_with_statuses_async(&self.node_id, &statuses, 100000)
                .await
                .into_kalamdb_error("Failed to list job_nodes for recovery")?;

            if job_nodes.is_empty() {
                log::debug!("No incomplete job_nodes to recover for this node");
            } else {
                log::warn!("Recovering {} incomplete job_nodes from previous run", job_nodes.len());

                for node in job_nodes {
                    let job_id = node.job_id.clone();
                    let cmd = kalamdb_raft::commands::MetaCommand::UpdateJobNodeStatus {
                        job_id,
                        node_id: self.node_id,
                        status: JobStatus::Queued,
                        error_message: Some("Node restarted".to_string()),
                        updated_at: chrono::Utc::now(),
                    };

                    app_ctx.executor().execute_meta(cmd).await.map_err(|e| {
                        KalamDbError::Other(format!("Failed to recover job_node via Raft: {}", e))
                    })?;
                }
            }
        }

        // Followers must not fail global jobs: a healthy leader may still be
        // inside backup/restore after the local job_node is already Completed.
        if self.is_cluster_leader().await {
            self.fail_jobs_without_in_progress_nodes("Server restarted").await?;
        }

        Ok(())
    }

    /// Fail Running/Retrying jobs that have no in-progress job_nodes.
    ///
    /// That is the leader-only hang/crash signature: local phase already
    /// marked the node Completed, then leader work died before updating
    /// `system.jobs`.
    pub(crate) async fn fail_jobs_without_in_progress_nodes(
        &self,
        reason: &str,
    ) -> Result<(), KalamDbError> {
        if !self.is_cluster_leader().await {
            return Ok(());
        }

        let filter = JobFilter {
            statuses: Some(vec![JobStatus::Running, JobStatus::Retrying]),
            limit: None,
            ..Default::default()
        };

        let running_jobs = self.list_jobs(filter).await?;
        if running_jobs.is_empty() {
            log::debug!("No incomplete jobs to recover");
            return Ok(());
        }

        let now_ms = Utc::now().timestamp_millis();
        for job in running_jobs {
            let job_nodes = self
                .job_nodes_provider
                .list_for_job_id_async(&job.job_id)
                .await
                .into_kalamdb_error("Failed to list job_nodes for zombie job recovery")?;
            if job_nodes.iter().any(|node| node.status.is_in_progress()) {
                continue;
            }

            let job_id = job.job_id.clone();
            log::warn!(
                "[{}] Failing leader-phase job with no in-progress job_nodes: {}",
                job_id.as_str(),
                reason
            );
            // Direct provider write: crash recovery must be visible immediately.
            // Raft FailJob can return before apply, leaving system.jobs Running.
            let mut failed = job;
            failed.status = JobStatus::Failed;
            failed.message = Some(reason.to_string());
            failed.updated_at = now_ms;
            failed.finished_at = Some(now_ms);
            self.jobs_provider
                .update_job_async(failed)
                .await
                .into_kalamdb_error("Failed to recover job")?;
            self.finalize_job_nodes(&job_id, JobStatus::Failed, Some(reason.to_string()))
                .await?;
            self.log_job_event(&job_id, &Level::Error, &format!("Job marked as failed ({reason})"));
        }

        Ok(())
    }

    /// Fail Running jobs whose executor is not alive in this process.
    ///
    /// Catches the same leader-only zombie without requiring a restart,
    /// while leaving jobs that are actually executing (or still queued on
    /// a job_node) alone.
    pub(crate) async fn fail_orphaned_running_jobs(&self) -> Result<(), KalamDbError> {
        if !self.is_cluster_leader().await {
            return Ok(());
        }

        const GRACE_MS: i64 = 30_000;
        let now_ms = Utc::now().timestamp_millis();
        let executing = self.executing_jobs.lock().clone();

        let filter = JobFilter {
            statuses: Some(vec![JobStatus::Running, JobStatus::Retrying]),
            limit: None,
            ..Default::default()
        };

        for job in self.list_jobs(filter).await? {
            if executing.contains(&job.job_id) {
                continue;
            }
            let started_at = job.started_at.unwrap_or(job.created_at);
            if now_ms.saturating_sub(started_at) < GRACE_MS {
                continue;
            }

            let job_nodes = self
                .job_nodes_provider
                .list_for_job_id_async(&job.job_id)
                .await
                .into_kalamdb_error("Failed to list job_nodes for orphan scan")?;
            if job_nodes.iter().any(|node| node.status.is_in_progress()) {
                continue;
            }

            let job_id = job.job_id.clone();
            let reason = "Job executor dropped without completing";
            log::error!("[{}] {}", job_id.as_str(), reason);
            let mut failed = job;
            failed.status = JobStatus::Failed;
            failed.message = Some(reason.to_string());
            failed.updated_at = now_ms;
            failed.finished_at = Some(now_ms);
            self.jobs_provider
                .update_job_async(failed)
                .await
                .into_kalamdb_error("Failed to fail orphaned job")?;
            self.finalize_job_nodes(&job_id, JobStatus::Failed, Some(reason.to_string()))
                .await?;
        }

        Ok(())
    }

    /// Finalize job_nodes for a job once the job reaches a terminal status.
    ///
    /// Ensures queued/running job_nodes do not remain stuck after the job
    /// is completed, failed, or cancelled.
    pub(crate) async fn finalize_job_nodes(
        &self,
        job_id: &JobId,
        status: JobStatus,
        error_message: Option<String>,
    ) -> Result<(), KalamDbError> {
        let job_nodes = self
            .job_nodes_provider
            .list_for_job_id_async(job_id)
            .await
            .into_kalamdb_error("Failed to list job_nodes for job finalization")?;

        if job_nodes.is_empty() {
            return Ok(());
        }

        let app_ctx = self.get_attached_app_context();
        let now = Utc::now();

        for node in job_nodes {
            if matches!(
                node.status,
                JobStatus::Completed
                    | JobStatus::Failed
                    | JobStatus::Cancelled
                    | JobStatus::Skipped
            ) {
                continue;
            }

            let cmd = MetaCommand::UpdateJobNodeStatus {
                job_id: job_id.clone(),
                node_id: node.node_id,
                status,
                error_message: error_message.clone(),
                updated_at: now,
            };

            app_ctx.executor().execute_meta(cmd).await.map_err(|e| {
                KalamDbError::Other(format!("Failed to finalize job_node via Raft: {}", e))
            })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{JobId, NodeId};
    use kalamdb_core::{app_context::AppContext, test_helpers::test_app_context_simple};
    use kalamdb_system::{providers::jobs::models::Job, JobNode, JobStatus, JobType};

    use crate::{init_job_manager, AppContextJobsExt};

    async fn ready_app() -> Arc<AppContext> {
        let app_ctx = test_app_context_simple();
        init_job_manager(&app_ctx);
        app_ctx.executor().start().await.unwrap();
        app_ctx.executor().initialize_cluster().await.unwrap();
        app_ctx.wire_raft_appliers();
        app_ctx
    }

    fn running_backup(job_id: &str, node_id: NodeId, now_ms: i64) -> Job {
        Job {
            job_id: JobId::new(job_id),
            job_type: JobType::Backup,
            status: JobStatus::Running,
            leader_status: None,
            parameters: Some(serde_json::json!({"backup_path": "/tmp/kdb_restore_stuck"})),
            message: None,
            exception_trace: None,
            idempotency_key: None,
            retry_count: 0,
            max_retries: 3,
            memory_used: None,
            cpu_used: None,
            created_at: now_ms,
            updated_at: now_ms,
            started_at: Some(now_ms),
            finished_at: None,
            node_id,
            leader_node_id: None,
            queue: None,
            priority: None,
        }
    }

    fn job_node(
        job_id: &str,
        node_id: NodeId,
        status: JobStatus,
        now_ms: i64,
        finished: bool,
    ) -> JobNode {
        JobNode {
            created_at: now_ms,
            updated_at: now_ms,
            started_at: Some(now_ms),
            finished_at: finished.then_some(now_ms),
            job_id: JobId::new(job_id),
            node_id,
            status,
            error_message: None,
        }
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn recover_fails_leader_only_running_job_when_job_node_already_completed() {
        let app_ctx = ready_app().await;
        let jobs_manager = app_ctx.job_manager();
        let node_id = *app_ctx.node_id().as_ref();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let job_id = "BK-7950b366d357";

        app_ctx
            .system_tables()
            .jobs()
            .create_job(running_backup(job_id, node_id, now_ms))
            .unwrap();
        app_ctx
            .system_tables()
            .job_nodes()
            .create_job_node(job_node(job_id, node_id, JobStatus::Completed, now_ms, true))
            .unwrap();

        jobs_manager.recover_incomplete_jobs().await.expect("recover incomplete jobs");

        let recovered = app_ctx
            .system_tables()
            .jobs()
            .get_job(&JobId::new(job_id))
            .unwrap()
            .expect("job must still exist");
        assert_eq!(recovered.status, JobStatus::Failed);
        assert!(recovered.finished_at.is_some(), "restart recovery must set finished_at");
        let message = recovered.message.unwrap_or_default();
        assert!(
            message.to_ascii_lowercase().contains("restart"),
            "expected restart failure message, got {message}"
        );
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn recover_does_not_fail_jobs_with_in_progress_job_nodes() {
        let app_ctx = ready_app().await;
        let jobs_manager = app_ctx.job_manager();
        let node_id = *app_ctx.node_id().as_ref();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let job_id = "FL-still-running-local";

        let mut flush = running_backup(job_id, node_id, now_ms);
        flush.job_type = JobType::Flush;
        app_ctx.system_tables().jobs().create_job(flush).unwrap();
        app_ctx
            .system_tables()
            .job_nodes()
            .create_job_node(job_node(job_id, node_id, JobStatus::Running, now_ms, false))
            .unwrap();

        jobs_manager.recover_incomplete_jobs().await.expect("recover incomplete jobs");

        let recovered = app_ctx
            .system_tables()
            .jobs()
            .get_job(&JobId::new(job_id))
            .unwrap()
            .expect("job must still exist");
        assert_eq!(recovered.status, JobStatus::Running);
        assert!(recovered.finished_at.is_none());
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn orphan_scan_fails_leader_only_running_job_without_live_executor() {
        let app_ctx = ready_app().await;
        let jobs_manager = app_ctx.job_manager();
        let node_id = *app_ctx.node_id().as_ref();
        let now_ms = chrono::Utc::now().timestamp_millis() - 45_000;
        let job_id = "BK-orphaned-no-executor";

        app_ctx
            .system_tables()
            .jobs()
            .create_job(running_backup(job_id, node_id, now_ms))
            .unwrap();
        app_ctx
            .system_tables()
            .job_nodes()
            .create_job_node(job_node(job_id, node_id, JobStatus::Completed, now_ms, true))
            .unwrap();

        jobs_manager.fail_orphaned_running_jobs().await.expect("orphan scan");

        let recovered = app_ctx
            .system_tables()
            .jobs()
            .get_job(&JobId::new(job_id))
            .unwrap()
            .expect("job must still exist");
        assert_eq!(recovered.status, JobStatus::Failed);
        assert!(recovered.finished_at.is_some());
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn orphan_scan_skips_job_still_tracked_as_executing() {
        let app_ctx = ready_app().await;
        let jobs_manager = app_ctx.job_manager();
        let node_id = *app_ctx.node_id().as_ref();
        let now_ms = chrono::Utc::now().timestamp_millis() - 45_000;
        let job_id = "BK-still-executing";

        app_ctx
            .system_tables()
            .jobs()
            .create_job(running_backup(job_id, node_id, now_ms))
            .unwrap();
        app_ctx
            .system_tables()
            .job_nodes()
            .create_job_node(job_node(job_id, node_id, JobStatus::Completed, now_ms, true))
            .unwrap();

        let _guard = jobs_manager.track_executing(JobId::new(job_id));
        jobs_manager.fail_orphaned_running_jobs().await.expect("orphan scan");

        let recovered = app_ctx
            .system_tables()
            .jobs()
            .get_job(&JobId::new(job_id))
            .unwrap()
            .expect("job must still exist");
        assert_eq!(recovered.status, JobStatus::Running);
        assert!(recovered.finished_at.is_none());
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn orphan_scan_does_not_fail_within_grace_period() {
        let app_ctx = ready_app().await;
        let jobs_manager = app_ctx.job_manager();
        let node_id = *app_ctx.node_id().as_ref();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let job_id = "BK-within-grace";

        app_ctx
            .system_tables()
            .jobs()
            .create_job(running_backup(job_id, node_id, now_ms))
            .unwrap();
        app_ctx
            .system_tables()
            .job_nodes()
            .create_job_node(job_node(job_id, node_id, JobStatus::Completed, now_ms, true))
            .unwrap();

        jobs_manager.fail_orphaned_running_jobs().await.expect("orphan scan");

        let recovered = app_ctx
            .system_tables()
            .jobs()
            .get_job(&JobId::new(job_id))
            .unwrap()
            .expect("job must still exist");
        assert_eq!(recovered.status, JobStatus::Running);
    }
}
