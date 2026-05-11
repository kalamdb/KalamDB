//! Topic Retention Job Executor
//!
//! **Phase 8**: Background job for cleaning up expired topic messages
//!
//! Handles retention policy enforcement for topic messages based on:
//! - Time-based retention (retention_seconds)
//! - Size-based retention (retention_max_bytes)
//!
//! ## Responsibilities
//! - Scan topic message store for expired messages
//! - Delete messages older than retention_seconds
//! - Track cleanup metrics (messages deleted, bytes freed)
//! - Respect per-topic retention policies
//!
//! ## Parameters Format
//! ```json
//! {
//!   "topic_id": "topic_abc123",
//!   "partition_id": null,
//!   "batch_size": 10000
//! }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use kalamdb_commons::models::TopicId;
use kalamdb_core::{app_context::AppContext, error::KalamDbError};
use kalamdb_system::JobType;
use serde::{Deserialize, Serialize};

use crate::executors::{JobContext, JobDecision, JobExecutor, JobParams};

fn default_batch_size() -> usize {
    10000
}

/// Typed parameters for topic retention operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicRetentionParams {
    /// Topic identifier (required)
    pub topic_id: TopicId,
    /// Optional partition ID to clean. None means all partitions.
    #[serde(default)]
    pub partition_id: Option<u32>,
    /// Maximum messages to delete per partition.
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

impl JobParams for TopicRetentionParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        if self.batch_size == 0 {
            return Err(KalamDbError::InvalidOperation(
                "batch_size must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}

/// Topic Retention Job Executor
///
/// Executes retention policy enforcement for topic messages.
pub struct TopicRetentionExecutor;

impl TopicRetentionExecutor {
    /// Create a new TopicRetentionExecutor
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl JobExecutor for TopicRetentionExecutor {
    type Params = TopicRetentionParams;

    fn job_type(&self) -> JobType {
        JobType::TopicRetention
    }

    fn name(&self) -> &'static str {
        "TopicRetentionExecutor"
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        ctx.log_info("Starting topic retention enforcement");

        let params = ctx.params();
        let topic_id = &params.topic_id;
        let topic =
            match ctx.app_ctx.system_tables().topics().get_topic_by_id_async(topic_id).await? {
                Some(topic) => topic,
                None => {
                    return Ok(JobDecision::Skipped {
                        message: format!("Topic {} no longer exists", topic_id.as_str()),
                    });
                },
            };

        let cutoff_time = topic.retention_seconds.filter(|seconds| *seconds > 0).map(|seconds| {
            chrono::Utc::now()
                .timestamp_millis()
                .saturating_sub(seconds.saturating_mul(1000))
        });
        let max_bytes = topic.retention_max_bytes.filter(|bytes| *bytes > 0);

        if cutoff_time.is_none() && max_bytes.is_none() {
            return Ok(JobDecision::Skipped {
                message: format!("Topic {} has retention disabled", topic_id.as_str()),
            });
        }

        let partitions: Vec<u32> = match params.partition_id {
            Some(partition_id) => {
                if partition_id >= topic.partitions {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "partition_id {} is outside topic {} partition count {}",
                        partition_id,
                        topic_id.as_str(),
                        topic.partitions
                    )));
                }
                vec![partition_id]
            },
            None => (0..topic.partitions).collect(),
        };

        let topic_publisher = ctx.app_ctx.topic_publisher();
        let mut messages_deleted = 0usize;
        let mut bytes_freed = 0u64;

        for partition_id in partitions {
            let stats = topic_publisher
                .enforce_retention(&topic, partition_id, cutoff_time, max_bytes, params.batch_size)
                .map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to enforce retention for topic {} partition {}: {}",
                        topic_id.as_str(),
                        partition_id,
                        e
                    ))
                })?;
            messages_deleted += stats.messages_deleted;
            bytes_freed += stats.bytes_freed;
        }

        Ok(JobDecision::Completed {
            message: Some(format!(
                "Enforced retention policy for topic {} - {} messages deleted, {} bytes freed",
                topic_id.as_str(),
                messages_deleted,
                bytes_freed
            )),
        })
    }

    async fn pre_validate(
        &self,
        app_ctx: &Arc<AppContext>,
        params: &Self::Params,
    ) -> Result<bool, KalamDbError> {
        params.validate()?;

        let Some(topic) =
            app_ctx.system_tables().topics().get_topic_by_id_async(&params.topic_id).await?
        else {
            return Ok(false);
        };

        if params.partition_id.is_some_and(|partition_id| partition_id >= topic.partitions) {
            return Err(KalamDbError::InvalidOperation(format!(
                "partition_id is outside topic {} partition count {}",
                params.topic_id.as_str(),
                topic.partitions
            )));
        }

        Ok(topic.retention_seconds.is_some() || topic.retention_max_bytes.is_some())
    }

    async fn cancel(&self, ctx: &JobContext<Self::Params>) -> Result<(), KalamDbError> {
        ctx.log_warn("Topic retention job cancellation requested");
        // Allow cancellation since partial cleanup is acceptable
        Ok(())
    }
}

