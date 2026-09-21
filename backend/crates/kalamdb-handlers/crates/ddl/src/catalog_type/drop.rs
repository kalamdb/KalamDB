//! Typed handler for DROP TYPE.

use std::sync::Arc;

use kalamdb_commons::models::CatalogTypeKind;
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::DropTypeStatement;

use crate::helpers::{async_blocking::run_blocking, guards::require_admin};

pub struct DropTypeHandler {
    app_context: Arc<AppContext>,
}

impl DropTypeHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<DropTypeStatement> for DropTypeHandler {
    async fn execute(
        &self,
        statement: DropTypeStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "drop type")?;
        if statement.cascade {
            return Err(KalamDbError::InvalidSql(
                "DROP TYPE CASCADE is not supported; drop dependents first or use RESTRICT"
                    .to_string(),
            ));
        }
        let app = Arc::clone(&self.app_context);
        run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            let existing = stores
                .find_type(&statement.namespace_id, &statement.name)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            let Some(catalog_type) = existing else {
                if statement.if_exists {
                    return Ok(ExecutionResult::Success {
                        message: format!(
                            "Type {}.{} does not exist, skipping",
                            statement.namespace_id, statement.name
                        ),
                    });
                }
                return Err(KalamDbError::NotFound(format!(
                    "type {}.{} not found",
                    statement.namespace_id, statement.name
                )));
            };
            match catalog_type.kind {
                CatalogTypeKind::ImplicitTableRow => {
                    return Err(KalamDbError::InvalidSql(format!(
                        "cannot drop implicit table row type {}; drop the table instead",
                        crate::catalog_type::type_alias(&catalog_type)
                    )));
                },
                CatalogTypeKind::TopicPayload => {
                    return Err(KalamDbError::InvalidSql(format!(
                        "cannot drop implicit topic payload type {}; drop the topic instead",
                        crate::catalog_type::type_alias(&catalog_type)
                    )));
                },
                CatalogTypeKind::RowAlias | CatalogTypeKind::Composite | CatalogTypeKind::Enum => {
                },
            }
            stores
                .drop_type(&catalog_type.type_id)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            Ok(ExecutionResult::Success {
                message: format!("Type {} dropped", crate::catalog_type::type_alias(&catalog_type)),
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{
        models::{NamespaceId, TopicId, UserId},
        Role,
    };
    use kalamdb_core::{
        sql::{context::ExecutionContext, executor::handlers::TypedStatementHandler},
        test_helpers::{create_test_session_for, test_app_context_simple},
    };
    use kalamdb_sql::ddl::DropTypeStatement;

    use super::*;

    fn dba_ctx(app: &Arc<kalamdb_core::app_context::AppContext>) -> ExecutionContext {
        ExecutionContext::new(UserId::new("root"), Role::Dba, create_test_session_for(app))
    }

    #[tokio::test]
    async fn drop_type_rejects_implicit_topic_payload() {
        let app = test_app_context_simple();
        let namespace_id = NamespaceId::new("app");
        if app.system_tables().namespaces().get_namespace(&namespace_id).unwrap().is_none() {
            app.system_tables()
                .namespaces()
                .create_namespace(kalamdb_system::Namespace::new("app"))
                .unwrap();
        }
        app.system_tables()
            .catalog_stores()
            .ensure_implicit_topic_payload_type(&TopicId::new("app.inbox"))
            .unwrap();
        let handler = DropTypeHandler::new(Arc::clone(&app));
        let statement =
            DropTypeStatement::parse("DROP TYPE app.inbox", &namespace_id).expect("parse");
        let error = handler.execute(statement, vec![], &dba_ctx(&app)).await.unwrap_err();
        let message = error.to_string();
        assert!(message.contains("topic payload"), "{message}");
        assert!(message.contains("drop the topic instead"), "{message}");
    }
}
