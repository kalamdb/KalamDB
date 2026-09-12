//! Typed handler for ALTER TYPE.

use std::sync::Arc;

use kalamdb_commons::models::{CatalogTypeKind, NamespaceId, TypeId};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{AlterTypeOperation, AlterTypeStatement, EnumValueNeighbor};
use kalamdb_system::{CatalogStores, CatalogType, CatalogTypeField};

use super::create::{catalog_field, require_type_reference};
use crate::helpers::{async_blocking::run_blocking, guards::require_admin};

pub struct AlterTypeHandler {
    app_context: Arc<AppContext>,
}

impl AlterTypeHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<AlterTypeStatement> for AlterTypeHandler {
    async fn execute(
        &self,
        statement: AlterTypeStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "alter type")?;
        let app = Arc::clone(&self.app_context);
        run_blocking(move || persist_alter_type(&app.system_tables().catalog_stores(), statement))
            .await
    }
}

fn persist_alter_type(
    stores: &CatalogStores,
    statement: AlterTypeStatement,
) -> Result<ExecutionResult, KalamDbError> {
    let catalog_type = stores
        .get_type(&statement.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .ok_or_else(|| KalamDbError::NotFound(format!("type {} not found", statement.type_id)))?;

    match statement.operation {
        AlterTypeOperation::SetSchema { schema } => {
            persist_set_schema(stores, catalog_type, schema)
        },
        AlterTypeOperation::AddValue {
            label,
            if_not_exists,
            neighbor,
        } => persist_add_value(stores, catalog_type, label, if_not_exists, neighbor),
        other => persist_attribute_op(stores, catalog_type, other),
    }
}

fn persist_attribute_op(
    stores: &CatalogStores,
    catalog_type: CatalogType,
    operation: AlterTypeOperation,
) -> Result<ExecutionResult, KalamDbError> {
    if catalog_type.kind != CatalogTypeKind::Composite {
        return Err(KalamDbError::InvalidSql(format!(
            "ALTER TYPE {} attribute operations require a composite type",
            catalog_type.type_id
        )));
    }
    let mut fields = stores
        .list_type_fields(&catalog_type.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    match operation {
        AlterTypeOperation::AddAttribute { field, type_ref } => {
            require_type_reference(
                stores,
                &catalog_type.namespace_id,
                &type_ref,
                Some(&catalog_type.type_id),
            )?;
            if fields.iter().any(|existing| existing.name == field) {
                return Err(KalamDbError::AlreadyExists(format!(
                    "attribute {field} already exists on {}",
                    catalog_type.type_id
                )));
            }
            let ordinal = fields.iter().map(|existing| existing.ordinal).max().unwrap_or(0) + 1;
            fields.push(catalog_field(
                &catalog_type.type_id,
                &catalog_type.namespace_id,
                field,
                type_ref,
                ordinal,
            )?);
        },
        AlterTypeOperation::DropAttribute { field, cascade } => {
            if cascade {
                return Err(KalamDbError::InvalidSql(
                    "DROP ATTRIBUTE CASCADE is not supported; drop dependents first".to_string(),
                ));
            }
            let before = fields.len();
            fields.retain(|existing| existing.name != field);
            if fields.len() == before {
                return Err(KalamDbError::NotFound(format!(
                    "attribute {field} not found on {}",
                    catalog_type.type_id
                )));
            }
        },
        AlterTypeOperation::RenameAttribute { from, to } => {
            if !fields.iter().any(|field| field.name == from) {
                return Err(KalamDbError::NotFound(format!(
                    "attribute {from} not found on {}",
                    catalog_type.type_id
                )));
            }
            if fields.iter().any(|field| field.name == to) {
                return Err(KalamDbError::AlreadyExists(format!(
                    "attribute {to} already exists on {}",
                    catalog_type.type_id
                )));
            }
            for existing in &mut fields {
                if existing.name == from {
                    existing.name = to;
                    existing.type_field_id = kalamdb_commons::models::TypeFieldId::new(
                        &catalog_type.type_id,
                        &existing.name,
                    )
                    .map_err(KalamDbError::InvalidSql)?;
                    break;
                }
            }
        },
        AlterTypeOperation::AlterAttributeType { field, type_ref } => {
            require_type_reference(
                stores,
                &catalog_type.namespace_id,
                &type_ref,
                Some(&catalog_type.type_id),
            )?;
            let Some(existing) = fields.iter_mut().find(|item| item.name == field) else {
                return Err(KalamDbError::NotFound(format!(
                    "attribute {field} not found on {}",
                    catalog_type.type_id
                )));
            };
            let rebuilt = catalog_field(
                &catalog_type.type_id,
                &catalog_type.namespace_id,
                field,
                type_ref,
                existing.ordinal,
            )?;
            *existing = rebuilt;
        },
        AlterTypeOperation::SetSchema { .. } | AlterTypeOperation::AddValue { .. } => {
            unreachable!("handled separately")
        },
    }
    stores
        .replace_type_fields(&catalog_type.type_id, fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!("Type {} altered", catalog_type.type_id),
    })
}

fn persist_add_value(
    stores: &CatalogStores,
    catalog_type: CatalogType,
    label: String,
    if_not_exists: bool,
    neighbor: Option<EnumValueNeighbor>,
) -> Result<ExecutionResult, KalamDbError> {
    if catalog_type.kind != CatalogTypeKind::Enum {
        return Err(KalamDbError::InvalidSql(format!(
            "ALTER TYPE {} ADD VALUE requires an enum type",
            catalog_type.type_id
        )));
    }
    let fields = stores
        .list_type_fields(&catalog_type.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let mut labels: Vec<String> = fields.into_iter().map(|field| field.name).collect();
    if labels.iter().any(|existing| existing == &label) {
        if if_not_exists {
            return Ok(ExecutionResult::Success {
                message: format!(
                    "Enum label '{label}' already exists on {}, skipping",
                    catalog_type.type_id
                ),
            });
        }
        return Err(KalamDbError::AlreadyExists(format!(
            "ENUM label '{label}' already exists on {}",
            catalog_type.type_id
        )));
    }
    let insert_at = match neighbor {
        None => labels.len(),
        Some(EnumValueNeighbor::Before(existing)) => {
            labels.iter().position(|item| item == &existing).ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "ENUM label '{existing}' not found on {}",
                    catalog_type.type_id
                ))
            })?
        },
        Some(EnumValueNeighbor::After(existing)) => {
            labels.iter().position(|item| item == &existing).ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "ENUM label '{existing}' not found on {}",
                    catalog_type.type_id
                ))
            })? + 1
        },
    };
    labels.insert(insert_at, label);
    let catalog_fields = labels
        .iter()
        .enumerate()
        .map(|(index, name)| {
            CatalogTypeField::new(
                catalog_type.type_id.clone(),
                name.clone(),
                (index + 1) as i32,
                None,
                None,
                name.clone(),
                false,
                true,
                false,
            )
            .map_err(KalamDbError::InvalidSql)
        })
        .collect::<Result<Vec<_>, _>>()?;
    stores
        .replace_type_fields(&catalog_type.type_id, catalog_fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!("Type {} altered", catalog_type.type_id),
    })
}

