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

use super::{
    create::{catalog_field, require_type_reference},
    type_alias,
};
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
        run_blocking(move || persist_alter_type(&app, statement)).await
    }
}

fn persist_alter_type(
    app: &AppContext,
    statement: AlterTypeStatement,
) -> Result<ExecutionResult, KalamDbError> {
    let stores = app.system_tables().catalog_stores();
    let catalog_type = stores
        .find_type(&statement.namespace_id, &statement.name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .ok_or_else(|| {
            KalamDbError::NotFound(format!(
                "type {}.{} not found",
                statement.namespace_id, statement.name
            ))
        })?;
    let type_id = catalog_type.type_id.clone();

    let result = match statement.operation {
        AlterTypeOperation::SetSchema { schema } => {
            persist_set_schema(&stores, catalog_type, schema)
        },
        AlterTypeOperation::RenameType { new_name } => {
            persist_rename_type(&stores, catalog_type, new_name)
        },
        AlterTypeOperation::AddValue {
            label,
            if_not_exists,
            neighbor,
        } => persist_add_value(&stores, catalog_type, label, if_not_exists, neighbor),
        other => persist_attribute_op(&stores, catalog_type, other),
    }?;
    let mut related = vec![type_id.clone()];
    related.extend(app.schema_registry().type_registry().dependent_type_ids(&type_id));
    app.schema_registry().type_registry().invalidate(&type_id);
    app.schema_registry().invalidate_tables_using_named_types(&related);
    Ok(result)
}

fn persist_attribute_op(
    stores: &CatalogStores,
    catalog_type: CatalogType,
    operation: AlterTypeOperation,
) -> Result<ExecutionResult, KalamDbError> {
    if catalog_type.kind != CatalogTypeKind::Composite {
        return Err(KalamDbError::InvalidSql(format!(
            "ALTER TYPE {} attribute operations require a composite type",
            type_alias(&catalog_type)
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
                Some((&catalog_type.namespace_id, catalog_type.name.as_str())),
            )?;
            if fields.iter().any(|existing| existing.name == field && !existing.dropped) {
                return Err(KalamDbError::AlreadyExists(format!(
                    "attribute {field} already exists on {}",
                    type_alias(&catalog_type)
                )));
            }
            if type_ref.not_null && type_has_stored_dependents(stores, &catalog_type.type_id)? {
                return Err(KalamDbError::InvalidSql(
                    "cannot ADD a NOT NULL attribute while the type is stored in dependents; add \
                     a nullable attribute"
                        .to_string(),
                ));
            }
            let slot = catalog_type
                .next_slot
                .max(fields.iter().map(|existing| existing.physical_slot()).max().unwrap_or(0) + 1);
            let mut added = catalog_field(
                stores,
                &catalog_type.type_id,
                &catalog_type.namespace_id,
                field,
                type_ref,
                slot,
            )?;
            added.slot = slot;
            added.ordinal =
                fields.iter().filter(|f| !f.dropped).map(|f| f.ordinal).max().unwrap_or(0) + 1;
            fields.push(added);
            let mut header = catalog_type.clone();
            header.next_slot = slot + 1;
            header.type_revision = header.type_revision.saturating_add(1);
            stores
                .upsert_type(header)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        },
        AlterTypeOperation::DropAttribute { field, cascade } => {
            if cascade {
                return Err(KalamDbError::InvalidSql(
                    "DROP ATTRIBUTE CASCADE is not supported; drop dependents first".to_string(),
                ));
            }
            if type_has_stored_dependents(stores, &catalog_type.type_id)? {
                return Err(KalamDbError::InvalidSql(
                    "cannot DROP ATTRIBUTE while the type is referenced by stored dependents"
                        .to_string(),
                ));
            }
            let Some(existing) = fields.iter_mut().find(|item| item.name == field && !item.dropped)
            else {
                return Err(KalamDbError::NotFound(format!(
                    "attribute {field} not found on {}",
                    type_alias(&catalog_type)
                )));
            };
            existing.dropped = true;
        },
        AlterTypeOperation::RenameAttribute { from, to } => {
            if type_has_stored_dependents(stores, &catalog_type.type_id)? {
                return Err(KalamDbError::InvalidSql(
                    "cannot RENAME ATTRIBUTE while the type is referenced by stored dependents"
                        .to_string(),
                ));
            }
            if !fields.iter().any(|field| field.name == from && !field.dropped) {
                return Err(KalamDbError::NotFound(format!(
                    "attribute {from} not found on {}",
                    type_alias(&catalog_type)
                )));
            }
            if fields.iter().any(|field| field.name == to && !field.dropped) {
                return Err(KalamDbError::AlreadyExists(format!(
                    "attribute {to} already exists on {}",
                    type_alias(&catalog_type)
                )));
            }
            for existing in &mut fields {
                if existing.name == from && !existing.dropped {
                    existing.name = to;
                    break;
                }
            }
        },
        AlterTypeOperation::AlterAttributeType { field, type_ref } => {
            if type_has_stored_dependents(stores, &catalog_type.type_id)? {
                return Err(KalamDbError::InvalidSql(
                    "cannot ALTER ATTRIBUTE TYPE while the type is referenced by stored dependents"
                        .to_string(),
                ));
            }
            require_type_reference(
                stores,
                &catalog_type.namespace_id,
                &type_ref,
                Some((&catalog_type.namespace_id, catalog_type.name.as_str())),
            )?;
            let Some(existing) = fields.iter_mut().find(|item| item.name == field && !item.dropped)
            else {
                return Err(KalamDbError::NotFound(format!(
                    "attribute {field} not found on {}",
                    type_alias(&catalog_type)
                )));
            };
            let mut rebuilt = catalog_field(
                stores,
                &catalog_type.type_id,
                &catalog_type.namespace_id,
                field,
                type_ref,
                existing.physical_slot(),
            )?;
            rebuilt.slot = existing.physical_slot();
            rebuilt.ordinal = existing.ordinal;
            rebuilt.type_field_id = existing.type_field_id.clone();
            *existing = rebuilt;
        },
        AlterTypeOperation::SetSchema { .. }
        | AlterTypeOperation::AddValue { .. }
        | AlterTypeOperation::RenameType { .. } => {
            unreachable!("handled separately")
        },
    }
    super::create::check_new_type_graph(stores, &catalog_type.type_id, &fields)?;
    stores
        .publish_type_fields(fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!("Type {} altered", type_alias(&catalog_type)),
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
            type_alias(&catalog_type)
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
                    type_alias(&catalog_type)
                ),
            });
        }
        return Err(KalamDbError::AlreadyExists(format!(
            "ENUM label '{label}' already exists on {}",
            type_alias(&catalog_type)
        )));
    }
    let insert_at = match neighbor {
        None => labels.len(),
        Some(EnumValueNeighbor::Before(existing)) => {
            labels.iter().position(|item| item == &existing).ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "ENUM label '{existing}' not found on {}",
                    type_alias(&catalog_type)
                ))
            })?
        },
        Some(EnumValueNeighbor::After(existing)) => {
            labels.iter().position(|item| item == &existing).ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "ENUM label '{existing}' not found on {}",
                    type_alias(&catalog_type)
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
        .publish_type_fields(catalog_fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let alias = type_alias(&catalog_type);
    let mut header = catalog_type;
    header.type_revision = header.type_revision.saturating_add(1);
    stores
        .upsert_type(header)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!("Type {alias} altered"),
    })
}

