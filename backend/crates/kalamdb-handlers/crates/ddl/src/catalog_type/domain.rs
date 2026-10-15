//! Typed handlers for `CREATE DOMAIN`, `ALTER DOMAIN`, and `DROP DOMAIN`.

use std::{collections::BTreeMap, sync::Arc};

use kalamdb_commons::{
    conversions::{
        arrow_json_conversion::validate_domain_constraints_for_field, DomainCheckSpec,
    },
    datatypes::KalamDataType,
    models::{CatalogTypeKind, TableId, TopicId, TypeId},
    schemas::{TableName, TableType},
};
use kalamdb_core::{
    app_context::AppContext,
    error::KalamDbError,
    sql::{
        context::{ExecutionContext, ExecutionResult, ScalarValue},
        executor::handlers::TypedStatementHandler,
    },
};
use kalamdb_sql::ddl::{
    AlterDomainOperation, AlterDomainStatement, CreateDomainStatement, DomainCheckClause,
    DropDomainStatement,
};
use kalamdb_store::EntityStore;
use kalamdb_system::{CatalogStores, CatalogType, CatalogTypeField};

use super::{
    alter::{bump_enclosing_type_revisions, persist_rename_type, persist_set_schema},
    create::{check_new_type_graph, require_type_reference, resolve_named_type_id},
    type_alias,
};
use crate::helpers::{
    async_blocking::run_blocking,
    guards::{require_admin, require_existing_namespace},
};

pub struct CreateDomainHandler {
    app_context: Arc<AppContext>,
}

impl CreateDomainHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<CreateDomainStatement> for CreateDomainHandler {
    async fn execute(
        &self,
        statement: CreateDomainStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "create domain")?;
        let namespace_id = statement.namespace_id.clone();
        let name = statement.name.clone();
        let app = Arc::clone(&self.app_context);
        let result = run_blocking(move || persist_create_domain(&app, statement)).await?;
        super::replicate_named_type(&self.app_context, &namespace_id, &name).await?;
        Ok(result)
    }
}

