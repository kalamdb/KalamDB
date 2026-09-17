use std::sync::Arc;

use kalamdb_core::{app_context::AppContext, error::KalamDbError};
use kalamdb_system::JobType;

use crate::{
    executors::job_cleanup::JobCleanupParams,
    scheduler_common::{classify_schedule_error, hourly_date_key, ScheduleErrorKind},
    JobsManager,
};

/// Scheduler for `system.jobs` / `system.job_nodes` history cleanup.
pub struct JobCleanupScheduler;

impl JobCleanupScheduler {
    /// Creates one idempotent hourly job-history cleanup job when expired
    /// terminal jobs exist. Disabled when `jobs.history_cleanup_interval_seconds` is 0.
    pub async fn check_and_schedule(
        app_context: &Arc<AppContext>,
        jobs_manager: &JobsManager,
    ) -> Result<(), KalamDbError> {
        let jobs_config = &app_context.config().jobs;
        if jobs_config.history_cleanup_interval_seconds == 0 {
            return Ok(());
        }

        let params = JobCleanupParams {
            retention_days: jobs_config.history_retention_days,
        };
        let idempotency_key =
            hourly_history_idempotency_key(JobType::JobCleanup, &hourly_date_key());

        match jobs_manager
            .create_job_typed(JobType::JobCleanup, params, Some(idempotency_key), None)
            .await
        {
            Ok(job_id) => {
                log::debug!("Created job history cleanup job {}", job_id.as_str());
            },
            Err(err) => match classify_schedule_error(&err) {
                ScheduleErrorKind::AlreadyActive => {
                    log::trace!("Job history cleanup job already exists (idempotent)");
                },
                ScheduleErrorKind::PreValidationSkipped => {
                    log::trace!("Job history cleanup skipped (no expired jobs)");
                },
                ScheduleErrorKind::Other => {
                    log::warn!("Failed to create job history cleanup job: {}", err);
                },
            },
        }

        Ok(())
    }
}

fn hourly_history_idempotency_key(job_type: JobType, date_key: &str) -> String {
    format!("{}:history:{}", job_type.short_prefix(), date_key)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{JobId, NodeId};
    use kalamdb_core::test_helpers::test_app_context_simple;
    use kalamdb_system::{providers::jobs::models::Job, JobStatus};

    use super::*;
    use crate::{init_job_manager, AppContextJobsExt};

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

    async fn ready_app() -> Arc<AppContext> {
        let app_ctx = test_app_context_simple();
        init_job_manager(&app_ctx);
        app_ctx.executor().start().await.unwrap();
        app_ctx.executor().initialize_cluster().await.unwrap();
        app_ctx.wire_raft_appliers();
        app_ctx
    }

    fn history_jobs(app_ctx: &Arc<AppContext>) -> Vec<Job> {
        app_ctx
            .system_tables()
            .jobs()
            .list_jobs()
            .unwrap()
            .into_iter()
            .filter(|job| job.job_type == JobType::JobCleanup)
            .collect()
    }

    #[test]
    fn hourly_history_key_uses_job_prefix() {
        assert_eq!(
            hourly_history_idempotency_key(JobType::JobCleanup, "2026-09-16-19"),
            "JC:history:2026-09-16-19"
        );
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn scheduler_skips_when_no_expired_history() {
        let app_ctx = ready_app().await;
        let jobs_manager = app_ctx.job_manager();

        JobCleanupScheduler::check_and_schedule(&app_ctx, &jobs_manager).await.unwrap();

        assert!(history_jobs(&app_ctx).is_empty());
    }

    #[tokio::test]
    #[ntest::timeout(15000)]
    async fn scheduler_creates_one_idempotent_job_when_history_is_expired() {
        let app_ctx = ready_app().await;
        app_ctx
            .system_tables()
            .jobs()
            .create_job(terminal_job("old_flush", 10))
            .unwrap();
        let jobs_manager = app_ctx.job_manager();

        JobCleanupScheduler::check_and_schedule(&app_ctx, &jobs_manager).await.unwrap();
        JobCleanupScheduler::check_and_schedule(&app_ctx, &jobs_manager).await.unwrap();

        let jobs = history_jobs(&app_ctx);
        assert_eq!(jobs.len(), 1);
        assert!(
            jobs[0]
                .idempotency_key
                .as_deref()
                .is_some_and(|key| key.starts_with("JC:history:")),
            "job cleanup should use the JC:history:<hour> idempotency format"
        );
    }
}
