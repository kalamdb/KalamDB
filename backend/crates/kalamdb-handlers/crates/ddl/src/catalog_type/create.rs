//! Typed handler for CREATE TYPE.

use std::sync::Arc;

use kalamdb_commons::{
    models::{CatalogTypeKind, SchemaId, TableId, TopicId, TypeId},
    schemas::{TableDefinition, TableName},
};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{CreateTypeBody, CreateTypeStatement, TypeReference};
use kalamdb_system::{CatalogStores, CatalogType, CatalogTypeField};

use crate::helpers::{
    async_blocking::run_blocking,
    guards::{require_admin, require_existing_namespace},
};

pub struct CreateTypeHandler {
    app_context: Arc<AppContext>,
}

impl CreateTypeHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<CreateTypeStatement> for CreateTypeHandler {
    async fn execute(
        &self,
        statement: CreateTypeStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "create type")?;
        let namespace_id = statement.namespace_id.clone();
        let name = statement.name.clone();
        let app = Arc::clone(&self.app_context);
        let result = run_blocking(move || persist_create_type(&app, statement)).await?;
        super::replicate_named_type(&self.app_context, &namespace_id, &name).await?;
        Ok(result)
    }
}

fn persist_create_type(
    app: &AppContext,
    statement: CreateTypeStatement,
) -> Result<ExecutionResult, KalamDbError> {
    require_existing_namespace(app, &statement.namespace_id)?;
    let stores = app.system_tables().catalog_stores();
    if stores
        .find_type(&statement.namespace_id, &statement.name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .is_some()
    {
        if statement.if_not_exists {
            return Ok(ExecutionResult::Success {
                message: format!(
                    "Type {}.{} already exists, skipping",
                    statement.namespace_id, statement.name
                ),
            });
        }
        return Err(KalamDbError::AlreadyExists(format!(
            "type {}.{} already exists",
            statement.namespace_id, statement.name
        )));
    }
    let table_id = TableId::new(
        statement.namespace_id.clone(),
        TableName::new(statement.name.as_str()),
    );
    if app.schema_registry().get(&table_id).is_some() {
        return Err(KalamDbError::AlreadyExists(format!(
            "type {}.{} already exists",
            statement.namespace_id, statement.name
        )));
    }
    let topic_id = TopicId::new(format!("{}.{}", statement.namespace_id.as_str(), statement.name));
    if app
        .system_tables()
        .topics()
        .get_topic_by_id(&topic_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .is_some()
    {
        return Err(KalamDbError::AlreadyExists(format!(
            "type {}.{} collides with topic payload type",
            statement.namespace_id, statement.name
        )));
    }

    let type_id = TypeId::generate();
    match &statement.body {
        CreateTypeBody::Composite { fields } => {
            for field in fields {
                require_type_reference(
                    &stores,
                    &statement.namespace_id,
                    &field.type_ref,
                    Some((&statement.namespace_id, statement.name.as_str())),
                )?;
            }
            let catalog_fields = fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    catalog_field(
                        &stores,
                        &type_id,
                        &statement.namespace_id,
                        field.name.clone(),
                        field.type_ref.clone(),
                        (index + 1) as i32,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let next_slot =
                catalog_fields.iter().map(|field| field.physical_slot()).max().unwrap_or(0) + 1;
            check_new_type_graph(&stores, &type_id, &catalog_fields)?;
            upsert_named_type(
                &stores,
                &statement,
                &type_id,
                CatalogTypeKind::Composite,
                next_slot,
            )?;
            stores
                .publish_type_fields(catalog_fields)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        },
        CreateTypeBody::Enum { labels } => {
            upsert_named_type(
                &stores,
                &statement,
                &type_id,
                CatalogTypeKind::Enum,
                (labels.len() as i32) + 1,
            )?;
            let catalog_fields = labels
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    CatalogTypeField::new(
                        type_id.clone(),
                        label.clone(),
                        (index + 1) as i32,
                        None,
                        None,
                        label.clone(),
                        false,
                        true,
                        false,
                    )
                    .map_err(KalamDbError::InvalidSql)
                })
                .collect::<Result<Vec<_>, _>>()?;
            stores
                .publish_type_fields(catalog_fields)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        },
        CreateTypeBody::FromTable { .. } => {
            return Err(KalamDbError::InvalidSql(
                "CREATE TYPE ... FROM TABLE is not supported in PostgreSQL 18".to_string(),
            ));
        },
    }

    Ok(ExecutionResult::Success {
        message: format!("Type {}.{} created", statement.namespace_id, statement.name),
    })
}

fn upsert_named_type(
    stores: &CatalogStores,
    statement: &CreateTypeStatement,
    type_id: &TypeId,
    kind: CatalogTypeKind,
    next_slot: i32,
) -> Result<(), KalamDbError> {
    let mut catalog_type = CatalogType::named(
        type_id.clone(),
        statement.namespace_id.clone(),
        statement.name.clone(),
        kind,
    );
    catalog_type.comment = statement.comment.clone();
    catalog_type.next_slot = next_slot.max(1);
    stores
        .upsert_type(catalog_type)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))
}

