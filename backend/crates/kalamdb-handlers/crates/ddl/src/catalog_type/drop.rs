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
        let (result, dropped_ids) = run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            let targets: Vec<(kalamdb_commons::models::SchemaId, String)> =
                if statement.targets.is_empty() {
                    vec![(statement.namespace_id.clone(), statement.name.clone())]
                } else {
                    statement.targets.clone()
                };
            let mut to_drop = Vec::new();
            for (namespace_id, name) in &targets {
                let existing = stores
                    .find_type(namespace_id, name)
                    .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                let Some(catalog_type) = existing else {
                    if statement.if_exists {
                        continue;
                    }
                    return Err(KalamDbError::NotFound(format!(
                        "type {namespace_id}.{name} not found"
                    )));
                };
                if catalog_type.namespace_id.is_system_namespace() {
                    return Err(KalamDbError::InvalidSql(format!(
                        "cannot drop built-in system type {}.{}",
                        catalog_type.namespace_id, catalog_type.name
                    )));
                }
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
                    CatalogTypeKind::RowAlias
                    | CatalogTypeKind::Composite
                    | CatalogTypeKind::Enum
                    | CatalogTypeKind::Domain => {},
                }
                to_drop.push(catalog_type);
            }
            if to_drop.is_empty() {
                return Ok((
                    ExecutionResult::Success {
                        message: format!(
                            "Type {}.{} does not exist, skipping",
                            statement.namespace_id, statement.name
                        ),
                    },
                    Vec::new(),
                ));
            }
            let tables = app
                .system_tables()
                .tables()
                .list_tables()
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            for catalog_type in &to_drop {
                if let Some(dependent) = stores
                    .type_is_referenced(&catalog_type.type_id)
                    .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
                {
                    return Err(KalamDbError::InvalidOperation(format!(
                        "cannot drop type {}: referenced by {dependent}",
                        crate::catalog_type::type_alias(catalog_type)
                    )));
                }
                for table_def in &tables {
                    for col in &table_def.columns {
                        if col.named_type_id() == Some(&catalog_type.type_id) {
                            return Err(KalamDbError::InvalidOperation(format!(
                                "cannot drop type {}: referenced by column {}.{}.{}",
                                crate::catalog_type::type_alias(catalog_type),
                                table_def.namespace_id,
                                table_def.table_name,
                                col.column_name
                            )));
                        }
                    }
                }
            }
            let mut dropped_ids = Vec::with_capacity(to_drop.len());
            let mut dropped_names = Vec::with_capacity(to_drop.len());
            for catalog_type in to_drop {
                let alias = crate::catalog_type::type_alias(&catalog_type);
                let type_id = catalog_type.type_id.clone();
                stores
                    .drop_type(&type_id)
                    .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                app.schema_registry().type_registry().invalidate(&type_id);
                dropped_ids.push(type_id);
                dropped_names.push(alias);
            }
            Ok((
                ExecutionResult::Success {
                    message: format!("Type {} dropped", dropped_names.join(", ")),
                },
                dropped_ids,
            ))
        })
        .await?;
        for type_id in dropped_ids {
            super::replicate_dropped_type(&self.app_context, type_id).await?;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{
        models::{SchemaId, TopicId, UserId},
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
        let namespace_id = SchemaId::new("app");
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
