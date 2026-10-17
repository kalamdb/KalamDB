//! Typed handler for ALTER TYPE.

use std::sync::Arc;

use arrow::datatypes::DataType;
use kalamdb_commons::models::{CatalogTypeKind, SchemaId, TypeId};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{AlterTypeOperation, AlterTypeStatement, EnumValueNeighbor};
use kalamdb_store::EntityStore;
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
        let (result, replicated_ids) =
            run_blocking(move || persist_alter_type(&app, statement)).await?;
        for type_id in &replicated_ids {
            super::replicate_stored_type(&self.app_context, type_id).await?;
        }
        Ok(result)
    }
}

fn persist_alter_type(
    app: &AppContext,
    statement: AlterTypeStatement,
) -> Result<(ExecutionResult, Vec<TypeId>), KalamDbError> {
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
    if catalog_type.namespace_id.is_system_namespace() {
        return Err(KalamDbError::InvalidSql(format!(
            "cannot alter built-in system type {}.{}",
            catalog_type.namespace_id, catalog_type.name
        )));
    }
    let type_id = catalog_type.type_id.clone();

    let result = match statement.operation {
        AlterTypeOperation::SetSchema { schema } => {
            crate::helpers::guards::require_existing_namespace(app, &schema)?;
            persist_set_schema(app, &stores, catalog_type, schema)
        },
        AlterTypeOperation::RenameType { new_name } => {
            persist_rename_type(app, &stores, catalog_type, new_name)
        },
        AlterTypeOperation::AddValue {
            label,
            if_not_exists,
            neighbor,
        } => persist_add_value(&stores, catalog_type, label, if_not_exists, neighbor),
        AlterTypeOperation::RenameValue { from, to } => {
            persist_rename_value(app, &stores, catalog_type, from, to)
        },
        other => persist_attribute_op(&stores, catalog_type, other),
    }?;
    let bumped_enclosing = bump_enclosing_type_revisions(&stores, &type_id)?;
    let mut related = vec![type_id.clone()];
    for id in &bumped_enclosing {
        if !related.contains(id) {
            related.push(id.clone());
        }
    }
    for id in app.schema_registry().type_registry().dependent_type_ids(&type_id) {
        if !related.contains(&id) {
            related.push(id);
        }
    }
    for id in &related {
        app.schema_registry().type_registry().invalidate(id);
    }
    app.schema_registry().invalidate_tables_using_named_types(&related);
    let mut replicated_ids = vec![type_id];
    replicated_ids.extend(bumped_enclosing);
    Ok((result, replicated_ids))
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
            let mut header = catalog_type.clone();
            header.type_revision = header.type_revision.saturating_add(1);
            stores
                .upsert_type(header)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
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
            let mut header = catalog_type.clone();
            header.type_revision = header.type_revision.saturating_add(1);
            stores
                .upsert_type(header)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
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
            let mut header = catalog_type.clone();
            header.type_revision = header.type_revision.saturating_add(1);
            stores
                .upsert_type(header)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        },
        AlterTypeOperation::SetSchema { .. }
        | AlterTypeOperation::AddValue { .. }
        | AlterTypeOperation::RenameValue { .. }
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
    let mut fields: Vec<CatalogTypeField> = stores
        .list_type_fields(&catalog_type.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .into_iter()
        .filter(|field| !field.dropped)
        .collect();
    if fields.iter().any(|existing| existing.name == label) {
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
        None => fields.len(),
        Some(EnumValueNeighbor::Before(existing)) => {
            fields.iter().position(|item| item.name == existing).ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "ENUM label '{existing}' not found on {}",
                    type_alias(&catalog_type)
                ))
            })?
        },
        Some(EnumValueNeighbor::After(existing)) => {
            fields.iter().position(|item| item.name == existing).ok_or_else(|| {
                KalamDbError::NotFound(format!(
                    "ENUM label '{existing}' not found on {}",
                    type_alias(&catalog_type)
                ))
            })? + 1
        },
    };
    let next_slot = catalog_type
        .next_slot
        .max(fields.iter().map(|field| field.physical_slot()).max().unwrap_or(0) + 1);
    let added = CatalogTypeField::enum_value_with_slot(
        catalog_type.type_id.clone(),
        label,
        next_slot,
        0,
    )
    .map_err(KalamDbError::InvalidSql)?;
    fields.insert(insert_at, added);
    for (index, field) in fields.iter_mut().enumerate() {
        field.ordinal = (index + 1) as i32;
        let slot = field.physical_slot();
        field.slot = slot;
        if let Ok(slot_id) =
            kalamdb_commons::models::TypeFieldId::from_slot(&catalog_type.type_id, slot)
        {
            field.type_field_id = slot_id;
        }
    }
    stores
        .replace_type_fields(&catalog_type.type_id, fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let alias = type_alias(&catalog_type);
    let mut header = catalog_type;
    header.next_slot = next_slot + 1;
    header.type_revision = header.type_revision.saturating_add(1);
    stores
        .upsert_type(header)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    Ok(ExecutionResult::Success {
        message: format!("Type {alias} altered"),
    })
}

