use std::sync::Arc;

use kalamdb_commons::models::TopicId;
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::DropTopicStatement;

use super::cleanup::clear_topic_data;

pub struct DropTopicHandler {
    app_context: Arc<AppContext>,
}

impl DropTopicHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<DropTopicStatement> for DropTopicHandler {
    async fn execute(
        &self,
        statement: DropTopicStatement,
        _params: Vec<ScalarValue>,
        _context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let topic_id = TopicId::new(&statement.topic_name);
        let topics_provider = self.app_context.system_tables().topics();
        let topic = topics_provider.get_topic_by_id_async(&topic_id).await?;

        if topic.is_none() {
            return Err(KalamDbError::NotFound(format!(
                "Topic '{}' does not exist",
                statement.topic_name
            )));
        }

        let topic_name = topic.expect("checked is_some").name;

        let (offsets_deleted, messages_deleted) =
            clear_topic_data(&self.app_context, &topic_id).map_err(|e| {
                KalamDbError::ExecutionError(format!(
                    "Failed to clean up dropped topic '{}' ({}): {}",
                    topic_name,
                    topic_id.as_str(),
                    e
                ))
            })?;

        topics_provider.delete_topic_async(&topic_id).await?;
        self.app_context.topic_publisher().remove_topic(&topic_id);

        log::info!(
            "Dropped topic '{}' - {} consumer group offsets deleted, {} messages deleted",
            topic_name,
            offsets_deleted,
            messages_deleted
        );

        Ok(ExecutionResult::Success {
            message: format!(
                "Dropped topic '{}' - {} consumer group offsets deleted, {} messages deleted",
                topic_name,
                offsets_deleted,
                messages_deleted
            ),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &DropTopicStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        use kalamdb_commons::Role;

        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "DROP TOPIC requires DBA or System role".to_string(),
            )),
        }
    }
}
