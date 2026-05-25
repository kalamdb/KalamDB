//! Namespace Executor - CREATE/DROP NAMESPACE operations
//!
//! This is the SINGLE place where namespace mutations happen.
//! All methods use spawn_blocking to avoid blocking the tokio runtime
//! with synchronous RocksDB calls.

use std::sync::Arc;

use kalamdb_commons::models::{NamespaceId, TableId};
use kalamdb_system::Namespace;

use crate::{
    app_context::AppContext,
    applier::{
        executor::utils::{run_blocking_applier, with_plan_cache_invalidation},
        ApplierError,
    },
};

/// Executor for namespace operations
pub struct NamespaceExecutor {
    app_context: Arc<AppContext>,
}

impl NamespaceExecutor {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    /// Execute CREATE NAMESPACE
    pub async fn create_namespace(
        &self,
        namespace_id: &NamespaceId,
    ) -> Result<String, ApplierError> {
        log::debug!("CommandExecutorImpl: Creating namespace {}", namespace_id);
        let app_context = self.app_context.clone();
        let namespace_id = namespace_id.clone();
        run_blocking_applier(move || {
            let namespace = Namespace::new(namespace_id.as_str());
            app_context
                .system_tables()
                .namespaces()
                .create_namespace(namespace)
                .map_err(|e| {
                    ApplierError::Execution(format!("Failed to create namespace: {}", e))
                })?;
            Ok(format!("Namespace {} created successfully", namespace_id))
        })
        .await
    }

    /// Execute DROP NAMESPACE
    pub async fn drop_namespace(&self, namespace_id: &NamespaceId) -> Result<String, ApplierError> {
        log::debug!("CommandExecutorImpl: Dropping namespace {}", namespace_id);
        let app_context = self.app_context.clone();
        let namespace_id = namespace_id.clone();
        with_plan_cache_invalidation(app_context, move |app_context: Arc<AppContext>| async move {
            run_blocking_applier(move || {
                let tables = app_context
                    .system_tables()
                    .tables()
                    .list_tables_in_namespace(&namespace_id)
                    .map_err(|e| {
                        ApplierError::Execution(format!(
                            "Failed to list tables in namespace {}: {}",
                            namespace_id, e
                        ))
                    })?;

                let dropped_tables = tables.len();
                for table in tables {
                    let table_id =
                        TableId::new(table.namespace_id.clone(), table.table_name.clone());
                    app_context.schema_registry().delete_table_definition(&table_id).map_err(
                        |e| {
                            ApplierError::Execution(format!(
                                "Failed to drop table {} while dropping namespace {}: {}",
                                table_id, namespace_id, e
                            ))
                        },
                    )?;
                }

                app_context
                    .system_tables()
                    .namespaces()
                    .delete_namespace(&namespace_id)
                    .map_err(|e| {
                        ApplierError::Execution(format!("Failed to drop namespace: {}", e))
                    })?;
                Ok(format!(
                    "Namespace {} dropped successfully ({} table(s) removed)",
                    namespace_id, dropped_tables
                ))
            })
            .await
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use kalamdb_commons::{
        models::{
            datatypes::KalamDataType,
            schemas::{ColumnDefinition, TableDefinition, TableOptions},
            TableId, TableName,
        },
        schemas::{ColumnDefault, TableType},
    };

    use super::*;
    use crate::{
        sql::executor::{handler_registry::HandlerRegistry, SqlExecutor},
        test_helpers::test_app_context_simple,
    };

    fn unique_namespace() -> NamespaceId {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_millis();
        NamespaceId::new(format!("drop_ns_{}_{}", std::process::id(), millis))
    }

    fn stream_table_definition(
        namespace_id: &NamespaceId,
        table_name: &TableName,
    ) -> TableDefinition {
        let id_column = ColumnDefinition::new(
            1,
            "id".to_string(),
            1,
            KalamDataType::BigInt,
            false,
            true,
            false,
            ColumnDefault::None,
            None,
        );

        TableDefinition::new(
            namespace_id.clone(),
            table_name.clone(),
            TableType::Stream,
            vec![id_column],
            TableOptions::stream(3600),
            None,
        )
        .expect("create stream table definition")
    }

    #[tokio::test]
    async fn drop_namespace_removes_all_tables_in_namespace() {
        let app_context = test_app_context_simple();
        let sql_executor =
            Arc::new(SqlExecutor::new(app_context.clone(), Arc::new(HandlerRegistry::new())));
        app_context.set_sql_executor(sql_executor);

        let namespace_id = unique_namespace();
        app_context
            .system_tables()
            .namespaces()
            .create_namespace(Namespace::new(namespace_id.as_str()))
            .expect("create namespace");

        let first_table_name = TableName::new("first_table");
        let second_table_name = TableName::new("second_table");
        let first_table_id = TableId::new(namespace_id.clone(), first_table_name.clone());
        let second_table_id = TableId::new(namespace_id.clone(), second_table_name.clone());

        for table_name in [&first_table_name, &second_table_name] {
            let mut table_def = stream_table_definition(&namespace_id, table_name);
            app_context
                .system_columns_service()
                .add_system_columns(&mut table_def)
                .expect("add system columns");
            app_context.schema_registry().register_table(table_def).expect("register table");
        }

        assert_eq!(
            app_context
                .system_tables()
                .tables()
                .list_tables_in_namespace(&namespace_id)
                .expect("list tables before drop")
                .len(),
            2
        );

        let executor = NamespaceExecutor::new(app_context.clone());
        let message = executor.drop_namespace(&namespace_id).await.expect("drop namespace");

        assert!(message.contains("2 table(s) removed"));
        assert!(app_context
            .system_tables()
            .namespaces()
            .get_namespace(&namespace_id)
            .expect("get namespace after drop")
            .is_none());
        assert!(app_context
            .system_tables()
            .tables()
            .list_tables_in_namespace(&namespace_id)
            .expect("list tables after drop")
            .is_empty());
        assert!(app_context
            .schema_registry()
            .get_table_if_exists(&first_table_id)
            .expect("first table lookup")
            .is_none());
        assert!(app_context
            .schema_registry()
            .get_table_if_exists(&second_table_id)
            .expect("second table lookup")
            .is_none());
    }
}
