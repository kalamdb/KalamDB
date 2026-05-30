use std::sync::Arc;

use kalamdb_commons::{models::TopicId, Role};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_handlers_support::audit;
use kalamdb_sql::ddl::{AlterTopicRetentionStatement, ClearTopicRetentionStatement};
use kalamdb_system::providers::topics::models::Topic;

use super::name_resolution::{resolve_topic_id, resolve_topic_name};

pub struct AlterTopicRetentionHandler {
    app_context: Arc<AppContext>,
}

impl AlterTopicRetentionHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

pub struct ClearTopicRetentionHandler {
    app_context: Arc<AppContext>,
}

impl ClearTopicRetentionHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

fn validate_retention_value(value: Option<i64>, option_name: &str) -> Result<(), KalamDbError> {
    if matches!(value, Some(v) if v <= 0) {
        return Err(KalamDbError::InvalidOperation(format!(
            "{} must be greater than 0 or NULL",
            option_name
        )));
    }
    Ok(())
}

async fn load_topic(
    app_context: &Arc<AppContext>,
    topic_id: &TopicId,
) -> Result<Topic, KalamDbError> {
    app_context
        .system_tables()
        .topics()
        .get_topic_by_id_async(topic_id)
        .await?
        .ok_or_else(|| KalamDbError::NotFound(format!("Topic '{}' does not exist", topic_id)))
}

async fn persist_topic_retention_change(
    app_context: &Arc<AppContext>,
    mut topic: Topic,
    retention_seconds: Option<i64>,
    retention_max_bytes: Option<i64>,
) -> Result<Topic, KalamDbError> {
    validate_retention_value(retention_seconds, "retention_seconds")?;
    validate_retention_value(retention_max_bytes, "retention_max_bytes")?;

    topic.retention_seconds = retention_seconds;
    topic.retention_max_bytes = retention_max_bytes;
    topic.updated_at = chrono::Utc::now().timestamp_millis();

    app_context.system_tables().topics().update_topic_async(topic.clone()).await?;
    app_context.topic_publisher().update_topic(topic.clone());

    Ok(topic)
}

fn retention_summary(topic: &Topic) -> String {
    format!(
        "retention_seconds={:?}, retention_max_bytes={:?}",
        topic.retention_seconds, topic.retention_max_bytes
    )
}

impl TypedStatementHandler<AlterTopicRetentionStatement> for AlterTopicRetentionHandler {
    async fn execute(
        &self,
        statement: AlterTopicRetentionStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        if statement.retention_seconds.is_none() && statement.retention_max_bytes.is_none() {
            return Err(KalamDbError::InvalidOperation(
                "ALTER TOPIC SET RETENTION requires at least one retention option".to_string(),
            ));
        }

        let resolved_topic_name = resolve_topic_name(&statement.topic_name, context);
        let topic_id = resolve_topic_id(&statement.topic_name, context);
        let topic = load_topic(&self.app_context, &topic_id).await?;
        let retention_seconds = statement.retention_seconds.unwrap_or(topic.retention_seconds);
        let retention_max_bytes =
            statement.retention_max_bytes.unwrap_or(topic.retention_max_bytes);
        let updated_topic = persist_topic_retention_change(
            &self.app_context,
            topic,
            retention_seconds,
            retention_max_bytes,
        )
        .await?;

        let audit_entry = audit::log_ddl_operation(
            context,
            "ALTER",
            "TOPIC",
            &resolved_topic_name,
            Some(retention_summary(&updated_topic)),
            None,
        );
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

        Ok(ExecutionResult::Success {
            message: format!(
                "Updated retention for topic '{}': {}",
                resolved_topic_name,
                retention_summary(&updated_topic)
            ),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &AlterTopicRetentionStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "ALTER TOPIC requires DBA or System role".to_string(),
            )),
        }
    }
}

impl TypedStatementHandler<ClearTopicRetentionStatement> for ClearTopicRetentionHandler {
    async fn execute(
        &self,
        statement: ClearTopicRetentionStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let resolved_topic_name = resolve_topic_name(&statement.topic_name, context);
        let topic_id = resolve_topic_id(&statement.topic_name, context);
        let topic = load_topic(&self.app_context, &topic_id).await?;
        let updated_topic =
            persist_topic_retention_change(&self.app_context, topic, None, None).await?;

        let audit_entry = audit::log_ddl_operation(
            context,
            "ALTER",
            "TOPIC",
            &resolved_topic_name,
            Some(retention_summary(&updated_topic)),
            None,
        );
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

        Ok(ExecutionResult::Success {
            message: format!("Cleared retention for topic '{}'", resolved_topic_name),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &ClearTopicRetentionStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "ALTER TOPIC requires DBA or System role".to_string(),
            )),
        }
    }
}
