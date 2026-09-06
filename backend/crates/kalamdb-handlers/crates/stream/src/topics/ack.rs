use std::sync::Arc;

use kalamdb_commons::models::ConsumerGroupId;
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::AckStatement;

use super::name_resolution::{resolve_topic_id, resolve_topic_name};
use crate::result_rows;

pub struct AckHandler {
    app_context: Arc<AppContext>,
}

impl AckHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<AckStatement> for AckHandler {
    async fn execute(
        &self,
        statement: AckStatement,
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
            .ack_offset(&topic_id, &group_id, statement.partition_id, statement.upto_offset)
            .map_err(|e| {
                KalamDbError::InvalidOperation(format!("Failed to commit offset: {}", e))
            })?;

        result_rows::ack_result(
            &resolved_topic_name,
            &statement.group_id,
            statement.partition_id,
            statement.upto_offset,
        )
    }

    async fn check_authorization(
        &self,
        _statement: &AckStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        use kalamdb_commons::Role;

        match context.user_role() {
            Role::Service | Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "Only service, dba, or system roles can acknowledge topic offsets".to_string(),
            )),
        }
    }
}