fn persist_rename_value(
    app: &AppContext,
    stores: &CatalogStores,
    catalog_type: CatalogType,
    from: String,
    to: String,
) -> Result<ExecutionResult, KalamDbError> {
    if catalog_type.kind != CatalogTypeKind::Enum {
        return Err(KalamDbError::InvalidSql(format!(
            "ALTER TYPE {} RENAME VALUE requires an enum type",
            type_alias(&catalog_type)
        )));
    }
    let mut fields: Vec<CatalogTypeField> = stores
        .list_type_fields(&catalog_type.type_id)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    if !fields.iter().any(|field| field.name == from && !field.dropped) {
        return Err(KalamDbError::NotFound(format!(
            "ENUM label '{from}' not found on {}",
            type_alias(&catalog_type)
        )));
    }
    if from != to && fields.iter().any(|field| field.name == to && !field.dropped) {
        return Err(KalamDbError::AlreadyExists(format!(
            "ENUM label '{to}' already exists on {}",
            type_alias(&catalog_type)
        )));
    }
    for field in &mut fields {
        let slot = field.physical_slot();
        field.slot = slot;
        if let Ok(slot_id) =
            kalamdb_commons::models::TypeFieldId::from_slot(&catalog_type.type_id, slot)
        {
            field.type_field_id = slot_id;
        }
        if field.name == from && !field.dropped {
            field.name = to.clone();
            field.type_name = to.clone();
        }
    }
    stores
        .replace_type_fields(&catalog_type.type_id, fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    if from != to {
        rewrite_stored_enum_values(app, &catalog_type.type_id, &from, &to)?;
    }
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

fn rewrite_stored_enum_values(
    app: &AppContext,
    enum_type_id: &TypeId,
    from: &str,
    to: &str,
) -> Result<(), KalamDbError> {
    use kalamdb_commons::{
        models::TableId,
        schemas::TableType,
    };

    let tables = app
        .system_tables()
        .tables()
        .list_tables()
        .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
    for table_def in tables {
        let matching_cols: Vec<String> = table_def
            .columns
            .iter()
            .filter(|col| col.named_type_id().is_some_and(|tid| tid == enum_type_id))
            .map(|col| col.column_name.clone())
            .collect();
        if matching_cols.is_empty() {
            continue;
        }
        let Some(pk_col) = table_def
            .columns
            .iter()
            .find(|c| c.is_primary_key)
            .map(|c| c.column_name.as_str())
        else {
            continue;
        };
        let table_id = TableId::new(table_def.namespace_id.clone(), table_def.table_name.clone());
        let arrow_schema = app
            .schema_registry()
            .type_registry()
            .arrow_schema_for_table(&app.system_tables().catalog_stores(), &table_def)?;
        let storage_schema =
            kalamdb_tables::storage_schema_for_table_with_arrow(&table_def, &arrow_schema)
                .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
        let backend = app.storage_backend();

        match table_def.table_type {
            TableType::Shared => {
                let store = kalamdb_tables::new_indexed_shared_table_store(
                    backend,
                    &table_id,
                    pk_col,
                    storage_schema,
                    &table_def.scalar_indexes,
                    &table_def.columns,
                );
                let entries = store
                    .scan_all_typed(None, None, None)
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                for (key, mut row) in entries {
                    let mut changed = false;
                    for col_name in &matching_cols {
                        if let Some(val) = row.fields.values.get_mut(col_name) {
                            if rewrite_enum_scalar_in_place(val, from, to) {
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        store
                            .insert(&key, &row)
                            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                    }
                }
            },
            TableType::User => {
                let store = kalamdb_tables::new_indexed_user_table_store(
                    backend,
                    &table_id,
                    pk_col,
                    storage_schema,
                    &table_def.scalar_indexes,
                    &table_def.columns,
                );
                let entries = store
                    .scan_all_typed(None, None, None)
                    .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                for (key, mut row) in entries {
                    let mut changed = false;
                    for col_name in &matching_cols {
                        if let Some(val) = row.fields.values.get_mut(col_name) {
                            if rewrite_enum_scalar_in_place(val, from, to) {
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        store
                            .insert(&key, &row)
                            .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;
                    }
                }
            },
            TableType::Stream | TableType::System => {},
        }
    }
    Ok(())
}

fn rewrite_enum_scalar_in_place(scalar: &mut ScalarValue, from: &str, to: &str) -> bool {
    match scalar {
        ScalarValue::Utf8(Some(val)) | ScalarValue::LargeUtf8(Some(val)) if val == from => {
            *val = to.to_string();
            true
        },
        ScalarValue::List(list_arr) => {
            use arrow::array::{Array, ListArray, StringBuilder};
            let values = list_arr.values();
            let Some(str_arr) = values.as_any().downcast_ref::<arrow::array::StringArray>() else {
                return false;
            };
            let mut any_changed = false;
            for i in 0..str_arr.len() {
                if !str_arr.is_null(i) && str_arr.value(i) == from {
                    any_changed = true;
                    break;
                }
            }
            if !any_changed {
                return false;
            }
            let mut builder = StringBuilder::with_capacity(str_arr.len(), str_arr.len() * 8);
            for i in 0..str_arr.len() {
                if str_arr.is_null(i) {
                    builder.append_null();
                } else {
                    let v = str_arr.value(i);
                    builder.append_value(if v == from { to } else { v });
                }
            }
            let new_values = Arc::new(builder.finish());
            let DataType::List(item_field) = list_arr.data_type().clone() else {
                return false;
            };
            if let Ok(rebuilt) = ListArray::try_new(
                item_field,
                list_arr.offsets().clone(),
                new_values,
                list_arr.nulls().cloned(),
            ) {
                *list_arr = Arc::new(rebuilt);
                true
            } else {
                false
            }
        },
        _ => false,
    }
}

pub(super) fn persist_rename_type(
    app: &AppContext,
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
    let table_id = kalamdb_commons::models::TableId::new(
        catalog_type.namespace_id.clone(),
        kalamdb_commons::schemas::TableName::new(new_name.as_str()),
    );
    if app.schema_registry().get(&table_id).is_some() {
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

pub(super) fn persist_set_schema(
    app: &AppContext,
    stores: &CatalogStores,
    mut catalog_type: CatalogType,
    namespace_id: SchemaId,
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
    let table_id = kalamdb_commons::models::TableId::new(
        namespace_id.clone(),
        kalamdb_commons::schemas::TableName::new(catalog_type.name.as_str()),
    );
    if app.schema_registry().get(&table_id).is_some() {
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

pub(super) fn collect_enclosing_type_ids(
    stores: &CatalogStores,
    root_type_id: &TypeId,
) -> Result<Vec<TypeId>, KalamDbError> {
    let all_types = stores
        .list_types()
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    let mut type_fields_map = Vec::with_capacity(all_types.len());
    for ty in &all_types {
        let fields = stores
            .list_type_fields(&ty.type_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        type_fields_map.push((ty.clone(), fields));
    }

    let mut enclosing = Vec::new();
    let mut queue = vec![root_type_id.clone()];
    while let Some(current) = queue.pop() {
        for (ty, fields) in &type_fields_map {
            if ty.type_id == *root_type_id || enclosing.contains(&ty.type_id) {
                continue;
            }
            let references_current = ty.source_type_id.as_ref() == Some(&current)
                || fields
                    .iter()
                    .any(|f| !f.dropped && f.field_type_id.as_ref() == Some(&current));
            if references_current {
                enclosing.push(ty.type_id.clone());
                queue.push(ty.type_id.clone());
            }
        }
    }
    Ok(enclosing)
}

pub(crate) fn bump_enclosing_type_revisions(
    stores: &CatalogStores,
    root_type_id: &TypeId,
) -> Result<Vec<TypeId>, KalamDbError> {
    let enclosing = collect_enclosing_type_ids(stores, root_type_id)?;
    for enclosing_id in &enclosing {
        if let Some(mut header) = stores
            .get_type(enclosing_id)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        {
            header.type_revision = header.type_revision.saturating_add(1);
            stores
                .upsert_type(header)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
        }
    }
    Ok(enclosing)
}

fn type_has_stored_dependents(
    stores: &CatalogStores,
    type_id: &TypeId,
) -> Result<bool, KalamDbError> {
    let mut family = vec![type_id.clone()];
    family.extend(collect_enclosing_type_ids(stores, type_id)?);

    for candidate in &family {
        if let Some(catalog_type) = stores
            .get_type(candidate)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        {
            if matches!(
                catalog_type.kind,
                CatalogTypeKind::ImplicitTableRow
                    | CatalogTypeKind::RowAlias
                    | CatalogTypeKind::TopicPayload
            ) {
                return Ok(true);
            }
        }
        for routine in stores
            .list_routines()
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        {
            if routine.return_type_id.as_ref() == Some(candidate) {
                return Ok(true);
            }
        }
        for param in stores
            .list_all_parameters()
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        {
            if param.type_id.as_ref() == Some(candidate) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use kalamdb_commons::{models::SchemaId, Role};
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
        let namespace_id = SchemaId::new(name);
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
            &SchemaId::new("app"),
        )
        .unwrap();
        create.execute(created, vec![], &ctx).await.unwrap();

        let handler = AlterTypeHandler::new(Arc::clone(&app));
        let statement = AlterTypeStatement::parse(
            "ALTER TYPE app.status ADD VALUE 'pending' BEFORE 'active'",
            &SchemaId::new("app"),
        )
        .unwrap();
        handler.execute(statement, vec![], &ctx).await.unwrap();

        let type_id = app
            .system_tables()
            .catalog_stores()
            .find_type(&SchemaId::new("app"), "status")
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
                    &SchemaId::new("app"),
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
                    &SchemaId::new("app"),
                )
                .unwrap(),
                vec![],
                &ctx,
            )
            .await
            .unwrap();
    }
}
