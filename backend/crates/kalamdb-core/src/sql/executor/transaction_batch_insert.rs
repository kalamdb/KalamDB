use std::{
    collections::{btree_map::Entry, BTreeMap},
    sync::{Arc, OnceLock},
};

use arrow::{
    array::{new_empty_array, ListArray, StructArray},
    buffer::OffsetBuffer,
    datatypes::{DataType, Field, SchemaRef},
};
use chrono::Utc;
use datafusion::scalar::ScalarValue;
use kalamdb_commons::{
    conversions::{
        arrow_json_conversion::{
            coerce_rows, coerce_scalar_to_field, materialize_domain_default_for_field,
        },
        json_value_to_scalar,
        schema_metadata::read_kalam_data_type_metadata,
    },
    ids::SnowflakeGenerator,
    models::{datatypes::KalamDataType, rows::row::Row, OperationKind, TransactionId, UserId},
    schemas::{ColumnDefault, TableType},
    PolicyCommand, Role, TableId,
};
use kalamdb_sql::{
    expr_to_scalar_with_params, insert_columns_match, is_default_expr,
    on_conflict_update_should_apply, on_conflict_values_insert,
    parse_on_conflict_action_with_params, validate_primary_key_conflict_target,
    OnConflictUpdateAssignment, OnConflictUpdateValue, ParsedOnConflictAction,
    ValuesInsertShapeOptions, ValuesInsertView,
};
use kalamdb_tables::SharedTableProvider;
use kalamdb_transactions::{build_insert_staged_mutations, StagedMutation};
use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, ObjectName, ObjectNamePart, Parens,
    SelectItem, Statement, UnaryOperator, Value as SqlValue,
};
use ulid::Ulid;
use uuid::Uuid;

use crate::{
    app_context::AppContext,
    error::KalamDbError,
    functions::FunctionService,
    schema_registry::CachedTableData,
    sql::{
        plan_cache::{
            FastInsertDefaultEntry, FastInsertDefaultTemplate, FastInsertMetadata,
            InsertMetadataCacheKey, SqlCacheRegistry,
        },
        ExecutionContext,
    },
};

static INSERT_DEFAULT_SNOWFLAKE_GENERATOR: OnceLock<SnowflakeGenerator> = OnceLock::new();

fn insert_default_snowflake_generator() -> &'static SnowflakeGenerator {
    INSERT_DEFAULT_SNOWFLAKE_GENERATOR.get_or_init(|| SnowflakeGenerator::new(0))
}

fn build_insert_metadata(
    requested_columns: &[String],
    cached_table: &CachedTableData,
) -> Result<FastInsertMetadata, KalamDbError> {
    let available_columns: Vec<&str> = cached_table
        .table
        .columns
        .iter()
        .filter(|column| !column.column_name.starts_with('_'))
        .map(|column| column.column_name.as_str())
        .collect();

    let column_names = if requested_columns.is_empty() {
        available_columns.iter().map(|column| (*column).to_string()).collect()
    } else {
        for column_name in requested_columns {
            if !available_columns.iter().any(|candidate| *candidate == column_name.as_str()) {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Column '{}' does not exist",
                    column_name
                )));
            }
        }
        requested_columns.to_vec()
    };

    let missing_defaults = cached_table
        .table
        .columns
        .iter()
        .filter(|column| !column.column_name.starts_with('_'))
        .filter(|column| !column_names.iter().any(|name| name == &column.column_name))
        .filter(|column| !column.default_value.is_none())
        .map(|column| {
            Ok(FastInsertDefaultEntry::new(
                column.column_name.clone(),
                prepare_default_template(&column.default_value)?,
            ))
        })
        .collect::<Result<Vec<_>, KalamDbError>>()?;

    let pk_columns = cached_table.table.get_primary_key_columns();

    Ok(FastInsertMetadata {
        table_type: cached_table.table.table_type.into(),
        column_names,
        missing_defaults,
        primary_key_column: if pk_columns.len() == 1 {
            Some(pk_columns[0].to_string())
        } else {
            None
        },
    })
}

pub(crate) fn staged_mutation_user_id(
    table_type: TableType,
    exec_ctx: &ExecutionContext,
) -> Option<UserId> {
    match table_type {
        TableType::User | TableType::Stream => Some(exec_ctx.user_id().clone()),
        TableType::Shared | TableType::System => None,
    }
}

enum PreparedDefaultValue {
    Constant(ScalarValue),
    Volatile(VolatileDefaultFunction),
    Procedure(kalamdb_commons::RoutineCall),
}

enum VolatileDefaultFunction {
    SnowflakeId,
    UuidV7,
    Ulid,
}

enum InsertValuesRowsError {
    Unsupported,
    Execution(KalamDbError),
}

impl From<KalamDbError> for InsertValuesRowsError {
    fn from(error: KalamDbError) -> Self {
        Self::Execution(error)
    }
}