fn persist_rename_type(
    stores: &CatalogStores,
    mut catalog_type: CatalogType,
    new_name: String,
) -> Result<ExecutionResult, KalamDbError> {
    reject_implicit_rename(&catalog_type)?;
    if new_name == catalog_type.name {
        return Ok(ExecutionResult::Success {
            message: format!("Type {} already named {new_name}", type_alias(&catalog_type)),
        });
    }
    if stores
        .find_type(&catalog_type.namespace_id, &new_name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .is_some()
    {
        return Err(KalamDbError::AlreadyExists(format!(
            "type {}.{} already exists",
            catalog_type.namespace_id, new_name
        )));
    }
    let previous_name = catalog_type.name.clone();
    catalog_type.name = new_name;
    stores
        .upsert_type(catalog_type.clone())
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!(
            "Type {}.{} renamed to {}",
            catalog_type.namespace_id, previous_name, catalog_type.name
        ),
    })
}

fn persist_set_schema(
    stores: &CatalogStores,
    mut catalog_type: CatalogType,
    namespace_id: NamespaceId,
) -> Result<ExecutionResult, KalamDbError> {
    reject_implicit_rename(&catalog_type)?;
    if namespace_id == catalog_type.namespace_id {
        return Ok(ExecutionResult::Success {
            message: format!(
                "Type {} already in schema {}",
                type_alias(&catalog_type),
                namespace_id
            ),
        });
    }
    if stores
        .find_type(&namespace_id, &catalog_type.name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .is_some()
    {
        return Err(KalamDbError::AlreadyExists(format!(
            "type {}.{} already exists",
            namespace_id, catalog_type.name
        )));
    }
    let previous = type_alias(&catalog_type);
    catalog_type.namespace_id = namespace_id;
    stores
        .upsert_type(catalog_type.clone())
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!(
            "Type {previous} moved to {}.{}",
            catalog_type.namespace_id, catalog_type.name
        ),
    })
}

fn reject_implicit_rename(catalog_type: &CatalogType) -> Result<(), KalamDbError> {
    if matches!(
        catalog_type.kind,
        CatalogTypeKind::ImplicitTableRow
            | CatalogTypeKind::RowAlias
            | CatalogTypeKind::TopicPayload
    ) {
        return Err(KalamDbError::InvalidSql(format!(
            "cannot rename or SET SCHEMA on {} type {}",
            catalog_type.kind.as_str(),
            type_alias(catalog_type)
        )));
    }
    Ok(())
}

fn type_has_stored_dependents(
    stores: &CatalogStores,
    type_id: &TypeId,
) -> Result<bool, KalamDbError> {
    Ok(stores
        .type_is_referenced(type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .is_some())
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

        let type_id = app
            .system_tables()
            .catalog_stores()
            .find_type(&NamespaceId::new("app"), "status")
            .unwrap()
            .expect("created enum")
            .type_id;
        let labels: Vec<String> = app
            .system_tables()
            .catalog_stores()
            .list_type_fields(&type_id)
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
