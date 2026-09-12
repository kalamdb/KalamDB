use std::sync::Arc;

use kalamdb_commons::{
    models::{CatalogTypeKind, NamespaceId, TopicId, TypeId},
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
use kalamdb_sql::ddl::CreateTopicStatement;
use kalamdb_system::providers::topics::models::Topic;

use super::name_resolution::resolve_topic_name;

pub struct CreateTopicHandler {
    app_context: Arc<AppContext>,
}

impl CreateTopicHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    fn extract_namespace_id(topic_name: &str) -> Result<NamespaceId, KalamDbError> {
        let (namespace, topic_local_name) = topic_name.split_once('.').ok_or_else(|| {
            KalamDbError::InvalidOperation(
                "Topic name must be namespace-qualified: <namespace>.<topic>".to_string(),
            )
        })?;

        if namespace.is_empty() || topic_local_name.is_empty() {
            return Err(KalamDbError::InvalidOperation(
                "Topic name must be namespace-qualified: <namespace>.<topic>".to_string(),
            ));
        }

        Ok(NamespaceId::new(namespace))
    }

    fn resolve_retention_value(
        sql_value: Option<Option<i64>>,
        default_value: i64,
        option_name: &str,
    ) -> Result<Option<i64>, KalamDbError> {
        let value = sql_value.unwrap_or(Some(default_value));
        if matches!(value, Some(v) if v <= 0) {
            return Err(KalamDbError::InvalidOperation(format!(
                "{} must be greater than 0 or NULL",
                option_name
            )));
        }
        Ok(value)
    }
}

impl TypedStatementHandler<CreateTopicStatement> for CreateTopicHandler {
    async fn execute(
        &self,
        statement: CreateTopicStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let topic_name = resolve_topic_name(&statement.topic_name, context);
        let namespace_id = Self::extract_namespace_id(&topic_name)?;
        let namespaces_provider = self.app_context.system_tables().namespaces();
        if namespaces_provider.get_namespace_async(&namespace_id).await?.is_none() {
            return Err(KalamDbError::NotFound(format!(
                "Namespace '{}' does not exist",
                namespace_id
            )));
        }

        let topic_id = TopicId::new(&topic_name);
        let topics_provider = self.app_context.system_tables().topics();
        let stores = self.app_context.system_tables().catalog_stores();
        if topics_provider.get_topic_by_id_async(&topic_id).await?.is_some() {
            if statement.if_not_exists {
                stores
                    .ensure_implicit_topic_payload_type(&topic_id)
                    .map_err(super::catalog_error)?;
                return Ok(ExecutionResult::Success {
                    message: format!("Topic {} already exists (IF NOT EXISTS)", topic_name),
                });
            }
            return Err(KalamDbError::AlreadyExists(format!(
                "Topic '{}' already exists",
                topic_name
            )));
        }
        if let Some(existing) =
            stores.get_type(&TypeId::new(topic_id.as_str())).map_err(super::catalog_error)?
        {
            if existing.kind != CatalogTypeKind::TopicPayload {
                return Err(KalamDbError::AlreadyExists(match existing.kind {
                    CatalogTypeKind::ImplicitTableRow => {
                        format!("topic '{topic_name}' collides with implicit table row type")
                    },
                    _ => format!("type {} already exists", existing.type_id),
                }));
            }
        }

        let mut topic = Topic::new(topic_id.clone(), topic_name.clone());
        topic.partitions = statement.partitions.unwrap_or(1);
        if topic.partitions == 0 {
            return Err(KalamDbError::InvalidOperation(
                "Topic partitions must be greater than 0".to_string(),
            ));
        }
        let topic_config = &self.app_context.config().topics;
        topic.retention_seconds = Self::resolve_retention_value(
            statement.retention_seconds,
            topic_config.default_retention_seconds,
            "retention_seconds",
        )?;
        topic.retention_max_bytes = Self::resolve_retention_value(
            statement.retention_max_bytes,
            topic_config.default_retention_max_bytes,
            "retention_max_bytes",
        )?;

        topics_provider.create_topic_async(topic.clone()).await?;
        self.app_context.topic_publisher().add_topic(topic.clone());
        stores
            .ensure_implicit_topic_payload_type(&topic_id)
            .map_err(super::catalog_error)?;

        Ok(ExecutionResult::Success {
            message: format!(
                "Created topic '{}' with {} partition(s)",
                topic_name, topic.partitions
            ),
        })
    }