async fn apply_missing_defaults(
    rows: &mut [Row],
    missing_defaults: &[FastInsertDefaultEntry],
    exec_ctx: &ExecutionContext,
    app_context: Arc<AppContext>,
) -> Result<(), KalamDbError> {
    if rows.is_empty() || missing_defaults.is_empty() {
        return Ok(());
    }

    let mut prepared_defaults = Vec::with_capacity(missing_defaults.len());
    for default_entry in missing_defaults {
        prepared_defaults.push((
            default_entry.column_name.clone(),
            prepare_statement_default(&default_entry.template, exec_ctx)?,
        ));
    }

    for row in rows.iter_mut() {
        for (col_name, prepared_default) in &prepared_defaults {
            let scalar =
                materialize_prepared_default(prepared_default, exec_ctx, &app_context).await?;
            row.values.insert(col_name.clone(), scalar);
        }
    }

    Ok(())
}

async fn try_build_insert_rows_from_values_rows(
    value_rows: &[Parens<Vec<Expr>>],
    insert_metadata: &FastInsertMetadata,
    cached_table: &CachedTableData,
    schema: &SchemaRef,
    exec_ctx: &ExecutionContext,
    params: &[ScalarValue],
    app_context: Arc<AppContext>,
) -> Result<Option<Vec<Row>>, KalamDbError> {
    let mut rows = match build_insert_rows_from_values_rows(
        value_rows,
        &insert_metadata.column_names,
        cached_table,
        schema,
        exec_ctx,
        params,
        Arc::clone(&app_context),
    )
    .await
    {
        Ok(rows) => rows,
        Err(InsertValuesRowsError::Unsupported) => return Ok(None),
        Err(InsertValuesRowsError::Execution(error)) => return Err(error),
    };

    if !insert_metadata.missing_defaults.is_empty() {
        apply_missing_defaults(&mut rows, &insert_metadata.missing_defaults, exec_ctx, app_context)
            .await?;
    }

    Ok(Some(rows))
}

#[inline]
fn is_file_put_function_name(name: &ObjectName) -> bool {
    match name.0.as_slice() {
        [ObjectNamePart::Identifier(ident)] => ident.value.eq_ignore_ascii_case("file_put"),
        [ObjectNamePart::Identifier(schema), ObjectNamePart::Identifier(ident)] => {
            schema.value.eq_ignore_ascii_case("pg_catalog")
                && ident.value.eq_ignore_ascii_case("file_put")
        },
        _ => false,
    }
}

#[inline]
fn is_row_function_name(name: &ObjectName) -> bool {
    matches!(
        name.0.as_slice(),
        [ObjectNamePart::Identifier(ident)] if ident.value.eq_ignore_ascii_case("row")
    )
}

async fn build_insert_rows_from_values_rows(
    value_rows: &[Parens<Vec<Expr>>],
    column_names: &[String],
    cached_table: &CachedTableData,
    schema: &SchemaRef,
    exec_ctx: &ExecutionContext,
    params: &[ScalarValue],
    app_context: Arc<AppContext>,
) -> Result<Vec<Row>, InsertValuesRowsError> {
    let mut rows = Vec::with_capacity(value_rows.len());
    let mut explicit_default_values = BTreeMap::new();
    let target_fields: Vec<Option<&Field>> =
        column_names.iter().map(|col| schema.field_with_name(col).ok()).collect();

    for value_row in value_rows {
        if value_row.content.len() != column_names.len() {
            return Err(InsertValuesRowsError::Unsupported);
        }

        let mut values = BTreeMap::new();
        for ((expr, column_name), target_field) in value_row
            .content
            .iter()
            .zip(column_names.iter())
            .zip(target_fields.iter().copied())
        {
            values.insert(
                column_name.clone(),
                insert_value_expr_to_scalar(
                    expr,
                    column_name,
                    target_field,
                    cached_table,
                    exec_ctx,
                    params,
                    &mut explicit_default_values,
                    &app_context,
                )
                .await?,
            );
        }
        rows.push(Row::new(values));
    }

    Ok(rows)
}