fn persist_set_schema(
    stores: &CatalogStores,
    catalog_type: CatalogType,
    namespace_id: NamespaceId,
) -> Result<ExecutionResult, KalamDbError> {
    if matches!(
        catalog_type.kind,
        CatalogTypeKind::ImplicitTableRow
            | CatalogTypeKind::RowAlias
            | CatalogTypeKind::TopicPayload
    ) {
        return Err(KalamDbError::InvalidSql(format!(
            "cannot SET SCHEMA on {} type {}",
            catalog_type.kind.as_str(),
            catalog_type.type_id
        )));
    }
    let new_id = TypeId::from_parts(Some(&namespace_id), &catalog_type.name);
    if new_id == catalog_type.type_id {
        return Ok(ExecutionResult::Success {
            message: format!("Type {} already in schema {}", catalog_type.type_id, namespace_id),
        });
    }
    if stores
        .get_type(&new_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .is_some()
    {
        return Err(KalamDbError::AlreadyExists(format!("type {new_id} already exists")));
    }
    let fields = stores
        .list_type_fields(&catalog_type.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let mut moved = catalog_type.clone();
    moved.type_id = new_id.clone();
    moved.namespace_id = namespace_id;
    stores
        .upsert_type(moved)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let moved_fields = fields
        .into_iter()
        .map(|mut field| {
            field.type_id = new_id.clone();
            field.type_field_id = kalamdb_commons::models::TypeFieldId::new(&new_id, &field.name)
                .map_err(KalamDbError::InvalidSql)?;
            Ok(field)
        })
        .collect::<Result<Vec<_>, KalamDbError>>()?;
    stores
        .replace_type_fields(&new_id, moved_fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    stores
        .drop_type(&catalog_type.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!("Type {} moved to {new_id}", catalog_type.type_id),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{models::NamespaceId, Role};
    use kalamdb_core::{
        sql::{context::ExecutionContext, executor::handlers::TypedStatementHandler},
        test_helpers::{create_test_session_for, test_app_context_simple},
    };
    use kalamdb_sql::ddl::{AlterTypeStatement, CreateTypeStatement};

    use super::*;
    use crate::catalog_type::CreateTypeHandler;

    fn dba_ctx(app: &Arc<kalamdb_core::app_context::AppContext>) -> ExecutionContext {
        ExecutionContext::new(
            kalamdb_commons::models::UserId::new("root"),
            Role::Dba,
            create_test_session_for(app),
        )
    }

    fn ensure_namespace(app: &kalamdb_core::app_context::AppContext, name: &str) {
        let namespaces = app.system_tables().namespaces();
        let namespace_id = NamespaceId::new(name);
        if namespaces.get_namespace(&namespace_id).unwrap().is_none() {
            namespaces.create_namespace(kalamdb_system::Namespace::new(name)).unwrap();
        }
    }

    #[tokio::test]
    async fn add_value_inserts_label_before_neighbor() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        let ctx = dba_ctx(&app);
        let create = CreateTypeHandler::new(Arc::clone(&app));
        let created = CreateTypeStatement::parse(
            "CREATE TYPE app.status AS ENUM ('active', 'blocked')",
            &NamespaceId::new("app"),
        )
        .unwrap();
        create.execute(created, vec![], &ctx).await.unwrap();

        let handler = AlterTypeHandler::new(Arc::clone(&app));
        let statement = AlterTypeStatement::parse(
            "ALTER TYPE app.status ADD VALUE 'pending' BEFORE 'active'",
            &NamespaceId::new("app"),
        )
        .unwrap();
        handler.execute(statement, vec![], &ctx).await.unwrap();

        let labels: Vec<String> = app
            .system_tables()
            .catalog_stores()
            .list_type_fields(&kalamdb_commons::models::TypeId::from_parts(
                Some(&NamespaceId::new("app")),
                "status",
            ))
            .unwrap()
            .into_iter()
            .map(|field| field.name)
            .collect();
        assert_eq!(labels, vec!["pending", "active", "blocked"]);
    }

    #[tokio::test]
    async fn add_value_if_not_exists_is_idempotent() {
        let app = test_app_context_simple();
        ensure_namespace(&app, "app");
        let ctx = dba_ctx(&app);
        let create = CreateTypeHandler::new(Arc::clone(&app));
        create
            .execute(
                CreateTypeStatement::parse(
                    "CREATE TYPE app.status AS ENUM ('active')",
                    &NamespaceId::new("app"),
                )
                .unwrap(),
                vec![],
                &ctx,
            )
            .await
            .unwrap();
        let handler = AlterTypeHandler::new(Arc::clone(&app));
        handler
            .execute(
                AlterTypeStatement::parse(
                    "ALTER TYPE app.status ADD VALUE IF NOT EXISTS 'active'",
                    &NamespaceId::new("app"),
                )
                .unwrap(),
                vec![],
                &ctx,
            )
            .await
            .unwrap();
    }
}
