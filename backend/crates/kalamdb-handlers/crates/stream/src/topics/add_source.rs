use std::sync::Arc;

use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::AddTopicSourceStatement;
use kalamdb_system::providers::topics::models::TopicRoute;

use super::name_resolution::{resolve_topic_id, resolve_topic_name};

pub struct AddTopicSourceHandler {
    app_context: Arc<AppContext>,
}

impl AddTopicSourceHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    fn namespace_from_topic_name(topic_name: &str) -> Option<String> {
        topic_name.split_once('.').map(|(namespace, _)| namespace.to_string())
    }

    fn route_matches_statement(existing: &TopicRoute, route: &TopicRoute, qualified: bool) -> bool {
        if existing.op != route.op {
            return false;
        }

        if qualified {
            existing.table_id == route.table_id
        } else {
            // Unqualified source names should be idempotent by table local-name + op.
            // This keeps stale cross-namespace route drift from failing reruns.
            existing.table_id.table_name() == route.table_id.table_name()
        }
    }
}

impl TypedStatementHandler<AddTopicSourceStatement> for AddTopicSourceHandler {
    async fn execute(
        &self,
        statement: AddTopicSourceStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let resolved_topic_name = resolve_topic_name(&statement.topic_name, context);
        let topic_id = resolve_topic_id(&statement.topic_name, context);
        let topics_provider = self.app_context.system_tables().topics();

        let mut topic =
            topics_provider.get_topic_by_id_async(&topic_id).await?.ok_or_else(|| {
                KalamDbError::NotFound(format!("Topic '{}' does not exist", resolved_topic_name))
            })?;

        let resolved_table_id = if statement.table_name_qualified {
            statement.table_id.clone()
        } else if let Some(topic_namespace) = Self::namespace_from_topic_name(&topic.name) {
            kalamdb_commons::models::TableId::from_strings(
                &topic_namespace,
                statement.table_id.table_name().as_str(),
            )
        } else {
            // Defensive fallback for malformed topic names.
            kalamdb_commons::models::TableId::from_strings(
                context.default_namespace().as_str(),
                statement.table_id.table_name().as_str(),
            )
        };

        let route = TopicRoute {
            table_id: resolved_table_id.clone(),
            op: statement.operation,
            payload_mode: statement.payload_mode,
            filter_expr: statement.filter_expr.clone(),
            partition_key_expr: None,
        };

        let duplicate = topic
            .routes
            .iter()
            .any(|existing| Self::route_matches_statement(existing, &route, statement.table_name_qualified));
        if duplicate {
            return Ok(ExecutionResult::Success {
                message: format!(
                    "Route for {}.{} ON {:?} already exists in topic '{}'",
                    route.table_id.namespace_id(),
                    route.table_id.table_name(),
                    route.op,
                    resolved_topic_name
                ),
            });
        }

        topic.routes.push(route);
        topic.updated_at = chrono::Utc::now().timestamp_millis();
        topics_provider.update_topic_async(topic.clone()).await?;
        self.app_context.topic_publisher().update_topic(topic);

        Ok(ExecutionResult::Success {
            message: format!(
                "Added source {}.{} ON {:?} to topic '{}'",
                resolved_table_id.namespace_id(),
                resolved_table_id.table_name(),
                statement.operation,
                resolved_topic_name
            ),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &AddTopicSourceStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        use kalamdb_commons::Role;

        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "ALTER TOPIC requires DBA or System role".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::{PayloadMode, TableId, TopicOp};
    use kalamdb_system::providers::topics::models::TopicRoute;

    use super::AddTopicSourceHandler;

    #[test]
    fn route_matches_statement_unqualified_ignores_namespace() {
        let existing = TopicRoute {
            table_id: TableId::from_strings("default", "messages"),
            op: TopicOp::Insert,
            payload_mode: PayloadMode::Full,
            filter_expr: None,
            partition_key_expr: None,
        };
        let desired = TopicRoute {
            table_id: TableId::from_strings("sql_file_case_01", "messages"),
            op: TopicOp::Insert,
            payload_mode: PayloadMode::Full,
            filter_expr: None,
            partition_key_expr: None,
        };

        assert!(AddTopicSourceHandler::route_matches_statement(&existing, &desired, false));
        assert!(!AddTopicSourceHandler::route_matches_statement(&existing, &desired, true));
    }

    #[test]
    fn namespace_from_topic_name_extracts_prefix() {
        assert_eq!(
            AddTopicSourceHandler::namespace_from_topic_name("app.topic"),
            Some("app".to_string())
        );
        assert_eq!(AddTopicSourceHandler::namespace_from_topic_name("topic"), None);
    }
}