async fn insert_value_expr_to_scalar(
    expr: &Expr,
    column_name: &str,
    target_field: Option<&Field>,
    cached_table: &CachedTableData,
    exec_ctx: &ExecutionContext,
    params: &[ScalarValue],
    explicit_default_values: &mut BTreeMap<String, PreparedDefaultValue>,
    app_context: &Arc<AppContext>,
) -> Result<ScalarValue, InsertValuesRowsError> {
    if is_default_expr(expr) {
        return materialize_explicit_default(
            column_name,
            cached_table,
            exec_ctx,
            explicit_default_values,
            app_context,
        )
        .await
        .map_err(InsertValuesRowsError::Execution);
    }

    if let Expr::Function(func) = expr {
        if is_file_put_function_name(&func.name) {
            let arg_expr = match &func.args {
                FunctionArguments::List(list) if list.args.len() == 1 => match &list.args[0] {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(inner)) => inner,
                    _ => return Err(InsertValuesRowsError::Unsupported),
                },
                _ => {
                    return Err(InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(
                        "file_put expects a single placeholder argument".to_string(),
                    )));
                },
            };
            let placeholder_scalar = expr_to_scalar_with_params(arg_expr, params)
                .map_err(|_| InsertValuesRowsError::Unsupported)?;
            let placeholder = match placeholder_scalar {
                ScalarValue::Utf8(Some(s))
                | ScalarValue::LargeUtf8(Some(s))
                | ScalarValue::Utf8View(Some(s)) => s,
                _ => {
                    return Err(InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(
                        "file_put argument must be a non-null string placeholder".to_string(),
                    )));
                },
            };
            let file_ref = app_context
                .file_storage_service()
                .resolve_placeholder(&placeholder)
                .map_err(|e| {
                    InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(e.to_string()))
                })?;
            return Ok(ScalarValue::Utf8(Some(file_ref.to_json())));
        }

        if app_context.system_tables().catalog_stores().has_user_routines() {
            if let Ok(call) =
                kalamdb_sql::ddl::column_default::function_to_routine_call_with_placeholders(
                    func,
                    &exec_ctx.default_namespace(),
                )
            {
                let stores = app_context.system_tables().catalog_stores();
                if let Ok(Some(routine)) = stores.get_routine(&call.routine_id) {
                    if routine.language.as_deref() != Some("INTERNAL") {
                        if routine.kind == kalamdb_commons::models::RoutineKind::Procedure {
                            return Err(InsertValuesRowsError::Execution(
                                KalamDbError::InvalidSql(format!(
                                    "{} is a procedure, not a function; invoke it with CALL",
                                    call.routine_id
                                )),
                            ));
                        }
                        let rv = crate::functions::FunctionService::execute_scalar_function_call(
                            Arc::clone(app_context),
                            exec_ctx,
                            &call,
                            params,
                        )
                        .await
                        .map_err(InsertValuesRowsError::Execution)?;
                        if let Some(field) = target_field {
                            return coerce_scalar_to_field(rv.value, field).map_err(|error| {
                                InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(
                                    error,
                                ))
                            });
                        }
                        return Ok(rv.value);
                    }
                }
            }
        }
    }

    eval_typed_insert_expr(expr, target_field, params)
}

fn validate_file_columns_in_rows(
    rows: &[Row],
    cached_table: &CachedTableData,
    table_id: &TableId,
    user_id: Option<&UserId>,
    app_context: &Arc<AppContext>,
) -> Result<(), KalamDbError> {
    crate::applier::executor::dml::validate_and_commit_file_columns_for_rows(
        app_context,
        cached_table,
        table_id,
        cached_table.table_entry().table_type,
        user_id,
        rows,
    )
    .map_err(KalamDbError::InvalidOperation)
}

fn eval_typed_insert_expr(
    expr: &Expr,
    target_field: Option<&Field>,
    params: &[ScalarValue],
) -> Result<ScalarValue, InsertValuesRowsError> {
    match expr {
        Expr::Nested(inner) => eval_typed_insert_expr(inner, target_field, params),
        Expr::Cast { expr: inner, .. } => {
            let Some(field) = target_field else {
                return Err(InsertValuesRowsError::Unsupported);
            };
            let scalar = eval_typed_insert_expr(inner, Some(field), params)?;
            coerce_scalar_to_field(scalar, field).map_err(|error| {
                InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(error))
            })
        },
        Expr::TypedString(typed) => {
            let scalar = expr_to_scalar_with_params(&Expr::Value(typed.value.clone()), params)
                .map_err(|_| InsertValuesRowsError::Unsupported)?;
            if let Some(field) = target_field {
                coerce_scalar_to_field(scalar, field).map_err(|error| {
                    InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(error))
                })
            } else {
                Ok(scalar)
            }
        },
        Expr::Value(val)
            if matches!(target_field.map(Field::data_type), Some(DataType::Decimal128(_, _))) =>
        {
            if let SqlValue::Number(num_str, _) = &val.value {
                return Ok(ScalarValue::Utf8(Some(num_str.clone())));
            }
            expr_to_scalar_with_params(expr, params).map_err(|_| InsertValuesRowsError::Unsupported)
        },
        Expr::UnaryOp { op, expr: inner }
            if matches!(op, UnaryOperator::Minus | UnaryOperator::Plus)
                && matches!(
                    target_field.map(Field::data_type),
                    Some(DataType::Decimal128(_, _))
                ) =>
        {
            if let Expr::Value(val) = inner.as_ref() {
                if let SqlValue::Number(num_str, _) = &val.value {
                    let signed = if matches!(op, UnaryOperator::Minus) {
                        format!("-{num_str}")
                    } else {
                        num_str.clone()
                    };
                    return Ok(ScalarValue::Utf8(Some(signed)));
                }
            }
            expr_to_scalar_with_params(expr, params).map_err(|_| InsertValuesRowsError::Unsupported)
        },
        Expr::Array(array) => {
            let Some(field) = target_field else {
                return Err(InsertValuesRowsError::Unsupported);
            };
            let DataType::List(item_field) = field.data_type() else {
                return Err(InsertValuesRowsError::Unsupported);
            };
            let mut items = Vec::with_capacity(array.elem.len());
            for elem_expr in &array.elem {
                let elem_scalar =
                    eval_typed_insert_expr(elem_expr, Some(item_field.as_ref()), params)?;
                if elem_scalar.is_null() && !item_field.is_nullable() {
                    return Err(InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(
                        format!("array element in column '{}' cannot be null", field.name()),
                    )));
                }
                let coerced = coerce_scalar_to_field(elem_scalar, item_field).map_err(|error| {
                    InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(error))
                })?;
                items.push(coerced);
            }
            let child_array = if items.is_empty() {
                new_empty_array(item_field.data_type())
            } else if matches!(item_field.data_type(), DataType::Struct(fields) if fields.is_empty())
            {
                let DataType::Struct(fields) = item_field.data_type() else {
                    unreachable!();
                };
                let len = items.len();
                let validity: Vec<bool> = items.iter().map(|v| !v.is_null()).collect();
                let nulls = if validity.iter().all(|valid| *valid) {
                    None
                } else {
                    Some(arrow::buffer::NullBuffer::from(validity))
                };
                let sa = StructArray::try_new_with_length(fields.clone(), Vec::new(), nulls, len)
                    .map_err(|error| {
                    InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(
                        error.to_string(),
                    ))
                })?;
                Arc::new(sa)
            } else {
                ScalarValue::iter_to_array(items).map_err(|error| {
                    InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(
                        error.to_string(),
                    ))
                })?
            };
            let offsets = OffsetBuffer::from_lengths([child_array.len()]);
            Ok(ScalarValue::List(Arc::new(ListArray::new(
                item_field.clone(),
                offsets,
                child_array,
                None,
            ))))
        },
        Expr::Tuple(elems) => eval_struct_tuple_expr(elems, target_field, params),
        Expr::Function(func) if is_row_function_name(&func.name) => {
            let exprs: Vec<Expr> = match &func.args {
                FunctionArguments::None => Vec::new(),
                FunctionArguments::Subquery(_) => return Err(InsertValuesRowsError::Unsupported),
                FunctionArguments::List(list) => {
                    let mut out = Vec::with_capacity(list.args.len());
                    for arg in &list.args {
                        match arg {
                            FunctionArg::Unnamed(FunctionArgExpr::Expr(e)) => out.push(e.clone()),
                            _ => return Err(InsertValuesRowsError::Unsupported),
                        }
                    }
                    out
                },
            };
            eval_struct_tuple_expr(&exprs, target_field, params)
        },
        _ => {
            expr_to_scalar_with_params(expr, params).map_err(|_| InsertValuesRowsError::Unsupported)
        },
    }
}

