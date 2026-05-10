use std::sync::Arc;

use kalamdb_commons::{
    models::{ConsumerGroupId, TopicId},
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
use kalamdb_sql::ddl::ResetConsumerGroupStatement;

use crate::result_rows;

pub struct ResetConsumerGroupHandler {
    app_context: Arc<AppContext>,
}

impl ResetConsumerGroupHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<ResetConsumerGroupStatement> for ResetConsumerGroupHandler {
    async fn execute(
        &self,
        statement: ResetConsumerGroupStatement,
        _params: Vec<ScalarValue>,
        _context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let topic_id = TopicId::new(&statement.topic_name);
        let group_id = ConsumerGroupId::new(&statement.group_id);

        let topics_provider = self.app_context.system_tables().topics();
        let _topic = topics_provider.get_topic_by_id_async(&topic_id).await?.ok_or_else(|| {
            KalamDbError::NotFound(format!("Topic '{}' does not exist", statement.topic_name))
        })?;

        self.app_context
            .topic_publisher()
            .reset_group_offset(&topic_id, &group_id, statement.partition_id, statement.next_offset)
            .map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to reset consumer group: {}", e))
            })?;

        result_rows::reset_consumer_group_result(
            &statement.topic_name,
            &statement.group_id,
            statement.partition_id,
            statement.next_offset,
        )
    }

    async fn check_authorization(
        &self,
        _statement: &ResetConsumerGroupStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "Only dba or system roles can reset consumer group offsets".to_string(),
            )),
        }
    }
}