fn persist_create_domain(
    app: &AppContext,
    statement: CreateDomainStatement,
) -> Result<ExecutionResult, KalamDbError> {
    require_existing_namespace(app, &statement.namespace_id)?;
    let stores = app.system_tables().catalog_stores();
    if stores
        .find_type(&statement.namespace_id, &statement.name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .is_some()
    {
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

    require_type_reference(
        &stores,
        &statement.namespace_id,
        &statement.base_type,
        Some((&statement.namespace_id, statement.name.as_str())),
    )?;
    if let Some((base_ns, base_name)) = statement.base_type.resolved_name(&statement.namespace_id)
    {
        let base_catalog = stores
            .find_type(&base_ns, &base_name)
            .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
            .ok_or_else(|| KalamDbError::NotFound(format!("type {base_ns}.{base_name} not found")))?;
        if base_catalog.kind == CatalogTypeKind::TopicPayload {
            return Err(KalamDbError::InvalidSql(format!(
                "topic payload type {base_ns}.{base_name} cannot be used as a domain base type"
            )));
        }
    }

    let type_id = TypeId::generate();
    let checks = build_initial_domain_checks(&statement.name, &statement.checks)?;
    let mut base_field = CatalogTypeField::new(
        type_id.clone(),
        "__base".to_string(),
        1,
        resolve_named_type_id(&stores, &statement.namespace_id, &statement.base_type)?,
        statement.base_type.builtin_data_type(),
        statement.base_type.resolved_type_name(&statement.namespace_id),
        statement.base_type.is_array,
        statement.not_null,
        false,
    )
    .map_err(KalamDbError::InvalidSql)?;
    base_field.element_nullable = !statement.base_type.not_null || statement.base_type.is_array;

    let catalog_fields = build_domain_catalog_fields(
        &type_id,
        base_field,
        statement.default_expr.as_deref(),
        &checks,
    )?;
    check_new_type_graph(&stores, &type_id, &catalog_fields)?;

    let next_slot = catalog_fields
        .iter()
        .map(CatalogTypeField::physical_slot)
        .max()
        .unwrap_or(1)
        + 1;
    let mut catalog_type = CatalogType::named(
        type_id,
        statement.namespace_id.clone(),
        statement.name.clone(),
        CatalogTypeKind::Domain,
    );
    catalog_type.next_slot = next_slot;
    stores
        .upsert_type(catalog_type)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
    stores
        .publish_type_fields(catalog_fields)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;

    Ok(ExecutionResult::Success {
        message: format!("Domain {}.{} created", statement.namespace_id, statement.name),
    })
}

pub struct AlterDomainHandler {
    app_context: Arc<AppContext>,
}

impl AlterDomainHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<AlterDomainStatement> for AlterDomainHandler {
    async fn execute(
        &self,
        statement: AlterDomainStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "alter domain")?;
        let app = Arc::clone(&self.app_context);
        let (result, replicated_ids) =
            run_blocking(move || persist_alter_domain(&app, statement)).await?;
        for type_id in &replicated_ids {
            super::replicate_stored_type(&self.app_context, type_id).await?;
        }
        Ok(result)
    }
}

fn persist_alter_domain(
    app: &AppContext,
    statement: AlterDomainStatement,
) -> Result<(ExecutionResult, Vec<TypeId>), KalamDbError> {
    let stores = app.system_tables().catalog_stores();
    let catalog_type = stores
        .find_type(&statement.namespace_id, &statement.name)
        .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?
        .ok_or_else(|| {
            KalamDbError::NotFound(format!(
                "domain {}.{} not found",
                statement.namespace_id, statement.name
            ))
        })?;
    if catalog_type.kind != CatalogTypeKind::Domain {
        return Err(KalamDbError::InvalidSql(format!(
            "{}.{} is not a domain",
            statement.namespace_id, statement.name
        )));
    }
    let type_id = catalog_type.type_id.clone();

    match statement.operation {
        AlterDomainOperation::RenameTo { new_name } => {
            let result = persist_rename_type(app, &stores, catalog_type, new_name)?;
            invalidate_domain_and_dependents(app, &stores, &type_id)?;
            return Ok((result, vec![type_id]));
        },
        AlterDomainOperation::SetSchema { schema } => {
            require_existing_namespace(app, &schema)?;
            let result = persist_set_schema(app, &stores, catalog_type, schema)?;
            invalidate_domain_and_dependents(app, &stores, &type_id)?;
            return Ok((result, vec![type_id]));
        },
        AlterDomainOperation::OwnerTo { .. } => {
            return Ok((
                ExecutionResult::Success {
                    message: format!("Domain {} owner updated", type_alias(&catalog_type)),
                },
                vec![],
            ));
        },
        other => {
            let prev_fields = stores
                .list_type_fields(&type_id)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            let (mut base_field, mut default_expr, mut checks) =
                extract_domain_state(&catalog_type, &prev_fields)?;
            let mut needs_row_validation = false;

            match other {
                AlterDomainOperation::SetDefault { expr } => {
                    default_expr = Some(expr);
                },
                AlterDomainOperation::DropDefault => {
                    default_expr = None;
                },
                AlterDomainOperation::SetNotNull => {
                    base_field.not_null = true;
                    needs_row_validation = true;
                },
                AlterDomainOperation::DropNotNull => {
                    base_field.not_null = false;
                },
                AlterDomainOperation::AddNotNull {
                    constraint_name: _,
                    validated,
                } => {
                    base_field.not_null = true;
                    if validated {
                        needs_row_validation = true;
                    }
                },
                AlterDomainOperation::AddCheck { clause } => {
                    if let Some(ref explicit_name) = clause.name {
                        if checks.iter().any(|c| c.name == *explicit_name) {
                            return Err(KalamDbError::AlreadyExists(format!(
                                "constraint \"{explicit_name}\" for domain \"{}\" already exists",
                                type_alias(&catalog_type)
                            )));
                        }
                    }
                    let assigned_name = clause
                        .name
                        .unwrap_or_else(|| next_domain_check_name(&catalog_type.name, &checks));
                    checks.push(DomainCheckSpec {
                        name:      assigned_name,
                        expr:      clause.expression,
                        validated: clause.validated,
                    });
                    if clause.validated {
                        needs_row_validation = true;
                    }
                },
                AlterDomainOperation::DropConstraint {
                    name,
                    if_exists,
                    cascade: _,
                } => {
                    let before_len = checks.len();
                    checks.retain(|c| c.name != name);
                    if checks.len() == before_len {
                        let not_null_name = format!("{}_not_null", catalog_type.name);
                        if base_field.not_null
                            && (name.eq_ignore_ascii_case(&not_null_name)
                                || name.eq_ignore_ascii_case("not_null"))
                        {
                            base_field.not_null = false;
                        } else if if_exists {
                            return Ok((
                                ExecutionResult::Success {
                                    message: format!(
                                        "Constraint \"{name}\" of domain {} does not exist, \
                                         skipping",
                                        type_alias(&catalog_type)
                                    ),
                                },
                                vec![],
                            ));
                        } else {
                            return Err(KalamDbError::NotFound(format!(
                                "constraint \"{name}\" of domain \"{}\" does not exist",
                                type_alias(&catalog_type)
                            )));
                        }
                    }
                },
                AlterDomainOperation::RenameConstraint { from, to } => {
                    if checks.iter().any(|c| c.name == to) {
                        return Err(KalamDbError::AlreadyExists(format!(
                            "constraint \"{to}\" for domain \"{}\" already exists",
                            type_alias(&catalog_type)
                        )));
                    }
                    let Some(target) = checks.iter_mut().find(|c| c.name == from) else {
                        return Err(KalamDbError::NotFound(format!(
                            "constraint \"{from}\" of domain \"{}\" does not exist",
                            type_alias(&catalog_type)
                        )));
                    };
                    target.name = to;
                },
                AlterDomainOperation::ValidateConstraint { name } => {
                    let Some(target) = checks.iter_mut().find(|c| c.name == name) else {
                        return Err(KalamDbError::NotFound(format!(
                            "constraint \"{name}\" of domain \"{}\" does not exist",
                            type_alias(&catalog_type)
                        )));
                    };
                    target.validated = true;
                    needs_row_validation = true;
                },
                AlterDomainOperation::RenameTo { .. }
                | AlterDomainOperation::SetSchema { .. }
                | AlterDomainOperation::OwnerTo { .. } => unreachable!(),
            }

            let new_fields = build_domain_catalog_fields(
                &type_id,
                base_field,
                default_expr.as_deref(),
                &checks,
            )?;
            let next_slot = new_fields
                .iter()
                .map(CatalogTypeField::physical_slot)
                .max()
                .unwrap_or(1)
                + 1;
            let prev_header = catalog_type.clone();
            let mut updated_header = catalog_type.clone();
            updated_header.type_revision = updated_header.type_revision.saturating_add(1);
            updated_header.next_slot = updated_header.next_slot.max(next_slot);
            stores
                .upsert_type(updated_header)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            stores
                .replace_type_fields(&type_id, new_fields)
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;

            let bumped_enclosing = bump_enclosing_type_revisions(&stores, &type_id)?;
            let related = invalidate_domain_and_dependents(app, &stores, &type_id)?;

            if needs_row_validation {
                if let Err(err) = validate_existing_table_rows_for_domain(app, &stores, &related) {
                    let _ = stores.upsert_type(prev_header);
                    let _ = stores.replace_type_fields(&type_id, prev_fields);
                    let _ = invalidate_domain_and_dependents(app, &stores, &type_id);
                    return Err(err);
                }
            }

            let mut replicated_ids = vec![type_id];
            for id in bumped_enclosing {
                if !replicated_ids.contains(&id) {
                    replicated_ids.push(id);
                }
            }
            Ok((
                ExecutionResult::Success {
                    message: format!("Domain {} altered", type_alias(&catalog_type)),
                },
                replicated_ids,
            ))
        },
    }
}

pub struct DropDomainHandler {
    app_context: Arc<AppContext>,
}

impl DropDomainHandler {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }
}