fn eval_struct_tuple_expr(
    elems: &[Expr],
    target_field: Option<&Field>,
    params: &[ScalarValue],
) -> Result<ScalarValue, InsertValuesRowsError> {
    let Some(field) = target_field else {
        return Err(InsertValuesRowsError::Unsupported);
    };
    if read_kalam_data_type_metadata(field) == Some(KalamDataType::File) {
        return Err(InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(format!(
            "column '{}' is of type pg_catalog.file and cannot be constructed from a composite \
             literal; use file_put()",
            field.name()
        ))));
    }
    let DataType::Struct(fields) = field.data_type() else {
        return Err(InsertValuesRowsError::Unsupported);
    };
    if elems.len() != fields.len() {
        return Err(InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(format!(
            "composite value for column '{}' has {} fields, expected {}",
            field.name(),
            elems.len(),
            fields.len()
        ))));
    }
    let mut child_arrays = Vec::with_capacity(fields.len());
    for (elem_expr, child_field) in elems.iter().zip(fields.iter()) {
        let child_scalar = eval_typed_insert_expr(elem_expr, Some(child_field.as_ref()), params)?;
        if child_scalar.is_null() && !child_field.is_nullable() {
            return Err(InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(format!(
                "field '{}' of composite column '{}' cannot be null",
                child_field.name(),
                field.name()
            ))));
        }
        let coerced = coerce_scalar_to_field(child_scalar, child_field).map_err(|error| {
            InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(error))
        })?;
        let arr = coerced.to_array().map_err(|error| {
            InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(error.to_string()))
        })?;
        child_arrays.push(arr);
    }
    let struct_array = StructArray::try_new_with_length(fields.clone(), child_arrays, None, 1)
        .map_err(|error| {
            InsertValuesRowsError::Execution(KalamDbError::InvalidOperation(error.to_string()))
        })?;
    Ok(ScalarValue::Struct(Arc::new(struct_array)))
}

async fn materialize_explicit_default(
    column_name: &str,
    cached_table: &CachedTableData,
    exec_ctx: &ExecutionContext,
    explicit_default_values: &mut BTreeMap<String, PreparedDefaultValue>,
    app_context: &Arc<AppContext>,
) -> Result<ScalarValue, KalamDbError> {
    let prepared_default = match explicit_default_values.entry(column_name.to_string()) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            entry.insert(prepare_explicit_default(column_name, cached_table, exec_ctx)?)
        },
    };
    materialize_prepared_default(prepared_default, exec_ctx, app_context).await
}

