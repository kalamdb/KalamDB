use std::sync::Arc;

use kalamdb_commons::{models::ConsumerGroupId, Role};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::ResetConsumerGroupStatement;

use super::name_resolution::{resolve_topic_id, resolve_topic_name};
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
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let resolved_topic_name = resolve_topic_name(&statement.topic_name, context);
        let topic_id = resolve_topic_id(&statement.topic_name, context);
        let group_id = ConsumerGroupId::new(&statement.group_id);

        let topics_provider = self.app_context.system_tables().topics();
        let _topic = topics_provider.get_topic_by_id_async(&topic_id).await?.ok_or_else(|| {
            KalamDbError::NotFound(format!("Topic '{}' does not exist", resolved_topic_name))
        })?;

        self.app_context
            .topic_publisher()
            .reset_group_offset(&topic_id, &group_id, statement.partition_id, statement.next_offset)
            .map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to reset consumer group: {}", e))
            })?;

        result_rows::reset_consumer_group_result(
            &resolved_topic_name,
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