impl Default for TopicRetentionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use datafusion::scalar::ScalarValue;
    use kalamdb_commons::{models::{rows::Row, NamespaceId, PayloadMode, TableId, TableName, TopicOp}, JobId, NodeId};
    use kalamdb_core::test_helpers::test_app_context_simple;
    use kalamdb_system::{providers::{jobs::models::Job, topics::{models::Topic, TopicRoute}}, JobStatus};

    use super::*;

    fn create_test_row(id: i32, payload: &str) -> Row {
        let mut values = BTreeMap::new();
        values.insert("id".to_string(), ScalarValue::Int32(Some(id)));
        values.insert("payload".to_string(), ScalarValue::Utf8(Some(payload.to_string())));
        Row { values }
    }

    fn make_job(id: &str, job_type: JobType) -> Job {
        let now = chrono::Utc::now().timestamp_millis();
        Job {
            job_id: JobId::new(id),
            job_type,
            status: JobStatus::Running,
            leader_status: None,
            parameters: None,
            message: None,
            exception_trace: None,
            idempotency_key: None,
            retry_count: 0,
            max_retries: 3,
            memory_used: None,
            cpu_used: None,
            created_at: now,
            updated_at: now,
            started_at: Some(now),
            finished_at: None,
            node_id: NodeId::from(1u64),
            leader_node_id: None,
            queue: None,
            priority: None,
        }
    }

    async fn setup_topic_with_route(
        app_ctx: &Arc<AppContext>,
        topic_name: &str,
        retention_seconds: Option<i64>,
        retention_max_bytes: Option<i64>,
    ) -> (Topic, TableId) {
        let namespace = NamespaceId::new("topic_retention_jobs");
        let table_id = TableId::new(namespace, TableName::new("events"));
        let mut topic = Topic::new(TopicId::new(topic_name), topic_name.to_string());
        topic.partitions = 1;
        topic.retention_seconds = retention_seconds;
        topic.retention_max_bytes = retention_max_bytes;
        topic.routes.push(TopicRoute {
            table_id: table_id.clone(),
            op: TopicOp::Insert,
            payload_mode: PayloadMode::Full,
            filter_expr: None,
            partition_key_expr: None,
        });

        app_ctx.system_tables().topics().create_topic_async(topic.clone()).await.unwrap();
        app_ctx.topic_publisher().add_topic(topic.clone());
        (topic, table_id)
    }

    #[test]
    fn test_topic_retention_params_validation() {
        // Valid params
        let valid_params = TopicRetentionParams {
            topic_id: TopicId::new("topic_123"),
            partition_id: None,
            batch_size: 1000,
        };
        assert!(valid_params.validate().is_ok());

        // Invalid: zero batch size
        let invalid_params = TopicRetentionParams {
            topic_id: TopicId::new("topic_123"),
            partition_id: None,
            batch_size: 0,
        };
        assert!(invalid_params.validate().is_err());
    }

    #[test]
    fn test_topic_retention_params_serialization() {
        let params = TopicRetentionParams {
            topic_id: TopicId::new("topic_abc"),
            partition_id: Some(0),
            batch_size: 100,
        };

        // Test JSON round-trip
        let json = serde_json::to_string(&params).unwrap();
        let deserialized: TopicRetentionParams = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.topic_id.as_str(), "topic_abc");
        assert_eq!(deserialized.partition_id, Some(0));
        assert_eq!(deserialized.batch_size, 100);
    }

    #[tokio::test]
    async fn test_pre_validate_skips_missing_topic() {
        let app_ctx = test_app_context_simple();
        let executor = TopicRetentionExecutor::new();
        let params = TopicRetentionParams {
            topic_id: TopicId::new("missing.topic"),
            partition_id: None,
            batch_size: 100,
        };

        let should_run = executor.pre_validate(&app_ctx, &params).await.unwrap();
        assert!(!should_run);
    }

    #[tokio::test]
    async fn test_pre_validate_skips_topic_with_retention_disabled() {
        let app_ctx = test_app_context_simple();
        let executor = TopicRetentionExecutor::new();
        let (topic, _) = setup_topic_with_route(&app_ctx, "disabled.topic", None, None).await;
        let params = TopicRetentionParams {
            topic_id: topic.topic_id.clone(),
            partition_id: None,
            batch_size: 100,
        };

        let should_run = executor.pre_validate(&app_ctx, &params).await.unwrap();
        assert!(!should_run);
    }

    #[tokio::test]
    async fn test_execute_enforces_byte_retention_and_reports_progress() {
        let app_ctx = test_app_context_simple();
        let executor = TopicRetentionExecutor::new();
        let (topic, table_id) = setup_topic_with_route(&app_ctx, "retained.topic", None, Some(1)).await;

        for idx in 0..3 {
            let row = create_test_row(idx, &format!("payload_{}", idx));
            app_ctx.topic_publisher().publish_message(&table_id, TopicOp::Insert, &row, None).unwrap();
        }

        let params = TopicRetentionParams {
            topic_id: topic.topic_id.clone(),
            partition_id: Some(0),
            batch_size: 100,
        };
        let ctx = JobContext::new(
            app_ctx.clone(),
            make_job("TR-exec", JobType::TopicRetention).job_id.as_str().to_string(),
            params,
        );

        let decision = executor.execute(&ctx).await.unwrap();

        match decision {
            JobDecision::Completed { message } => {
                let message = message.unwrap_or_default();
                assert!(message.contains("messages deleted"));
                assert!(message.contains("bytes freed"));
            },
            other => panic!("expected completed decision, got {:?}", other),
        }

        assert_eq!(app_ctx.topic_publisher().earliest_available_offset(&topic.topic_id, 0).unwrap(), 3);
        assert_eq!(app_ctx.topic_publisher().latest_offset(&topic.topic_id, 0).unwrap(), Some(2));
        assert!(app_ctx.topic_publisher().fetch_messages(&topic.topic_id, 0, 3, 10).unwrap().is_empty());

        let err = app_ctx.topic_publisher().fetch_messages(&topic.topic_id, 0, 0, 10).unwrap_err();
        assert!(err.to_string().contains("OffsetOutOfRange"));
    }
}