fn prepare_explicit_default(
    column_name: &str,
    cached_table: &CachedTableData,
    exec_ctx: &ExecutionContext,
) -> Result<PreparedDefaultValue, KalamDbError> {
    let column = cached_table
        .table
        .columns
        .iter()
        .find(|column| column.column_name.eq_ignore_ascii_case(column_name))
        .ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Column '{}' does not exist", column_name))
        })?;

    if column.default_value.is_none() {
        if let Ok(schema) = cached_table.arrow_schema() {
            if let Ok(field) = schema.field_with_name(&column.column_name) {
                if let Some(domain_default) = materialize_domain_default_for_field(field)
                    .map_err(KalamDbError::InvalidOperation)?
                {
                    return Ok(PreparedDefaultValue::Constant(domain_default));
                }
            }
        }
        return Ok(PreparedDefaultValue::Constant(ScalarValue::Null));
    }

    let template = prepare_default_template(&column.default_value)?;
    prepare_statement_default(&template, exec_ctx)
}

pub(crate) struct LiteralInsertRows {
    pub table_type:         TableType,
    pub primary_key_column: Option<String>,
    pub rows:               Vec<Row>,
    pub returning:          Option<Vec<SelectItem>>,
}

pub(crate) struct LiteralOnConflictUpdateRows {
    pub table_type:         TableType,
    pub primary_key_column: String,
    pub rows:               Vec<Row>,
    pub action:             ParsedOnConflictAction,
    pub returning:          Option<Vec<SelectItem>>,
}

pub(crate) struct OnConflictStagedMutation {
    pub mutation:     StagedMutation,
    pub returned_row: Row,
}

pub(crate) async fn try_build_literal_insert_rows(
    statement: &Statement,
    app_context: Arc<AppContext>,
    sql_cache_registry: &SqlCacheRegistry,
    exec_ctx: &ExecutionContext,
    table_id: &TableId,
    params: &[ScalarValue],
) -> Result<Option<LiteralInsertRows>, KalamDbError> {
    try_build_literal_insert_rows_inner(
        statement,
        app_context,
        sql_cache_registry,
        exec_ctx,
        table_id,
        false,
        params,
    )
    .await
}

async fn try_build_literal_insert_rows_inner(
    statement: &Statement,
    app_context: Arc<AppContext>,
    sql_cache_registry: &SqlCacheRegistry,
    exec_ctx: &ExecutionContext,
    table_id: &TableId,
    allow_on_conflict: bool,
    params: &[ScalarValue],
) -> Result<Option<LiteralInsertRows>, KalamDbError> {
    if table_id.namespace_id().is_system_namespace() {
        return Ok(None);
    }

    let shape_options = if allow_on_conflict {
        ValuesInsertShapeOptions::ON_CONFLICT_ROWS
    } else {
        ValuesInsertShapeOptions::PLAIN
    };
    let Some(view) = kalamdb_sql::values_insert_view_from_statement(statement, shape_options)
    else {
        return Ok(None);
    };

    build_literal_insert_rows_from_values(
        view,
        app_context,
        sql_cache_registry,
        exec_ctx,
        table_id,
        !allow_on_conflict,
        params,
    )
    .await
}

async fn build_literal_insert_rows_from_values(
    view: ValuesInsertView<'_>,
    app_context: Arc<AppContext>,
    sql_cache_registry: &SqlCacheRegistry,
    exec_ctx: &ExecutionContext,
    table_id: &TableId,
    include_returning: bool,
    params: &[ScalarValue],
) -> Result<Option<LiteralInsertRows>, KalamDbError> {
    let insert = view.insert;
    let value_rows = view.value_rows;

    let cached_table = match app_context.schema_registry().get(table_id) {
        Some(cached) => cached,
        None => return Ok(None),
    };

    let cached_table_entry = cached_table.table_entry();
    if cached_table_entry.table_type == TableType::Shared
        && matches!(exec_ctx.user_role(), Role::Anonymous)
    {
        return Err(KalamDbError::PermissionDenied(
            "Anonymous shared-table writes are denied".to_string(),
        ));
    }

    let requested_columns: Vec<String> =
        insert.columns.iter().filter_map(kalamdb_sql::object_name_to_string).collect();
    let metadata_cache_key =
        InsertMetadataCacheKey::new(table_id.clone(), requested_columns.clone());
    let insert_metadata = match sql_cache_registry.insert_metadata_cache().get(&metadata_cache_key)
    {
        Some(metadata) => metadata,
        None => {
            let metadata =
                Arc::new(build_insert_metadata(&requested_columns, cached_table.as_ref())?);
            sql_cache_registry
                .insert_metadata_cache()
                .insert_arc(metadata_cache_key, Arc::clone(&metadata));
            metadata
        },
    };

    let schema = cached_table.arrow_schema()?;
    let rows = match try_build_insert_rows_from_values_rows(
        value_rows,
        &insert_metadata,
        cached_table.as_ref(),
        &schema,
        exec_ctx,
        params,
        Arc::clone(&app_context),
    )
    .await?
    {
        Some(rows) => rows,
        None => return Ok(None),
    };

    let rows = coerce_rows(rows, &schema)
        .map_err(|e| KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e)))?;
    validate_file_columns_in_rows(
        &rows,
        cached_table.as_ref(),
        table_id,
        staged_mutation_user_id(insert_metadata.table_type, exec_ctx).as_ref(),
        &app_context,
    )?;

    Ok(Some(LiteralInsertRows {
        table_type: insert_metadata.table_type,
        primary_key_column: insert_metadata.primary_key_column.clone(),
        rows,
        returning: if include_returning {
            insert.returning.clone()
        } else {
            None
        },
    }))
}

