//! Typed DDL handler for CREATE NAMESPACE statements
//!
//! This module demonstrates the TypedStatementHandler pattern where handlers
//! receive fully parsed AST structs instead of raw SQL strings.
//!
//! When a namespace is created, it is also registered as a DataFusion schema
//! so that queries like `SELECT * FROM namespace.table` work correctly.

use std::sync::Arc;

use datafusion::catalog::MemorySchemaProvider;
use kalamdb_commons::models::{SchemaId, UserId};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::CreateSchemaStatement;

use crate::helpers::{async_blocking::run_blocking, audit, guards::require_admin};

/// Typed handler for CREATE NAMESPACE statements
pub struct CreateSchemaHandler {
    app_context: Arc<AppContext>,
}

fn is_namespace_already_exists_error(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("namespace") && message.contains("already exists")
}

impl CreateSchemaHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    /// Register namespace as a DataFusion schema in the kalam catalog
    ///
    /// This enables queries like `SELECT * FROM namespace.table` to work.
    /// Uses DataFusion's native MemorySchemaProvider for dynamic schema registration.
    fn register_namespace_schema(&self, namespace_id: &SchemaId) -> Result<(), KalamDbError> {
        let base_session = self.app_context.base_session_context();

        // Get the "kalam" catalog (default catalog configured in DataFusionSessionFactory)
        let catalog = base_session.catalog("kalam").ok_or_else(|| {
            KalamDbError::CatalogError("kalam catalog not found in session".to_string())
        })?;

        // Check if schema already registered (idempotent)
        if catalog.schema(namespace_id.as_str()).is_some() {
            log::debug!(
                "Schema '{}' already registered in DataFusion catalog",
                namespace_id.as_str()
            );
            return Ok(());
        }

        // Create a new schema provider for this namespace
        // Tables will be registered here when CREATE TABLE is executed
        let schema_provider = Arc::new(MemorySchemaProvider::new());

        catalog.register_schema(namespace_id.as_str(), schema_provider).map_err(|e| {
            KalamDbError::CatalogError(format!(
                "Failed to register schema '{}': {}",
                namespace_id.as_str(),
                e
            ))
        })?;

        log::debug!("Registered DataFusion schema for namespace '{}'", namespace_id.as_str());

        Ok(())
    }
}

impl CreateSchemaHandler {
    async fn create_catalog_schema(
        &self,
        name: &str,
        if_not_exists: bool,
        context: &ExecutionContext,
        audit_kind: &str,
    ) -> Result<ExecutionResult, KalamDbError> {
        kalamdb_sql::validation::validate_namespace_name(name).map_err(|e| e.to_string())?;

        let namespace_id = SchemaId::new(name);
        let app_ctx = self.app_context.clone();
        let ns_id = namespace_id.clone();
        let existing =
            run_blocking(move || app_ctx.system_tables().namespaces().get_namespace(&ns_id))
                .await?;

        if existing.is_some() {
            if if_not_exists {
                self.register_namespace_schema(&namespace_id)?;
                let message = format!("Schema '{name}' already exists");
                return Ok(ExecutionResult::Success { message });
            }
            return Err(KalamDbError::AlreadyExists(format!("Schema '{name}' already exists")));
        }

        // Cluster mode replicates through Raft. The command name stays
        // CreateNamespace so stored meta bytes do not change.
        let executor = self.app_context.executor();
        let created_by = Some(UserId::new(context.user_id().as_str()));
        let cmd = if if_not_exists {
            kalamdb_raft::MetaCommand::CreateNamespaceIfNotExists {
                namespace_id: namespace_id.clone(),
                created_by,
            }
        } else {
            kalamdb_raft::MetaCommand::CreateNamespace {
                namespace_id: namespace_id.clone(),
                created_by,
            }
        };

        if let Err(error) = executor.execute_meta(cmd).await {
            if if_not_exists && is_namespace_already_exists_error(&error) {
                self.register_namespace_schema(&namespace_id)?;
                let message = format!("Schema '{name}' already exists");
                return Ok(ExecutionResult::Success { message });
            }

            return Err(KalamDbError::ExecutionError(format!(
                "Failed to create schema via executor: {error}"
            )));
        }

        self.register_namespace_schema(&namespace_id)?;

        let audit_entry = audit::log_ddl_operation(context, "CREATE", audit_kind, name, None, None);
        audit::persist_audit_entry(&self.app_context, &audit_entry).await?;

        Ok(ExecutionResult::Success {
            message: format!("Schema '{name}' created successfully"),
        })
    }
}

impl TypedStatementHandler<CreateSchemaStatement> for CreateSchemaHandler {
    async fn execute(
        &self,
        statement: CreateSchemaStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        self.create_catalog_schema(
            statement.name.as_str(),
            statement.if_not_exists,
            context,
            "SCHEMA",
        )
        .await
    }

