//! Typed handler for SHOW EXPORT statement

use std::sync::Arc;

use arrow::{
    array::{RecordBatch, StringArray, TimestampMicrosecondArray},
    datatypes::{DataType, Field, Schema, TimeUnit},
};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_jobs::AppContextJobsExt;
use kalamdb_sql::ddl::ShowExportStatement;
use kalamdb_system::{
    providers::jobs::models::{Job, JobFilter, JobSortField, SortOrder},
    JobType,
};

/// Handler for SHOW EXPORT
///
/// Lists the user's export jobs with status and download link (when complete).
pub struct ShowExportHandler {
    app_context: Arc<AppContext>,
}

impl ShowExportHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    fn result_schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("job_id", DataType::Utf8, false),
            Field::new("status", DataType::Utf8, false),
            Field::new("created_at", DataType::Timestamp(TimeUnit::Microsecond, None), false),
            Field::new("message", DataType::Utf8, true),
            Field::new("download_url", DataType::Utf8, true),
        ]))
    }

    fn created_at_micros(created_at_millis: i64) -> i64 {
        created_at_millis.saturating_mul(1_000)
    }

    fn extract_parameter(job: &Job, key: &str) -> Option<String> {
        let parameters = job.parameters.as_ref()?;
        match parameters {
            serde_json::Value::Object(_) => parameters
                .get(key)
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned),
            serde_json::Value::String(raw_json) => serde_json::from_str::<serde_json::Value>(
                raw_json,
            )
            .ok()?
            .get(key)
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
            _ => None,
        }
    }

    /// Build a download URI for a completed export.
    fn build_download_url(user_id: &str, export_id: &str) -> String {
        format!("/v1/exports/{}/{}", user_id, export_id)
    }

    /// Extract export_id from job parameters JSON
    fn extract_export_id(job: &Job) -> Option<String> {
        Self::extract_parameter(job, "export_id")
    }
}