pub(crate) async fn try_build_literal_on_conflict_update_rows(
    statement: &Statement,
    app_context: Arc<AppContext>,
    sql_cache_registry: &SqlCacheRegistry,
    exec_ctx: &ExecutionContext,
    table_id: &TableId,
    params: &[ScalarValue],
) -> Result<Option<LiteralOnConflictUpdateRows>, KalamDbError> {
    let Some((view, on_conflict)) = on_conflict_values_insert(statement) else {
        return Ok(None);
    };

    let insert_rows = match build_literal_insert_rows_from_values(
        view,
        app_context,
        sql_cache_registry,
        exec_ctx,
        table_id,
        false,
        params,
    )
    .await?
    {
        Some(rows) => rows,
        None => return Ok(None),
    };

    let primary_key_column = insert_rows.primary_key_column.ok_or_else(|| {
        KalamDbError::InvalidOperation(
            "ON CONFLICT requires a single primary key column".to_string(),
        )
    })?;

    validate_primary_key_conflict_target(on_conflict, &primary_key_column)
        .map_err(KalamDbError::InvalidOperation)?;

    let action = parse_on_conflict_action_with_params(on_conflict, params)
        .map_err(KalamDbError::InvalidOperation)?;

    Ok(Some(LiteralOnConflictUpdateRows {
        table_type: insert_rows.table_type,
        primary_key_column,
        rows: insert_rows.rows,
        action,
        returning: view.insert.returning.clone(),
    }))
}

const SYSTEM_TABLE_DML_DENIED: &str = "cannot insert into system tables";

pub(crate) fn reject_system_table_dml(table_type: TableType) -> Result<(), KalamDbError> {
    if table_type == TableType::System {
        return Err(KalamDbError::PermissionDenied(SYSTEM_TABLE_DML_DENIED.to_string()));
    }
    Ok(())
}

pub(crate) struct OnConflictUserIds {
    pub lookup_user_id:           Option<UserId>,
    pub default_mutation_user_id: Option<UserId>,
}

pub(crate) fn on_conflict_user_ids(
    table_type: TableType,
    exec_ctx: &ExecutionContext,
) -> Result<OnConflictUserIds, KalamDbError> {
    reject_system_table_dml(table_type)?;
    let user_id = exec_ctx.user_id().clone();
    Ok(match table_type {
        TableType::User | TableType::Stream => OnConflictUserIds {
            lookup_user_id:           Some(user_id.clone()),
            default_mutation_user_id: Some(user_id),
        },
        TableType::Shared => OnConflictUserIds {
            lookup_user_id:           Some(user_id),
            default_mutation_user_id: None,
        },
        TableType::System => unreachable!("system tables rejected above"),
    })
}

pub(crate) fn build_on_conflict_staged_mutation_for_action(
    action: &ParsedOnConflictAction,
    transaction_id: &TransactionId,
    table_id: &TableId,
    table_type: TableType,
    mutation_user_id: Option<UserId>,
    primary_key: String,
    inserted_row: &Row,
    existing_row: Option<&Row>,
) -> Result<Option<OnConflictStagedMutation>, KalamDbError> {
    match action {
        ParsedOnConflictAction::DoNothing => {
            if existing_row.is_some() {
                return Ok(None);
            }
            Ok(Some(build_on_conflict_staged_mutation(
                transaction_id,
                table_id,
                table_type,
                mutation_user_id,
                primary_key,
                inserted_row,
                &[],
                None,
            )?))
        },
        ParsedOnConflictAction::DoUpdate {
            assignments,
            where_clause,
        } => {
            if existing_row.is_some()
                && !on_conflict_update_should_apply(where_clause)
                    .map_err(KalamDbError::InvalidOperation)?
            {
                return Ok(None);
            }
            Ok(Some(build_on_conflict_staged_mutation(
                transaction_id,
                table_id,
                table_type,
                mutation_user_id,
                primary_key,
                inserted_row,
                assignments,
                existing_row,
            )?))
        },
    }
}

