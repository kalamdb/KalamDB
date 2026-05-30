use std::sync::Arc;

use kalamdb_commons::{
    models::{NamespaceId, TopicId},
    Role,
};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::CreateTopicStatement;
use kalamdb_system::providers::topics::models::Topic;

use super::name_resolution::resolve_topic_name;

pub struct CreateTopicHandler {
    app_context: Arc<AppContext>,
}

impl CreateTopicHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    fn extract_namespace_id(topic_name: &str) -> Result<NamespaceId, KalamDbError> {
        let (namespace, topic_local_name) = topic_name.split_once('.').ok_or_else(|| {
            KalamDbError::InvalidOperation(
                "Topic name must be namespace-qualified: <namespace>.<topic>".to_string(),
            )
        })?;

        if namespace.is_empty() || topic_local_name.is_empty() {
            return Err(KalamDbError::InvalidOperation(
                "Topic name must be namespace-qualified: <namespace>.<topic>".to_string(),
            ));
        }

        Ok(NamespaceId::new(namespace))
    }

    fn resolve_retention_value(
        sql_value: Option<Option<i64>>,
        default_value: i64,
        option_name: &str,
    ) -> Result<Option<i64>, KalamDbError> {
        let value = sql_value.unwrap_or(Some(default_value));
        if matches!(value, Some(v) if v <= 0) {
            return Err(KalamDbError::InvalidOperation(format!(
                "{} must be greater than 0 or NULL",
                option_name
            )));
        }
        Ok(value)
    }
}

impl TypedStatementHandler<CreateTopicStatement> for CreateTopicHandler {
    async fn execute(
        &self,
        statement: CreateTopicStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let topic_name = resolve_topic_name(&statement.topic_name, context);
        let namespace_id = Self::extract_namespace_id(&topic_name)?;
        let namespaces_provider = self.app_context.system_tables().namespaces();
        if namespaces_provider.get_namespace_async(&namespace_id).await?.is_none() {
            return Err(KalamDbError::NotFound(format!(
                "Namespace '{}' does not exist",
                namespace_id
            )));
        }

        let topic_id = TopicId::new(&topic_name);
        let topics_provider = self.app_context.system_tables().topics();
        if topics_provider.get_topic_by_id_async(&topic_id).await?.is_some() {
            if statement.if_not_exists {
                return Ok(ExecutionResult::Success {
                    message: format!("Topic {} already exists (IF NOT EXISTS)", topic_name),
                });
            }
            return Err(KalamDbError::AlreadyExists(format!(
                "Topic '{}' already exists",
                topic_name
            )));
        }

        let mut topic = Topic::new(topic_id.clone(), topic_name.clone());
        topic.partitions = statement.partitions.unwrap_or(1);
        if topic.partitions == 0 {
            return Err(KalamDbError::InvalidOperation(
                "Topic partitions must be greater than 0".to_string(),
            ));
        }
        let topic_config = &self.app_context.config().topics;
        topic.retention_seconds = Self::resolve_retention_value(
            statement.retention_seconds,
            topic_config.default_retention_seconds,
            "retention_seconds",
        )?;
        topic.retention_max_bytes = Self::resolve_retention_value(
            statement.retention_max_bytes,
            topic_config.default_retention_max_bytes,
            "retention_max_bytes",
        )?;

        topics_provider.create_topic_async(topic.clone()).await?;
        self.app_context.topic_publisher().add_topic(topic.clone());

        Ok(ExecutionResult::Success {
            message: format!(
                "Created topic '{}' with {} partition(s)",
                topic_name, topic.partitions
            ),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &CreateTopicStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "CREATE TOPIC requires DBA or System role".to_string(),
            )),
        }
    }
}