impl TypedStatementHandler<DropDomainStatement> for DropDomainHandler {
    async fn execute(
        &self,
        statement: DropDomainStatement,
        _params: Vec<ScalarValue>,
        context: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        require_admin(context, "drop domain")?;
        if statement.cascade {
            return Err(KalamDbError::InvalidSql(
                "DROP DOMAIN CASCADE is not supported; drop dependents first or use RESTRICT"
                    .to_string(),
            ));
        }
        let app = Arc::clone(&self.app_context);
        let (result, dropped_ids) = run_blocking(move || {
            let stores = app.system_tables().catalog_stores();
            let mut to_drop = Vec::new();
            for target in &statement.targets {
                let existing = stores
                    .find_type(&target.namespace_id, &target.name)
                    .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
                let Some(catalog_type) = existing else {
                    if statement.if_exists {
                        continue;
                    }
                    return Err(KalamDbError::NotFound(format!(
                        "domain {}.{} not found",
                        target.namespace_id, target.name
                    )));
                };
                if catalog_type.kind != CatalogTypeKind::Domain {
                    return Err(KalamDbError::InvalidSql(format!(
                        "{}.{} is not a domain",
                        catalog_type.namespace_id, catalog_type.name
                    )));
                }
                to_drop.push(catalog_type);
            }
            if to_drop.is_empty() {
                return Ok((
                    ExecutionResult::Success {
                        message: "Domain does not exist, skipping".to_string(),
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
                        "cannot drop domain {}: referenced by {dependent}",
                        type_alias(catalog_type)
                    )));
                }
                for table_def in &tables {
                    for col in &table_def.columns {
                        if col.named_type_id() == Some(&catalog_type.type_id) {
                            return Err(KalamDbError::InvalidOperation(format!(
                                "cannot drop domain {}: referenced by column {}.{}.{}",
                                type_alias(catalog_type),
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
                let alias = type_alias(&catalog_type);
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
                    message: format!("Domain {} dropped", dropped_names.join(", ")),
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

fn build_initial_domain_checks(
    domain_name: &str,
    clauses: &[DomainCheckClause],
) -> Result<Vec<DomainCheckSpec>, KalamDbError> {
    let mut specs = Vec::with_capacity(clauses.len());
    for clause in clauses {
        if let Some(ref explicit_name) = clause.name {
            if specs.iter().any(|existing: &DomainCheckSpec| existing.name == *explicit_name) {
                return Err(KalamDbError::AlreadyExists(format!(
                    "constraint \"{explicit_name}\" for domain \"{domain_name}\" already exists"
                )));
            }
        }
        let assigned_name = clause
            .name
            .clone()
            .unwrap_or_else(|| next_domain_check_name(domain_name, &specs));
        specs.push(DomainCheckSpec {
            name:      assigned_name,
            expr:      clause.expression.clone(),
            validated: clause.validated,
        });
    }
    Ok(specs)
}

fn next_domain_check_name(domain_name: &str, existing: &[DomainCheckSpec]) -> String {
    let base = format!("{domain_name}_check");
    if !existing.iter().any(|c| c.name == base) {
        return base;
    }
    let mut idx = 1usize;
    loop {
        let candidate = format!("{domain_name}_check{idx}");
        if !existing.iter().any(|c| c.name == candidate) {
            return candidate;
        }
        idx += 1;
    }
}

fn build_domain_catalog_fields(
    type_id: &TypeId,
    mut base_field: CatalogTypeField,
    default_expr: Option<&str>,
    checks: &[DomainCheckSpec],
) -> Result<Vec<CatalogTypeField>, KalamDbError> {
    let mut fields = Vec::with_capacity(2 + checks.len());
    base_field.ordinal = 1;
    base_field.slot = 1;
    fields.push(base_field);

    let mut next_ordinal = 2i32;
    if let Some(expr) = default_expr.filter(|s| !s.trim().is_empty()) {
        let default_field = CatalogTypeField::new(
            type_id.clone(),
            "__default".to_string(),
            next_ordinal,
            None,
            Some(KalamDataType::Text),
            expr.to_string(),
            false,
            false,
            false,
        )
        .map_err(KalamDbError::InvalidSql)?;
        fields.push(default_field);
        next_ordinal += 1;
    }

    for check in checks {
        let check_field = CatalogTypeField::new(
            type_id.clone(),
            check.name.clone(),
            next_ordinal,
            None,
            Some(KalamDataType::Text),
            check.expr.clone(),
            false,
            check.validated,
            false,
        )
        .map_err(KalamDbError::InvalidSql)?;
        fields.push(check_field);
        next_ordinal += 1;
    }

    Ok(fields)
}

fn extract_domain_state(
    catalog_type: &CatalogType,
    fields: &[CatalogTypeField],
) -> Result<(CatalogTypeField, Option<String>, Vec<DomainCheckSpec>), KalamDbError> {
    let Some(base_field) = fields.iter().find(|f| f.name == "__base" && !f.dropped).cloned() else {
        return Err(KalamDbError::ExecutionError(format!(
            "domain {} has no __base field",
            type_alias(catalog_type)
        )));
    };
    let default_expr = fields
        .iter()
        .find(|f| f.name == "__default" && !f.dropped)
        .map(|f| f.type_name.clone());
    let checks = fields
        .iter()
        .filter(|f| f.name != "__base" && f.name != "__default" && !f.dropped)
        .map(|f| DomainCheckSpec {
            name:      f.name.clone(),
            expr:      f.type_name.clone(),
            validated: f.not_null,
        })
        .collect();
    Ok((base_field, default_expr, checks))
}

fn invalidate_domain_and_dependents(
    app: &AppContext,
    stores: &CatalogStores,
    type_id: &TypeId,
) -> Result<Vec<TypeId>, KalamDbError> {
    let mut related = vec![type_id.clone()];
    for id in super::alter::collect_enclosing_type_ids(stores, type_id)? {
        if !related.contains(&id) {
            related.push(id);
        }
    }
    for id in app.schema_registry().type_registry().dependent_type_ids(type_id) {
        if !related.contains(&id) {
            related.push(id);
        }
    }
    for id in &related {
        app.schema_registry().type_registry().invalidate(id);
    }
    app.schema_registry().invalidate_tables_using_named_types(&related);
    Ok(related)
}

fn validate_existing_table_rows_for_domain(
    app: &AppContext,
    stores: &CatalogStores,
    related_type_ids: &[TypeId],
) -> Result<(), KalamDbError> {
    let schema_registry = app.schema_registry();
    let type_registry = schema_registry.type_registry();
    let tables = app
        .system_tables()
        .tables()
        .list_tables()
        .map_err(|e| KalamDbError::ExecutionError(e.to_string()))?;

    for table_def in tables {
        let matching_columns: Vec<String> = table_def
            .columns
            .iter()
            .filter(|col| {
                col.named_type_id()
                    .is_some_and(|tid| related_type_ids.iter().any(|r| r == tid))
            })
            .map(|col| col.column_name.clone())
            .collect();
        if matching_columns.is_empty() {
            continue;
        }

        let table_id = TableId::new(table_def.namespace_id.clone(), table_def.table_name.clone());
        let arrow_schema = type_registry.arrow_schema_for_table(stores, &table_def)?;
        let Some(pk_col) = table_def
            .columns
            .iter()
            .find(|c| c.is_primary_key)
            .map(|c| c.column_name.as_str())
        else {
            continue;
        };
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
                let mut latest: BTreeMap<String, (i64, bool, kalamdb_commons::models::rows::Row)> =
                    BTreeMap::new();
                for (_key, row) in entries {
                    let pk_key = format!(
                        "{:?}",
                        row.fields.values.get(pk_col).unwrap_or(&ScalarValue::Null)
                    );
                    let ver = row._version.as_i64();
                    let replace = latest.get(&pk_key).is_none_or(|(prev_v, _, _)| ver >= *prev_v);
                    if replace {
                        latest.insert(pk_key, (ver, row._deleted, row.fields));
                    }
                }
                for (_pk, (_ver, deleted, fields)) in latest {
                    if deleted {
                        continue;
                    }
                    for col_name in &matching_columns {
                        let Ok(arrow_field) = arrow_schema.field_with_name(col_name) else {
                            continue;
                        };
                        let value = fields.values.get(col_name).unwrap_or(&ScalarValue::Null);
                        validate_domain_constraints_for_field(arrow_field, value)
                            .map_err(KalamDbError::InvalidOperation)?;
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
                let mut latest: BTreeMap<
                    (String, String),
                    (i64, bool, kalamdb_commons::models::rows::Row),
                > = BTreeMap::new();
                for (_key, row) in entries {
                    let pk_key = format!(
                        "{:?}",
                        row.fields.values.get(pk_col).unwrap_or(&ScalarValue::Null)
                    );
                    let map_key = (row.user_id.as_str().to_string(), pk_key);
                    let ver = row._version.as_i64();
                    let replace = latest.get(&map_key).is_none_or(|(prev_v, _, _)| ver >= *prev_v);
                    if replace {
                        latest.insert(map_key, (ver, row._deleted, row.fields));
                    }
                }
                for (_pk, (_ver, deleted, fields)) in latest {
                    if deleted {
                        continue;
                    }
                    for col_name in &matching_columns {
                        let Ok(arrow_field) = arrow_schema.field_with_name(col_name) else {
                            continue;
                        };
                        let value = fields.values.get(col_name).unwrap_or(&ScalarValue::Null);
                        validate_domain_constraints_for_field(arrow_field, value)
                            .map_err(KalamDbError::InvalidOperation)?;
                    }
                }
            },
            TableType::Stream | TableType::System => {},
        }
    }

    Ok(())
}