pub(crate) fn build_on_conflict_staged_mutation(
    transaction_id: &TransactionId,
    table_id: &TableId,
    table_type: TableType,
    user_id: Option<UserId>,
    primary_key: String,
    inserted_row: &Row,
    assignments: &[OnConflictUpdateAssignment],
    existing_row: Option<&Row>,
) -> Result<OnConflictStagedMutation, KalamDbError> {
    if let Some(existing_row) = existing_row {
        let mut updates = BTreeMap::new();
        for assignment in assignments {
            let value = match &assignment.value {
                OnConflictUpdateValue::InsertedColumn(column_name) => {
                    inserted_row.values.get(column_name).cloned().ok_or_else(|| {
                        KalamDbError::InvalidOperation(format!(
                            "EXCLUDED.{} does not exist in the inserted row",
                            column_name
                        ))
                    })?
                },
                OnConflictUpdateValue::Literal(value) => value.clone(),
            };
            updates.insert(assignment.column_name.clone(), value);
        }

        let mut returned_values = existing_row.values.clone();
        for (column_name, value) in &updates {
            returned_values.insert(column_name.clone(), value.clone());
        }

        Ok(OnConflictStagedMutation {
            mutation:     StagedMutation::new(
                transaction_id.clone(),
                table_id.clone(),
                table_type,
                user_id,
                OperationKind::Update,
                primary_key,
                Row::new(updates),
                false,
            ),
            returned_row: Row::new(returned_values),
        })
    } else {
        Ok(OnConflictStagedMutation {
            mutation:     StagedMutation::new(
                transaction_id.clone(),
                table_id.clone(),
                table_type,
                user_id,
                OperationKind::Insert,
                primary_key,
                inserted_row.clone(),
                false,
            ),
            returned_row: inserted_row.clone(),
        })
    }
}

fn prepare_default_template(
    default_value: &ColumnDefault,
) -> Result<FastInsertDefaultTemplate, KalamDbError> {
    match default_value {
        ColumnDefault::None => Err(KalamDbError::InvalidOperation(
            "Missing default value metadata for omitted insert column".to_string(),
        )),
        ColumnDefault::Literal(json) => {
            Ok(FastInsertDefaultTemplate::Literal(json_value_to_scalar(json)))
        },
        ColumnDefault::FunctionCall(call) => {
            if call.has_placeholder() {
                return Err(KalamDbError::InvalidOperation(
                    "DEFAULT procedure arguments cannot use placeholders".to_string(),
                ));
            }
            if let Some(builtin) = call.scalar_udf_name() {
                match call.unqualified_name() {
                    "now" | "current_timestamp" => Ok(FastInsertDefaultTemplate::CurrentTimestamp),
                    "current_user" => Ok(FastInsertDefaultTemplate::CurrentUser),
                    "snowflake_id" | "auto_increment" => Ok(FastInsertDefaultTemplate::SnowflakeId),
                    "uuid_v7" => Ok(FastInsertDefaultTemplate::UuidV7),
                    "ulid" => Ok(FastInsertDefaultTemplate::Ulid),
                    _ => Err(KalamDbError::InvalidOperation(format!(
                        "Unsupported default function '{builtin}' in transaction batch INSERT"
                    ))),
                }
            } else {
                Ok(FastInsertDefaultTemplate::Procedure(call.clone()))
            }
        },
    }
}

fn prepare_statement_default(
    default_template: &FastInsertDefaultTemplate,
    exec_ctx: &ExecutionContext,
) -> Result<PreparedDefaultValue, KalamDbError> {
    match default_template {
        FastInsertDefaultTemplate::Literal(value) => {
            Ok(PreparedDefaultValue::Constant(value.clone()))
        },
        FastInsertDefaultTemplate::CurrentTimestamp => Ok(PreparedDefaultValue::Constant(
            ScalarValue::TimestampMicrosecond(Some(Utc::now().timestamp_micros()), None),
        )),
        FastInsertDefaultTemplate::CurrentUser => {
            let user_id = exec_ctx.user_id();
            Ok(PreparedDefaultValue::Constant(ScalarValue::Utf8(Some(
                user_id.as_str().to_string(),
            ))))
        },
        FastInsertDefaultTemplate::SnowflakeId => {
            Ok(PreparedDefaultValue::Volatile(VolatileDefaultFunction::SnowflakeId))
        },
        FastInsertDefaultTemplate::UuidV7 => {
            Ok(PreparedDefaultValue::Volatile(VolatileDefaultFunction::UuidV7))
        },
        FastInsertDefaultTemplate::Ulid => {
            Ok(PreparedDefaultValue::Volatile(VolatileDefaultFunction::Ulid))
        },
        FastInsertDefaultTemplate::Procedure(call) => {
            Ok(PreparedDefaultValue::Procedure(call.clone()))
        },
    }
}

async fn materialize_prepared_default(
    prepared_default: &PreparedDefaultValue,
    exec_ctx: &ExecutionContext,
    app_context: &Arc<AppContext>,
) -> Result<ScalarValue, KalamDbError> {
    match prepared_default {
        PreparedDefaultValue::Constant(value) => Ok(value.clone()),
        PreparedDefaultValue::Volatile(VolatileDefaultFunction::SnowflakeId) => {
            let id = insert_default_snowflake_generator().next_id().map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to generate SNOWFLAKE_ID() default value: {}",
                    e
                ))
            })?;
            Ok(ScalarValue::Int64(Some(id)))
        },
        PreparedDefaultValue::Volatile(VolatileDefaultFunction::UuidV7) => {
            Ok(ScalarValue::Utf8(Some(Uuid::now_v7().to_string())))
        },
        PreparedDefaultValue::Volatile(VolatileDefaultFunction::Ulid) => {
            Ok(ScalarValue::Utf8(Some(Ulid::generate().to_string())))
        },
        PreparedDefaultValue::Procedure(call) => {
            FunctionService::execute_routine_call(Arc::clone(app_context), exec_ctx, call).await
        },
    }
}