    async fn check_authorization(
        &self,
        _statement: &CreateTopicStatement,
        context: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        match context.user_role() {
            Role::Dba | Role::System => Ok(()),
            _ => Err(KalamDbError::PermissionDenied(
                "CREATE TOPIC requires DBA or System role".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{
        models::{CatalogTypeKind, NamespaceId, TypeId, UserId},
        Role,
    };
    use kalamdb_core::{
        sql::{context::ExecutionContext, executor::handlers::TypedStatementHandler},
        test_helpers::{create_test_session_for, test_app_context_simple},
    };
    use kalamdb_sql::ddl::{CreateTopicStatement, DropTopicStatement};

    use super::{super::DropTopicHandler, *};

    fn dba_ctx(app: &Arc<kalamdb_core::app_context::AppContext>) -> ExecutionContext {
        ExecutionContext::new(UserId::new("root"), Role::Dba, create_test_session_for(app))
    }

    fn ensure_namespace(app: &kalamdb_core::app_context::AppContext, name: &str) {
        let namespaces = app.system_tables().namespaces();
        let namespace_id = NamespaceId::new(name);
        if namespaces.get_namespace(&namespace_id).unwrap().is_none() {
            namespaces.create_namespace(kalamdb_system::Namespace::new(name)).unwrap();
        }
    }

    #[tokio::test]
    async fn create_topic_catalogs_implicit_payload_type() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        let handler = CreateTopicHandler::new(Arc::clone(&app));
        handler
            .execute(
                CreateTopicStatement {
                    topic_name:          "app.inbox".to_string(),
                    if_not_exists:       false,
                    partitions:          Some(1),
                    retention_seconds:   None,
                    retention_max_bytes: None,
                },
                vec![],
                &dba_ctx(&app),
            )
            .await
            .expect("create topic");
        let catalog_type = app
            .system_tables()
            .catalog_stores()
            .get_type(&TypeId::new("app.inbox"))
            .unwrap()
            .expect("payload type");
        assert_eq!(catalog_type.kind, CatalogTypeKind::TopicPayload);
        assert_eq!(catalog_type.namespace_id, NamespaceId::new("app"));
        assert_eq!(catalog_type.name, "inbox");
    }

    #[tokio::test]
    async fn create_topic_rejects_existing_table_row_type() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        app.system_tables()
            .catalog_stores()
            .upsert_type(kalamdb_system::CatalogType {
                type_id:        TypeId::new("app.inbox"),
                namespace_id:   NamespaceId::new("app"),
                name:           "inbox".to_string(),
                kind:           CatalogTypeKind::ImplicitTableRow,
                table_id:       Some(kalamdb_commons::models::TableId::from_strings(
                    "app", "inbox",
                )),
                source_type_id: None,
                comment:        None,
            })
            .unwrap();
        let handler = CreateTopicHandler::new(Arc::clone(&app));
        let error = handler
            .execute(
                CreateTopicStatement {
                    topic_name:          "app.inbox".to_string(),
                    if_not_exists:       false,
                    partitions:          Some(1),
                    retention_seconds:   None,
                    retention_max_bytes: None,
                },
                vec![],
                &dba_ctx(&app),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("collides"), "{error}");
        assert!(error.to_string().contains("implicit table row type"), "{error}");
    }

    #[tokio::test]
    async fn create_topic_rejects_existing_named_type() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        app.system_tables()
            .catalog_stores()
            .upsert_type(kalamdb_system::CatalogType {
                type_id:        TypeId::new("app.inbox"),
                namespace_id:   NamespaceId::new("app"),
                name:           "inbox".to_string(),
                kind:           CatalogTypeKind::Composite,
                table_id:       None,
                source_type_id: None,
                comment:        None,
            })
            .unwrap();
        let handler = CreateTopicHandler::new(Arc::clone(&app));
        let error = handler
            .execute(
                CreateTopicStatement {
                    topic_name:          "app.inbox".to_string(),
                    if_not_exists:       false,
                    partitions:          Some(1),
                    retention_seconds:   None,
                    retention_max_bytes: None,
                },
                vec![],
                &dba_ctx(&app),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
    }

    #[tokio::test]
    async fn drop_topic_removes_implicit_payload_type() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        let ctx = dba_ctx(&app);
        CreateTopicHandler::new(Arc::clone(&app))
            .execute(
                CreateTopicStatement {
                    topic_name:          "app.inbox".to_string(),
                    if_not_exists:       false,
                    partitions:          Some(1),
                    retention_seconds:   None,
                    retention_max_bytes: None,
                },
                vec![],
                &ctx,
            )
            .await
            .expect("create topic");
        DropTopicHandler::new(Arc::clone(&app))
            .execute(
                DropTopicStatement {
                    topic_name: "app.inbox".to_string(),
                    if_exists:  false,
                },
                vec![],
                &ctx,
            )
            .await
            .expect("drop topic");
        assert!(app
            .system_tables()
            .catalog_stores()
            .get_type(&TypeId::new("app.inbox"))
            .unwrap()
            .is_none());
    }
}