    async fn check_authorization(
        &self,
        _statement: &CreateSchemaStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        use crate::helpers::guards::block_anonymous_write;

        block_anonymous_write(context, "CREATE SCHEMA")?;
        require_admin(context, "create schema")
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{models::UserId, Role};
    use kalamdb_core::test_helpers::{create_test_session_simple, test_app_context_simple};
    use kalamdb_system::Namespace;

    use super::*;

    fn test_context() -> ExecutionContext {
        ExecutionContext::new(UserId::from("test_user"), Role::Dba, create_test_session_simple())
    }

    #[test]
    fn test_namespace_already_exists_error_recognizer() {
        assert!(is_namespace_already_exists_error(&"Namespace 'app' already exists"));
        assert!(is_namespace_already_exists_error(
            &"Already exists: Namespace 'app' already exists"
        ));
        assert!(!is_namespace_already_exists_error(&"Table 'app.t' already exists"));
    }

    #[ignore = "Requires Raft for CREATE NAMESPACE"]
    #[tokio::test]
    async fn test_typed_create_namespace() {
        let app_ctx = test_app_context_simple();
        let handler = CreateSchemaHandler::new(app_ctx);
        let ctx = test_context();

        let stmt = CreateSchemaStatement {
            name: SchemaId::new("test_typed_ns"),
            if_not_exists: false,
        };

        let result = handler.execute(stmt, vec![], &ctx).await;
        assert!(result.is_ok());

        match result.unwrap() {
            ExecutionResult::Success { message } => {
                assert!(message.contains("test_typed_ns"));
                assert!(message.contains("created successfully"));
            },
            _ => panic!("Expected Success result"),
        }
    }

    #[ignore = "Requires Raft for CREATE NAMESPACE"]
    #[tokio::test]
    async fn test_typed_create_namespace_if_not_exists() {
        let app_ctx = test_app_context_simple();
        let handler = CreateSchemaHandler::new(app_ctx);
        let ctx = test_context();

        let stmt = CreateSchemaStatement {
            name: SchemaId::new("test_typed_ns_ine"),
            if_not_exists: true,
        };

        // First creation should succeed
        let result1 = handler.execute(stmt.clone(), vec![], &ctx).await;
        assert!(result1.is_ok());

        // Second creation with IF NOT EXISTS should also succeed
        let result2 = handler.execute(stmt, vec![], &ctx).await;
        assert!(result2.is_ok());

        match result2.unwrap() {
            ExecutionResult::Success { message } => assert!(message.contains("already exists")),
            _ => panic!("Expected Success result"),
        }
    }

    #[tokio::test]
    async fn test_typed_authorization_check() {
        let app_ctx = test_app_context_simple();
        let handler = CreateSchemaHandler::new(app_ctx);
        let user_ctx = ExecutionContext::new(
            UserId::from("regular_user"),
            Role::User,
            create_test_session_simple(),
        );

        let stmt = CreateSchemaStatement {
            name: SchemaId::new("unauthorized_ns"),
            if_not_exists: false,
        };

        let auth_result = handler.check_authorization(&stmt, &user_ctx).await;
        assert!(auth_result.is_err());

        if let Err(KalamDbError::Unauthorized(msg)) = auth_result {
            assert!(msg.contains("Insufficient privileges"));
        } else {
            panic!("Expected Unauthorized error");
        }
    }

    #[tokio::test]
    async fn test_typed_create_namespace_already_exists_returns_error_without_if_not_exists() {
        let app_ctx = test_app_context_simple();
        app_ctx
            .system_tables()
            .namespaces()
            .create_namespace(Namespace::new("existing_ns"))
            .expect("failed to seed namespace");

        let handler = CreateSchemaHandler::new(app_ctx);
        let ctx = test_context();
        let stmt = CreateSchemaStatement {
            name: SchemaId::new("existing_ns"),
            if_not_exists: false,
        };

        let result = handler.execute(stmt, vec![], &ctx).await;
        assert!(matches!(result, Err(KalamDbError::AlreadyExists(_))));
    }

    #[tokio::test]
    async fn test_typed_create_namespace_already_exists_succeeds_with_if_not_exists() {
        let app_ctx = test_app_context_simple();
        app_ctx
            .system_tables()
            .namespaces()
            .create_namespace(Namespace::new("existing_ns_if"))
            .expect("failed to seed namespace");

        let handler = CreateSchemaHandler::new(app_ctx);
        let ctx = test_context();
        let stmt = CreateSchemaStatement {
            name: SchemaId::new("existing_ns_if"),
            if_not_exists: true,
        };

        let result = handler.execute(stmt, vec![], &ctx).await;
        assert!(result.is_ok());
    }
}