/// Batch-process multiple INSERT statements for the same table in an active transaction.
///
/// Resolves table metadata once for the batch, converts all VALUES rows to staged
/// mutations, and submits them with a single `stage_batch()` call.
pub(crate) async fn try_batch_inserts_in_transaction(
    statements: &[&Statement],
    app_context: Arc<AppContext>,
    sql_cache_registry: &SqlCacheRegistry,
    exec_ctx: &ExecutionContext,
    table_id: &TableId,
    transaction_id: &TransactionId,
) -> Result<Option<Vec<usize>>, KalamDbError> {
    if statements.is_empty() {
        return Ok(Some(vec![]));
    }

    if table_id.namespace_id().is_system_namespace() {
        return Ok(None);
    }

    let first_insert = match kalamdb_sql::insert_from_statement(statements[0]) {
        Some(insert) => insert,
        None => return Ok(None),
    };

    let cached_table = match app_context.schema_registry().get(table_id) {
        Some(cached) => cached,
        None => return Ok(None),
    };

    let cached_table_entry = cached_table.table_entry();
    let table_type = cached_table_entry.table_type;
    if table_type == TableType::Shared && matches!(exec_ctx.user_role(), Role::Anonymous) {
        return Err(KalamDbError::PermissionDenied(
            "Anonymous shared-table writes are denied".to_string(),
        ));
    }

    let requested_columns: Vec<String> = first_insert
        .columns
        .iter()
        .filter_map(kalamdb_sql::object_name_to_string)
        .collect();
    let metadata_cache_key =
        InsertMetadataCacheKey::new(table_id.clone(), requested_columns.clone());
    let insert_metadata = match sql_cache_registry.insert_metadata_cache().get(&metadata_cache_key)
    {
        Some(metadata) => metadata,
        None => {
            let metadata =
                Arc::new(build_insert_metadata(&requested_columns, cached_table.as_ref())?);
            sql_cache_registry
                .insert_metadata_cache()
                .insert_arc(metadata_cache_key, Arc::clone(&metadata));
            metadata
        },
    };

    let pk_column = match insert_metadata.primary_key_column.as_deref() {
        Some(column) => column,
        None => return Ok(None),
    };

    let schema = cached_table.arrow_schema()?;
    let user_id = staged_mutation_user_id(table_type, exec_ctx);
    let mut all_rows = Vec::with_capacity(statements.len());
    let mut per_statement_counts = Vec::with_capacity(statements.len());
    let mut total_rows = 0usize;

    for statement in statements {
        if !insert_columns_match(statement, &requested_columns) {
            return Ok(None);
        }

        let Some(view) = kalamdb_sql::values_insert_view_from_statement(
            statement,
            ValuesInsertShapeOptions::BATCH,
        ) else {
            return Ok(None);
        };

        let rows = match try_build_insert_rows_from_values_rows(
            view.value_rows,
            &insert_metadata,
            cached_table.as_ref(),
            &schema,
            exec_ctx,
            &[],
            Arc::clone(&app_context),
        )
        .await?
        {
            Some(rows) => rows,
            None => return Ok(None),
        };

        per_statement_counts.push(rows.len());
        total_rows += rows.len();
        all_rows.extend(rows);
    }

    let all_rows = coerce_rows(all_rows, &schema)
        .map_err(|e| KalamDbError::InvalidOperation(format!("Schema coercion failed: {}", e)))?;
    validate_file_columns_in_rows(
        &all_rows,
        cached_table.as_ref(),
        table_id,
        user_id.as_ref(),
        &app_context,
    )?;
    if table_type == TableType::Shared {
        let provider = cached_table.get_provider().ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Shared table provider not found: {table_id}"))
        })?;
        let provider = (provider.as_ref() as &dyn std::any::Any)
            .downcast_ref::<SharedTableProvider>()
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!(
                    "Shared table provider type mismatch: {table_id}"
                ))
            })?;
        let snapshot_commit_seq = app_context
            .transaction_coordinator()
            .get_handle(transaction_id)
            .and_then(|handle| handle.snapshot_commit_seq())
            .and_then(|log_index| {
                kalamdb_commons::ids::RaftVersionId::try_new(
                    log_index,
                    kalamdb_commons::ids::MAX_ORDINAL,
                )
                .ok()
                .map(|packed| packed.version())
            });
        provider
            .check_rows_authorized(
                exec_ctx.user_id(),
                exec_ctx.user_role(),
                PolicyCommand::Insert,
                true,
                &all_rows,
                snapshot_commit_seq,
            )
            .await?;
    }
    let all_mutations = build_insert_staged_mutations(
        transaction_id,
        table_id,
        table_type,
        user_id,
        pk_column,
        all_rows,
    )
    .map_err(|error| KalamDbError::InvalidOperation(error.to_string()))?;

    app_context
        .transaction_coordinator()
        .stage_batch(transaction_id, all_mutations)?;

    tracing::debug!(
        table_id = %table_id,
        statements = statements.len(),
        total_rows,
        "sql.transaction_batch_insert"
    );

    Ok(Some(per_statement_counts))
}