impl TypedStatementHandler<ShowExportStatement> for ShowExportHandler {
    async fn execute(
        &self,
        _statement: ShowExportStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let user_id = context.user_id().to_string();

        // Query jobs for this user's export jobs
        let job_manager = self.app_context.job_manager();
        let filter = JobFilter {
            job_type: Some(JobType::UserExport),
            limit: Some(20),
            sort_by: Some(JobSortField::CreatedAt),
            sort_order: Some(SortOrder::Desc),
            ..Default::default()
        };

        let all_jobs = job_manager.list_jobs(filter).await?;

        // Filter to only this user's exports (check parameters JSON)
        let user_jobs: Vec<&Job> = all_jobs
            .iter()
            .filter(|job| Self::extract_parameter(job, "user_id").as_deref() == Some(&user_id))
            .collect();

        // Build result schema
        let schema = Self::result_schema();

        if user_jobs.is_empty() {
            let batch = RecordBatch::new_empty(schema.clone());
            return Ok(ExecutionResult::Rows {
                batches: vec![batch],
                row_count: 0,
                schema: Some(schema),
            });
        }

        let mut job_ids = Vec::new();
        let mut statuses = Vec::new();
        let mut created_ats = Vec::new();
        let mut messages = Vec::new();
        let mut download_urls = Vec::new();

        for job in &user_jobs {
            job_ids.push(job.job_id.as_str().to_string());
            statuses.push(format!("{}", job.status));
            created_ats.push(Self::created_at_micros(job.created_at));
            messages.push(job.message.clone().unwrap_or_default());

            // Build download URL only for completed jobs
            let url = if job.status == kalamdb_system::JobStatus::Completed {
                Self::extract_export_id(job)
                    .map(|eid| Self::build_download_url(&user_id, &eid))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            download_urls.push(url);
        }

        let row_count = user_jobs.len();

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(job_ids)),
                Arc::new(StringArray::from(statuses)),
                Arc::new(TimestampMicrosecondArray::from(created_ats)),
                Arc::new(StringArray::from(messages)),
                Arc::new(StringArray::from(download_urls)),
            ],
        )
        .map_err(|e| {
            KalamDbError::InvalidOperation(format!("Failed to build export results: {}", e))
        })?;

        Ok(ExecutionResult::Rows {
            batches: vec![batch],
            row_count,
            schema: Some(schema),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &ShowExportStatement,
        _context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        // Any authenticated user can view their own exports
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::StringArray;
    use kalamdb_commons::{JobId, NodeId};
    use kalamdb_core::sql::context::ExecutionContext;
    use kalamdb_jobs::init_job_manager;
    use kalamdb_sql::ddl::ShowExportStatement;
    use kalamdb_system::{providers::jobs::models::Job, JobStatus, JobType};
    use serde_json::json;

    use super::*;

    #[test]
    fn show_export_schema_uses_timestamp_for_created_at() {
        let schema = ShowExportHandler::result_schema();
        let field = schema.field_with_name("created_at").expect("created_at field");

        assert!(matches!(field.data_type(), DataType::Timestamp(TimeUnit::Microsecond, None)));
    }

    #[test]
    fn show_export_created_at_converts_millis_to_micros() {
        assert_eq!(ShowExportHandler::created_at_micros(1_741_900_245_123), 1_741_900_245_123_000);
    }

    #[test]
    fn show_export_download_url_is_relative_uri() {
        let url = ShowExportHandler::build_download_url("alice", "export-123");

        assert_eq!(url, "/v1/exports/alice/export-123");
    }

    #[tokio::test]
    async fn show_export_returns_completed_job_for_current_user() {
        use kalamdb_commons::models::{Role, UserId};
        use kalamdb_core::app_context::AppContext;

        let app_context = AppContext::new_test();
        init_job_manager(&app_context);

        let now_ms = chrono::Utc::now().timestamp_millis();
        app_context
            .system_tables()
            .jobs()
            .create_job(Job {
                job_id: JobId::new("UE-test-show-export"),
                job_type: JobType::UserExport,
                status: JobStatus::Completed,
                leader_status: None,
                parameters: Some(json!({
                    "user_id": "alice",
                    "export_id": "export-alice-1"
                })),
                message: Some("done".to_string()),
                exception_trace: None,
                idempotency_key: None,
                retry_count: 0,
                max_retries: 3,
                memory_used: None,
                cpu_used: None,
                created_at: now_ms,
                updated_at: now_ms,
                started_at: Some(now_ms),
                finished_at: Some(now_ms),
                node_id: NodeId::from(1u64),
                leader_node_id: None,
                queue: None,
                priority: None,
            })
            .expect("insert export job");

        let handler = ShowExportHandler::new(Arc::clone(&app_context));
        let exec_ctx = ExecutionContext::new(
            UserId::from("alice"),
            Role::User,
            Arc::new(app_context.session_factory().create_session()),
        );

        let result = handler.execute(ShowExportStatement, vec![], &exec_ctx).await;
        assert!(result.is_ok(), "SHOW EXPORT should succeed: {result:?}");

        let ExecutionResult::Rows {
            batches, row_count, ..
        } = result.expect("SHOW EXPORT result")
        else {
            panic!("Expected SHOW EXPORT to return rows");
        };

        assert_eq!(row_count, 1);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_rows(), 1);

        let job_ids = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("job_id column should be Utf8");
        assert_eq!(job_ids.value(0), "UE-test-show-export");
    }

    #[tokio::test]
    async fn show_export_accepts_stringified_job_parameters() {
        use kalamdb_commons::models::{Role, UserId};
        use kalamdb_core::app_context::AppContext;

        let app_context = AppContext::new_test();
        init_job_manager(&app_context);

        let now_ms = chrono::Utc::now().timestamp_millis();
        app_context
            .system_tables()
            .jobs()
            .create_job(Job {
                job_id: JobId::new("UE-test-show-export-stringified"),
                job_type: JobType::UserExport,
                status: JobStatus::Completed,
                leader_status: None,
                parameters: Some(serde_json::Value::String(
                    r#"{"user_id":"alice","export_id":"export-alice-2"}"#.to_string(),
                )),
                message: Some("done".to_string()),
                exception_trace: None,
                idempotency_key: None,
                retry_count: 0,
                max_retries: 3,
                memory_used: None,
                cpu_used: None,
                created_at: now_ms,
                updated_at: now_ms,
                started_at: Some(now_ms),
                finished_at: Some(now_ms),
                node_id: NodeId::from(1u64),
                leader_node_id: None,
                queue: None,
                priority: None,
            })
            .expect("insert export job");

        let handler = ShowExportHandler::new(Arc::clone(&app_context));
        let exec_ctx = ExecutionContext::new(
            UserId::from("alice"),
            Role::User,
            Arc::new(app_context.session_factory().create_session()),
        );

        let result = handler.execute(ShowExportStatement, vec![], &exec_ctx).await;
        assert!(result.is_ok(), "SHOW EXPORT should succeed: {result:?}");

        let ExecutionResult::Rows {
            batches, row_count, ..
        } = result.expect("SHOW EXPORT result")
        else {
            panic!("Expected SHOW EXPORT to return rows");
        };

        assert_eq!(row_count, 1);
        assert_eq!(batches[0].num_rows(), 1);

        let download_urls = batches[0]
            .column(4)
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("download_url column should be Utf8");
        assert_eq!(download_urls.value(0), "/v1/exports/alice/export-alice-2");
    }
}
