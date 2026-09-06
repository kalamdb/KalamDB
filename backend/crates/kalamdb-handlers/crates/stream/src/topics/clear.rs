use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::ClearTopicStatement;

use super::{cleanup::clear_topic_data, name_resolution::resolve_topic_id};

pub struct ClearTopicHandler {
    app_context: Arc<AppContext>,
}

impl ClearTopicHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<ClearTopicStatement> for ClearTopicHandler {
    async fn execute(
        &self,
        statement: ClearTopicStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let resolved_topic_id = resolve_topic_id(statement.topic_id.as_str(), context);
        let topic_id = &resolved_topic_id;
        let topics_provider = self.app_context.system_tables().topics();
        let topic = topics_provider.get_topic_by_id_async(topic_id).await?;

        if topic.is_none() {
            return Err(KalamDbError::NotFound(format!(
                "Topic '{}' does not exist",
                topic_id.as_str()
            )));
        }

        let topic_name = topic.expect("checked is_some").name;

        let (offsets_deleted, messages_deleted) = clear_topic_data(&self.app_context, topic_id)
            .map_err(|e| {
                KalamDbError::ExecutionError(format!(
                    "Failed to clear topic '{}' ({}): {}",
                    topic_name,
                    topic_id.as_str(),
                    e
                ))
            })?;

        log::info!(
            "Cleared topic '{}' ({}) - {} consumer group offsets deleted, {} messages deleted",
            topic_name,
            topic_id.as_str(),
            offsets_deleted,
            messages_deleted
        );

        Ok(ExecutionResult::Success {
            message: format!(
                "Cleared topic '{}' - {} consumer group offsets deleted, {} messages deleted",
                topic_name, offsets_deleted, messages_deleted
            ),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &ClearTopicStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        use kalamdb_commons::Role;

        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "CLEAR TOPIC requires DBA or System role".to_string(),
            )),
        }
    }
}
