use std::sync::Arc;

use kalamdb_core::{app_context::AppContext, error::KalamDbError};
use kalamdb_system::JobType;

use crate::{
    executors::topic_retention::TopicRetentionParams,
    scheduler_common::{
        classify_schedule_error, hourly_date_key, hourly_topic_idempotency_key, ScheduleErrorKind,
    },
    JobsManager,
};

/// Scheduler for topic retention jobs.
pub struct TopicRetentionScheduler;

impl TopicRetentionScheduler {
    /// Scans system.topics for topics with at least one retention limit and creates
    /// one idempotent hourly retention job per topic.
    pub async fn check_and_schedule(
        app_context: &Arc<AppContext>,
        jobs_manager: &JobsManager,
    ) -> Result<(), KalamDbError> {
        let topics = app_context.system_tables().topics().list_topics()?;
        let date_key = hourly_date_key();
        let batch_size = app_context.config().topics.retention_batch_size;
        let mut topics_found = 0usize;
        let mut jobs_created = 0usize;

        for topic in topics {
            if topic.retention_seconds.is_none() && topic.retention_max_bytes.is_none() {
                continue;
            }

            topics_found += 1;
            let params = TopicRetentionParams {
                topic_id: topic.topic_id.clone(),
                partition_id: None,
                batch_size,
            };
            let idempotency_key =
                hourly_topic_idempotency_key(JobType::TopicRetention, &topic.topic_id, &date_key);

            match jobs_manager
                .create_job_typed(JobType::TopicRetention, params, Some(idempotency_key), None)
                .await
            {
                Ok(job_id) => {
                    jobs_created += 1;
                    log::debug!(
                        "Created topic retention job {} for {}",
                        job_id.as_str(),
                        topic.topic_id.as_str()
                    );
                },
                Err(err) => match classify_schedule_error(&err) {
                    ScheduleErrorKind::AlreadyActive => {
                        log::trace!(
                            "Topic retention job for {} already exists (idempotent)",
                            topic.topic_id.as_str()
                        );
                    },
                    ScheduleErrorKind::PreValidationSkipped => {
                        log::trace!(
                            "Topic retention job for {} skipped by pre-validation",
                            topic.topic_id.as_str()
                        );
                    },
                    ScheduleErrorKind::Other => {
                        log::warn!(
                            "Failed to create topic retention job for {}: {}",
                            topic.topic_id.as_str(),
                            err
                        );
                    },
                },
            }
        }

        if topics_found > 0 {
            log::trace!(
                "Topic retention check: found {} retained topics, created {} new jobs",
                topics_found,
                jobs_created
            );
        } else {
            log::trace!("Topic retention check: no topics with retention limits found");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{panic::{catch_unwind, AssertUnwindSafe}, sync::Arc};

    use kalamdb_commons::models::TopicId;
    use kalamdb_core::test_helpers::test_app_context;
    use kalamdb_system::{providers::topics::models::Topic, JobType};

    use super::*;
    use crate::{init_job_manager, AppContextJobsExt};

    #[tokio::test]
    async fn test_scheduler_creates_one_idempotent_job_per_retained_topic() {
        let app_ctx = test_app_context();
        if catch_unwind(AssertUnwindSafe(|| app_ctx.job_manager())).is_err() {
            init_job_manager(&app_ctx);
        }

        let retained_topic_id = TopicId::new(&format!(
            "topic.scheduler.retained.{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let disabled_topic_id = TopicId::new(&format!(
            "topic.scheduler.disabled.{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));

        let mut retained_topic = Topic::new(
            retained_topic_id.clone(),
            retained_topic_id.as_str().to_string(),
        );
        retained_topic.retention_seconds = Some(60);

        let disabled_topic = Topic::new(
            disabled_topic_id.clone(),
            disabled_topic_id.as_str().to_string(),
        );

        app_ctx.system_tables().topics().create_topic(retained_topic).unwrap();
        app_ctx.system_tables().topics().create_topic(disabled_topic).unwrap();

        let jobs_manager = app_ctx.job_manager();
        TopicRetentionScheduler::check_and_schedule(&app_ctx, &jobs_manager).await.unwrap();
        TopicRetentionScheduler::check_and_schedule(&app_ctx, &jobs_manager).await.unwrap();

        let jobs: Vec<_> = app_ctx
            .system_tables()
            .jobs()
            .list_jobs()
            .unwrap()
            .into_iter()
            .filter(|job| {
                job.job_type == JobType::TopicRetention
                    && job
                        .idempotency_key
                        .as_deref()
                        .is_some_and(|key| key.starts_with(&format!("TR:{}:", retained_topic_id.as_str())))
            })
            .collect();

        assert_eq!(jobs.len(), 1, "scheduler should create one idempotent job for the retained topic only");
        assert!(
            jobs[0]
                .idempotency_key
                .as_deref()
                .is_some_and(|key| key.starts_with(&format!("TR:{}:", retained_topic_id.as_str()))),
            "topic retention job should use the TR:<topic_id>:<hour> idempotency format"
        );
    }
}