pub fn ensure_implicit_row_type(
    stores: &CatalogStores,
    table_id: &TableId,
    table_def: &TableDefinition,
) -> Result<TypeId, KalamDbError> {
    if let Some(existing) = stores
        .find_type(table_id.namespace_id(), table_id.table_name().as_str())
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
    {
        if existing.kind != CatalogTypeKind::ImplicitTableRow {
            return Err(KalamDbError::AlreadyExists(match existing.kind {
                CatalogTypeKind::TopicPayload => {
                    format!("topic '{}' collides with implicit table row type", existing.type_id)
                },
                _ => format!("type {}.{} already exists", existing.namespace_id, existing.name),
            }));
        }
        return Ok(existing.type_id);
    }

    let type_id = TypeId::generate();
    let mut catalog_type = CatalogType::named(
        type_id.clone(),
        table_id.namespace_id().clone(),
        table_id.table_name().as_str(),
        CatalogTypeKind::ImplicitTableRow,
    );
    catalog_type.table_id = Some(table_id.clone());
    catalog_type.next_slot = (table_def.columns.len() as i32) + 1;
    stores
        .upsert_type(catalog_type)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;

    let fields = table_def
        .columns
        .iter()
        .filter(|column| {
            !kalamdb_commons::constants::SystemColumnNames::is_system_column(&column.column_name)
        })
        .map(|column| {
            CatalogTypeField::from_column(&type_id, column).map_err(KalamDbError::InvalidSql)
        })
        .collect::<Result<Vec<_>, _>>()?;
    stores
        .publish_type_fields(fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(type_id)
}

pub fn sync_implicit_row_type(
    stores: &CatalogStores,
    table_id: &TableId,
    table_def: &TableDefinition,
) -> Result<Option<TypeId>, KalamDbError> {
    let Some(mut existing) = stores
        .find_type(table_id.namespace_id(), table_id.table_name().as_str())
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
    else {
        let type_id = ensure_implicit_row_type(stores, table_id, table_def)?;
        return Ok(Some(type_id));
    };
    if existing.kind != CatalogTypeKind::ImplicitTableRow {
        return Ok(None);
    }
    let type_id = existing.type_id.clone();
    existing.type_revision = existing.type_revision.saturating_add(1);
    existing.next_slot = existing.next_slot.max((table_def.columns.len() as i32) + 1);
    stores
        .upsert_type(existing)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;

    let fields = table_def
        .columns
        .iter()
        .filter(|column| {
            !kalamdb_commons::constants::SystemColumnNames::is_system_column(&column.column_name)
        })
        .map(|column| {
            CatalogTypeField::from_column(&type_id, column).map_err(KalamDbError::InvalidSql)
        })
        .collect::<Result<Vec<_>, _>>()?;
    stores
        .replace_type_fields(&type_id, fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(Some(type_id))
}

pub(super) fn catalog_field(
    stores: &CatalogStores,
    type_id: &TypeId,
    current_schema: &SchemaId,
    name: String,
    type_ref: TypeReference,
    ordinal: i32,
) -> Result<CatalogTypeField, KalamDbError> {
    let mut field = CatalogTypeField::new(
        type_id.clone(),
        name,
        ordinal,
        resolve_named_type_id(stores, current_schema, &type_ref)?,
        type_ref.builtin_data_type(),
        type_ref.resolved_type_name(current_schema),
        type_ref.is_array,
        type_ref.not_null,
        type_ref.nonempty,
    )
    .map_err(KalamDbError::InvalidSql)?;
    field.element_nullable = !type_ref.not_null || type_ref.is_array;
    Ok(field)
}

pub(crate) fn require_type_reference(
    stores: &CatalogStores,
    current_schema: &SchemaId,
    type_ref: &TypeReference,
    creating: Option<(&SchemaId, &str)>,
) -> Result<(), KalamDbError> {
    let Some((namespace_id, name)) = type_ref.resolved_name(current_schema) else {
        return Ok(());
    };
    if creating.is_some_and(|(cns, cname)| cns == &namespace_id && cname == name) {
        return Err(KalamDbError::InvalidSql(format!(
            "type {namespace_id}.{name} cannot reference itself"
        )));
    }
    resolve_named_type_id(stores, current_schema, type_ref).map(|_| ())
}

/// Live catalog TypeId for a named type reference. Builtins return `None`.
pub(crate) fn resolve_named_type_id(
    stores: &CatalogStores,
    current_schema: &SchemaId,
    type_ref: &TypeReference,
) -> Result<Option<TypeId>, KalamDbError> {
    let Some((namespace_id, name)) = type_ref.resolved_name(current_schema) else {
        return Ok(None);
    };
    let found = stores
        .find_type(&namespace_id, &name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .ok_or_else(|| KalamDbError::NotFound(format!("type {namespace_id}.{name} not found")))?;
    Ok(Some(found.type_id))
}

pub(super) fn check_new_type_graph(
    stores: &CatalogStores,
    type_id: &TypeId,
    fields: &[CatalogTypeField],
) -> Result<(), KalamDbError> {
    use std::collections::HashMap;

    use kalamdb_commons::{assert_finite_type_graph, TypeGraphNode};

    let mut nodes = HashMap::new();
    for catalog_type in stores
        .list_types()
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
    {
        let existing_fields = stores
            .list_type_fields(&catalog_type.type_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        let refs: Vec<_> = existing_fields
            .iter()
            .filter(|field| !field.dropped)
            .filter_map(|field| field.type_ref().ok())
            .collect();
        nodes.insert(catalog_type.type_id, TypeGraphNode::from_type_refs(&refs));
    }
    let refs: Vec<_> = fields.iter().filter_map(|field| field.type_ref().ok()).collect();
    nodes.insert(type_id.clone(), TypeGraphNode::from_type_refs(&refs));
    assert_finite_type_graph(type_id, &nodes)
        .map_err(|error| KalamDbError::InvalidSql(error.message))
}

pub(crate) fn require_procedure_types(
    stores: &CatalogStores,
    statement: &kalamdb_sql::ddl::CreateProcedureStatement,
) -> Result<(), KalamDbError> {
    for parameter in &statement.parameters {
        require_type_reference(stores, &statement.namespace_id, &parameter.type_ref, None)?;
    }
    if let Some(return_type) = &statement.return_type {
        require_type_reference(stores, &statement.namespace_id, return_type, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{models::SchemaId, Role};
    use kalamdb_core::{
        sql::{context::ExecutionContext, executor::handlers::TypedStatementHandler},
        test_helpers::{create_test_session_for, test_app_context_simple},
    };
    use kalamdb_sql::ddl::CreateTypeStatement;

    use super::*;

    fn dba_ctx(app: &Arc<AppContext>) -> ExecutionContext {
        ExecutionContext::new(
            kalamdb_commons::models::UserId::new("root"),
            Role::Dba,
            create_test_session_for(app),
        )
    }

    fn ensure_namespace(app: &AppContext, name: &str) {
        let namespaces = app.system_tables().namespaces();
        let namespace_id = SchemaId::new(name);
        if namespaces.get_namespace(&namespace_id).unwrap().is_none() {
            namespaces.create_namespace(kalamdb_system::Namespace::new(name)).unwrap();
        }
    }

    #[tokio::test]
    async fn unknown_nested_type_is_rejected() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        let handler = CreateTypeHandler::new(Arc::clone(&app));
        let statement = CreateTypeStatement::parse(
            "CREATE TYPE app.envelope AS (status app.missing_status)",
            &SchemaId::new("app"),
        )
        .expect("parse");
        let error = handler.execute(statement, vec![], &dba_ctx(&app)).await.unwrap_err();
        let message = error.to_string();
        assert!(message.contains("app.missing_status"), "{message}");
        assert!(message.contains("not found"), "{message}");
    }

    #[tokio::test]
    async fn self_referential_composite_is_rejected() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        let handler = CreateTypeHandler::new(Arc::clone(&app));
        let statement = CreateTypeStatement::parse(
            "CREATE TYPE app.node AS (child app.node)",
            &SchemaId::new("app"),
        )
        .expect("parse");
        let error = handler.execute(statement, vec![], &dba_ctx(&app)).await.unwrap_err();
        assert!(error.to_string().contains("cannot reference itself"), "{}", error);
    }

    #[tokio::test]
    async fn create_type_rejects_existing_topic_name() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        let topic_id = kalamdb_commons::models::TopicId::new("app.inbox");
        app.system_tables()
            .topics()
            .create_topic(kalamdb_system::providers::topics::models::Topic::new(
                topic_id,
                "app.inbox".to_string(),
            ))
            .unwrap();
        let handler = CreateTypeHandler::new(Arc::clone(&app));
        let statement = CreateTypeStatement::parse(
            "CREATE TYPE app.inbox AS ENUM ('a')",
            &SchemaId::new("app"),
        )
        .expect("parse");
        let error = handler.execute(statement, vec![], &dba_ctx(&app)).await.unwrap_err();
        let message = error.to_string();
        assert!(message.contains("app.inbox"), "{message}");
        assert!(message.contains("already exists") || message.contains("collides"), "{message}");
    }
}
