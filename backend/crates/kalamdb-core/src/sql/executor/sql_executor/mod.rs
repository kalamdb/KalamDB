use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use arrow::{
    array::{Array, RecordBatch},
    datatypes::{Field, Schema, SchemaRef},
};
use datafusion::{
    common::tree_node::{Transformed, TransformedResult, TreeNode, TreeNodeRecursion},
    dataframe::DataFrame,
    datasource::MemTable,
    execution::context::SessionState,
    logical_expr::{Expr as DataFusionExpr, LogicalPlan},
    physical_plan::collect,
    prelude::{SessionConfig, SessionContext},
    scalar::ScalarValue,
    sql::parser::Statement as DfStatement,
};
use kalamdb_commons::{
    conversions::arrow_json_conversion::{arrow_value_to_scalar, json_rows_to_arrow_batch},
    models::{rows::row::Row, ReadContext, SchemaId, TableId, TransactionId, UserId},
    quote_sql_identifier,
    schemas::TableType,
    try_pk_bucket_key, PkBucketKey, Role, SystemTable,
};
use kalamdb_datafusion_sources::exec::DeferredBatchExec;
use kalamdb_session_datafusion::{install_extension, ScanDiagnosticsContext};
use kalamdb_sql::{
    classifier::{SqlStatement, SqlStatementKind, StatementClassificationError},
    rewrite_explain_for_datafusion,
};
use kalamdb_tables::{SharedTableProvider, UserTableProvider};
use kalamdb_transactions::{TransactionQueryContext, TransactionQueryExtension};
use sqlparser::ast::Statement;
use uuid::Uuid;

mod datafusion_error;
mod postgres_meta;
mod system_migrations;

use super::{
    point_read_session_cache_key::PointReadSessionCacheKey, PreparedExecutionStatement, SqlExecutor,
};
use crate::{
    error::KalamDbError,
    sql::{
        executor::{
            default_ordering::apply_default_order_by,
            handler_registry::HandlerRegistry,
            parameter_binding::{
                bind_placeholders_in_expr, replace_placeholders_in_plan, validate_params,
            },
            request_transaction_state::{
                map_request_transaction_error, AppContextRequestTransactionCoordinator,
                RequestTransactionState,
            },
        },
        plan_cache::{PlanCacheKey, SqlCacheRegistry, SqlCacheRegistryConfig},
        ExecutionContext, ExecutionResult,
    },
    transactions::CoordinatorAccessValidator,
};

#[derive(Debug, Clone, Copy)]
pub(super) enum DmlKind {
    Insert,
    Update,
    Delete,
}

struct OnConflictStageResult {
    rows_affected: usize,
    returned_rows: Vec<Row>,
}

struct OnConflictExistingRow {
    row:              Row,
    mutation_user_id: Option<UserId>,
}

enum OnConflictTableProvider<'a> {
    User(&'a UserTableProvider),
    Shared(&'a SharedTableProvider),
}

fn query_metric_kind(kind: &SqlStatementKind) -> kalamdb_observability::QueryMetricKind {
    match kind {
        SqlStatementKind::Select => kalamdb_observability::QueryMetricKind::Select,
        SqlStatementKind::Insert(_) => kalamdb_observability::QueryMetricKind::Insert,
        SqlStatementKind::Update(_) => kalamdb_observability::QueryMetricKind::Update,
        SqlStatementKind::Delete(_) => kalamdb_observability::QueryMetricKind::Delete,
        _ => kalamdb_observability::QueryMetricKind::Other,
    }
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();

    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }

    let first = needle[0].to_ascii_lowercase();
    haystack.windows(needle.len()).any(|window| {
        window[0].to_ascii_lowercase() == first && window.eq_ignore_ascii_case(needle)
    })
}

fn contains_internal_namespace_hint(sql: &str) -> bool {
    contains_ignore_ascii_case(sql, "system.") || contains_ignore_ascii_case(sql, "dba.")
}

fn table_ref_is_client_catalog(table: &datafusion::common::TableReference) -> bool {
    matches!(table.schema(), Some("pg_catalog") | Some("information_schema"))
}

fn extract_select_from_table_id(sql: &str, default_namespace: &SchemaId) -> Option<TableId> {
    let lowered = sql.to_ascii_lowercase();
    let from_idx = lowered.find(" from ")?;
    let rest = sql[from_idx + 6..].trim_start();
    let ident: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '.')
        .collect();
    if ident.is_empty() {
        return None;
    }
    let mut parts = ident.split('.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(table), None, None) => {
            Some(TableId::from_namespace(default_namespace.clone(), table))
        },
        (Some(namespace), Some(table), None) => Some(TableId::from_strings(namespace, table)),
        _ => None,
    }
}

fn extract_sql_target_table_id(sql: &str, default_namespace: &SchemaId) -> Option<TableId> {
    kalamdb_sql::extract_dml_table_id_fast(sql, default_namespace)
        .or_else(|| kalamdb_sql::extract_dml_table_id(sql, default_namespace))
        .or_else(|| extract_select_from_table_id(sql, default_namespace))
}

fn nullable_arrow_schema(schema: &SchemaRef) -> SchemaRef {
    let fields = schema
        .fields()
        .iter()
        .map(|field| {
            Field::new(field.name(), field.data_type().clone(), true)
                .with_metadata(field.metadata().clone())
        })
        .collect::<Vec<_>>();
    Arc::new(Schema::new(fields).with_metadata(schema.metadata().clone()))
}

impl SqlExecutor {
    async fn try_execute_literal_insert_via_applier(
        &self,
        sql: &str,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !params.is_empty() {
            super::parameter_binding::validate_params(params)?;
        }

        let Some(table_id) = metadata.table_id.as_ref() else {
            return Ok(None);
        };

        let has_active_tx = self.active_request_transaction_id(exec_ctx)?.is_some();
        if has_active_tx && !contains_ignore_ascii_case(sql, "ON CONFLICT") {
            return Ok(None);
        }

        let parsed_statement = match metadata.parsed_dml.as_deref() {
            Some(statement) => statement,
            None => return Ok(None),
        };

        // ON CONFLICT must be handled before the active-transaction bail-out below.
        if let Statement::Insert(insert) = parsed_statement {
            if kalamdb_sql::insert_has_on_conflict(insert) {
                if let Some(on_conflict_rows) =
                    super::transaction_batch_insert::try_build_literal_on_conflict_update_rows(
                        parsed_statement,
                        Arc::clone(&self.app_context),
                        self.sql_cache_registry.as_ref(),
                        exec_ctx,
                        table_id,
                        params,
                    )
                    .await?
                {
                    return self
                        .execute_literal_on_conflict_update(table_id, exec_ctx, on_conflict_rows)
                        .await
                        .map(Some);
                }
            }
        }

        if has_active_tx {
            return Ok(None);
        }

        let Some(insert_rows) = kalamdb_observability::kdb_await_in_info_span!(
            super::transaction_batch_insert::try_build_literal_insert_rows(
                parsed_statement,
                Arc::clone(&self.app_context),
                self.sql_cache_registry.as_ref(),
                exec_ctx,
                table_id,
                params,
            ),
            "sql.insert_bind"
        )?
        else {
            return Ok(None);
        };

        if let Some(returning) = insert_rows.returning.as_ref() {
            let returning_rows = insert_rows.rows.clone();
            let Some(_) = self
                .insert_literal_rows_via_applier(
                    insert_rows.table_type,
                    table_id,
                    exec_ctx,
                    insert_rows.rows,
                )
                .await?
            else {
                return Ok(None);
            };

            self.build_on_conflict_returning_result(table_id, returning, returning_rows)
                .await
                .map(Some)
        } else {
            let Some(rows_affected) = self
                .insert_literal_rows_via_applier(
                    insert_rows.table_type,
                    table_id,
                    exec_ctx,
                    insert_rows.rows,
                )
                .await?
            else {
                return Ok(None);
            };

            Ok(Some(ExecutionResult::Inserted { rows_affected }))
        }
    }

    async fn insert_literal_rows_via_applier(
        &self,
        table_type: TableType,
        table_id: &TableId,
        exec_ctx: &ExecutionContext,
        rows: Vec<Row>,
    ) -> Result<Option<usize>, KalamDbError> {
        if table_type == TableType::System {
            return Ok(None);
        }

        if table_type == TableType::Shared {
            let provider =
                self.app_context.schema_registry().get_provider(table_id).ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "Table provider not found for RLS check: {}",
                        table_id
                    ))
                })?;
            let provider = (provider.as_ref() as &dyn std::any::Any)
                .downcast_ref::<SharedTableProvider>()
                .ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "Shared table provider type mismatch for RLS check: {}",
                        table_id
                    ))
                })?;
            kalamdb_observability::kdb_await_in_info_span!(
                provider.check_rows_authorized(
                    exec_ctx.user_id(),
                    exec_ctx.user_role(),
                    kalamdb_commons::PolicyCommand::Insert,
                    true,
                    &rows,
                    None,
                ),
                "table.rls_insert"
            )?;
        }

        let applier = self.app_context.applier();
        let rows_affected = match table_type {
            TableType::Shared => applier
                .insert_shared_data(table_id.clone(), Some(exec_ctx.user_id().clone()), rows)
                .await
                .map_err(KalamDbError::from)?
                .rows_affected(),
            TableType::User | TableType::Stream => applier
                .insert_user_data(table_id.clone(), exec_ctx.user_id().clone(), rows)
                .await
                .map_err(KalamDbError::from)?
                .rows_affected(),
            TableType::System => unreachable!("system tables handled above"),
        };

        Ok(Some(rows_affected as usize))
    }

    async fn run_with_request_transaction<'a, T, F, Fut>(
        &'a self,
        exec_ctx: &'a ExecutionContext,
        request_prefix: &str,
        context_error: &str,
        op: F,
    ) -> Result<T, KalamDbError>
    where
        F: FnOnce(&'a ExecutionContext, TransactionId) -> Fut,
        Fut: std::future::Future<Output = Result<T, KalamDbError>>,
    {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(format!(
                    "{context_error} requires a request-scoped execution context (prefix={request_prefix})"
                ))
            })?;
        request_state.sync(&request_transaction_coordinator);

        let started_transaction = if request_state.is_active() {
            false
        } else {
            request_state
                .begin(&request_transaction_coordinator)
                .map_err(map_request_transaction_error)?;
            true
        };

        let transaction_id = request_state.active_transaction_id().cloned().ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("{context_error} could not start a transaction"))
        })?;

        match op(exec_ctx, transaction_id).await {
            Ok(value) => {
                if started_transaction {
                    if let Err(error) = request_state.commit(&request_transaction_coordinator).await
                    {
                        let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                        return Err(map_request_transaction_error(error));
                    }
                }
                Ok(value)
            },
            Err(error) => {
                if started_transaction {
                    let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                }
                Err(error)
            },
        }
    }

    async fn execute_literal_on_conflict_update(
        &self,
        table_id: &TableId,
        exec_ctx: &ExecutionContext,
        on_conflict_rows: super::transaction_batch_insert::LiteralOnConflictUpdateRows,
    ) -> Result<ExecutionResult, KalamDbError> {
        if on_conflict_rows.table_type == TableType::Shared
            && matches!(exec_ctx.user_role(), Role::User | Role::Service)
        {
            return Err(KalamDbError::InvalidOperation(
                "ON CONFLICT on RLS-protected shared tables is not supported until both conflict \
                 branches can be policy-checked"
                    .to_string(),
            ));
        }
        super::transaction_batch_insert::reject_system_table_dml(on_conflict_rows.table_type)?;

        let owned_exec_ctx;
        let dml_exec_ctx = if exec_ctx.request_id().is_some() {
            exec_ctx
        } else {
            owned_exec_ctx =
                exec_ctx.clone().with_request_id(format!("sql-on-conflict-{}", Uuid::now_v7()));
            &owned_exec_ctx
        };

        let on_conflict_rows_ref = &on_conflict_rows;
        let stage_result = self
            .run_with_request_transaction(
                dml_exec_ctx,
                "sql-on-conflict",
                "ON CONFLICT DO UPDATE",
                |ctx, transaction_id| async move {
                    self.stage_literal_on_conflict_update(
                        table_id,
                        ctx,
                        &transaction_id,
                        on_conflict_rows_ref,
                    )
                    .await
                },
            )
            .await?;

        if let Some(returning) = on_conflict_rows.returning.as_ref() {
            self.build_on_conflict_returning_result(table_id, returning, stage_result.returned_rows)
                .await
        } else {
            Ok(ExecutionResult::Inserted {
                rows_affected: stage_result.rows_affected,
            })
        }
    }

    async fn stage_literal_on_conflict_update(
        &self,
        table_id: &TableId,
        exec_ctx: &ExecutionContext,
        transaction_id: &TransactionId,
        on_conflict_rows: &super::transaction_batch_insert::LiteralOnConflictUpdateRows,
    ) -> Result<OnConflictStageResult, KalamDbError> {
        let user_ids = super::transaction_batch_insert::on_conflict_user_ids(
            on_conflict_rows.table_type,
            exec_ctx,
        )?;

        let provider_arc =
            self.app_context.schema_registry().get_provider(table_id).ok_or_else(|| {
                KalamDbError::InvalidOperation(format!("Table provider not found: {}", table_id))
            })?;
        let table_provider = match on_conflict_rows.table_type {
            TableType::User | TableType::Stream => {
                let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                    .downcast_ref::<UserTableProvider>()
                    .ok_or_else(|| {
                        KalamDbError::InvalidOperation(format!(
                            "Provider type mismatch for user table {}",
                            table_id
                        ))
                    })?;
                OnConflictTableProvider::User(provider)
            },
            TableType::Shared => {
                let provider = (provider_arc.as_ref() as &dyn std::any::Any)
                    .downcast_ref::<SharedTableProvider>()
                    .ok_or_else(|| {
                        KalamDbError::InvalidOperation(format!(
                            "Provider type mismatch for shared table {}",
                            table_id
                        ))
                    })?;
                OnConflictTableProvider::Shared(provider)
            },
            TableType::System => unreachable!("system tables rejected above"),
        };

        let mut mutations = Vec::with_capacity(on_conflict_rows.rows.len());
        let mut returned_rows = Vec::with_capacity(on_conflict_rows.rows.len());
        let mut row_state_by_pk: HashMap<PkBucketKey, OnConflictExistingRow> =
            HashMap::with_capacity(on_conflict_rows.rows.len());
        for row in &on_conflict_rows.rows {
            let primary_key =
                row.values.get(&on_conflict_rows.primary_key_column).ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "ON CONFLICT requires primary key column '{}'",
                        on_conflict_rows.primary_key_column
                    ))
                })?;
            let pk_key = try_pk_bucket_key(primary_key).map_err(KalamDbError::InvalidOperation)?;
            let primary_key_string = pk_key.to_string();
            if !row_state_by_pk.contains_key(&pk_key) {
                if let Some(existing_row) = self
                    .on_conflict_existing_row(
                        transaction_id,
                        table_id,
                        on_conflict_rows.table_type,
                        user_ids.lookup_user_id.clone(),
                        primary_key,
                        &table_provider,
                    )
                    .await?
                {
                    row_state_by_pk.insert(pk_key.clone(), existing_row);
                }
            }

            let existing_row = row_state_by_pk.get(&pk_key);
            let mutation_user_id = existing_row
                .as_ref()
                .and_then(|existing| existing.mutation_user_id.clone())
                .or_else(|| user_ids.default_mutation_user_id.clone());
            let existing_row_ref = existing_row.map(|existing| &existing.row);

            let Some(staged) =
                super::transaction_batch_insert::build_on_conflict_staged_mutation_for_action(
                    &on_conflict_rows.action,
                    transaction_id,
                    table_id,
                    on_conflict_rows.table_type,
                    mutation_user_id,
                    primary_key_string.clone(),
                    row,
                    existing_row_ref,
                )?
            else {
                continue;
            };

            let returned_row = staged.returned_row.clone();
            let mutation_user_id = staged.mutation.user_id.clone();
            mutations.push(staged.mutation);
            returned_rows.push(staged.returned_row);
            row_state_by_pk.insert(
                pk_key,
                OnConflictExistingRow {
                    row: returned_row,
                    mutation_user_id,
                },
            );
        }

        self.app_context
            .transaction_coordinator()
            .stage_batch(transaction_id, mutations)?;

        Ok(OnConflictStageResult {
            rows_affected: returned_rows.len(),
            returned_rows,
        })
    }

    async fn on_conflict_existing_row(
        &self,
        transaction_id: &TransactionId,
        table_id: &TableId,
        table_type: TableType,
        user_id: Option<UserId>,
        primary_key: &ScalarValue,
        table_provider: &OnConflictTableProvider<'_>,
    ) -> Result<Option<OnConflictExistingRow>, KalamDbError> {
        let primary_key_str = primary_key.to_string();
        if let Some(overlay) =
            self.app_context.transaction_coordinator().get_overlay(transaction_id)
        {
            let overlay_entry = match table_type {
                TableType::User | TableType::Stream => overlay.latest_visible_entry_for_scope(
                    table_id,
                    user_id.as_ref(),
                    primary_key_str.as_str(),
                ),
                TableType::Shared => overlay
                    .latest_visible_entry_for_scope(
                        table_id,
                        user_id.as_ref(),
                        primary_key_str.as_str(),
                    )
                    .or_else(|| overlay.latest_visible_entry(table_id, primary_key_str.as_str())),
                TableType::System => {
                    overlay.latest_visible_entry(table_id, primary_key_str.as_str())
                },
            };
            if let Some(entry) = overlay_entry {
                return if entry.is_deleted() {
                    Ok(None)
                } else {
                    Ok(Some(OnConflictExistingRow {
                        row:              entry.payload.clone(),
                        mutation_user_id: entry.user_id.clone(),
                    }))
                };
            }
        }

        match (table_type, table_provider) {
            (TableType::User | TableType::Stream, OnConflictTableProvider::User(provider)) => {
                let user_id = user_id.ok_or_else(|| {
                    KalamDbError::InvalidOperation(
                        "user table ON CONFLICT lookup requires user_id".to_string(),
                    )
                })?;
                Ok(provider.find_by_pk(&user_id, primary_key).await?.map(|(_, row)| {
                    OnConflictExistingRow {
                        row:              row.fields,
                        mutation_user_id: Some(user_id),
                    }
                }))
            },
            (TableType::Shared, OnConflictTableProvider::Shared(provider)) => {
                Ok(provider.find_by_pk(primary_key).await?.map(|(_, row)| OnConflictExistingRow {
                    row:              row.fields,
                    mutation_user_id: None,
                }))
            },
            _ => Err(KalamDbError::InvalidOperation(format!(
                "Provider type mismatch for table {}",
                table_id
            ))),
        }
    }

    async fn try_execute_standalone_catalog_function_select(
        &self,
        statement: &mut Statement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        had_catalog_func: &mut bool,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !self.app_context.system_tables().catalog_stores().has_user_routines() {
            return Ok(None);
        }
        let Statement::Query(query) = statement else {
            return Ok(None);
        };
        if query.with.is_some()
            || query.order_by.is_some()
            || query.limit_clause.is_some()
            || query.fetch.is_some()
            || !query.locks.is_empty()
        {
            return Ok(None);
        }
        let sqlparser::ast::SetExpr::Select(select) = query.body.as_mut() else {
            return Ok(None);
        };
        if select.distinct.is_some()
            || select.top.is_some()
            || !select.from.is_empty()
            || !select.lateral_views.is_empty()
            || select.selection.is_some()
            || !matches!(
                &select.group_by,
                sqlparser::ast::GroupByExpr::Expressions(exprs, _) if exprs.is_empty()
            )
            || !select.cluster_by.is_empty()
            || !select.distribute_by.is_empty()
            || !select.sort_by.is_empty()
            || select.having.is_some()
            || !select.named_window.is_empty()
            || select.qualify.is_some()
            || select.projection.len() != 1
        {
            return Ok(None);
        }

        let (col_name, func) = match &mut select.projection[0] {
            sqlparser::ast::SelectItem::UnnamedExpr(sqlparser::ast::Expr::Function(func)) => {
                let name = func
                    .name
                    .0
                    .last()
                    .map(|part| part.to_string().trim_matches('"').to_ascii_lowercase())
                    .unwrap_or_else(|| "result".to_string());
                (name, func)
            },
            sqlparser::ast::SelectItem::ExprWithAlias {
                expr: sqlparser::ast::Expr::Function(func),
                alias,
            } => (alias.value.clone(), func),
            _ => return Ok(None),
        };

        let Some(routine_id) =
            Self::resolve_catalog_routine_id(&func.name, &exec_ctx.default_namespace())
        else {
            return Ok(None);
        };
        let stores = self.app_context.system_tables().catalog_stores();
        let Ok(Some(routine)) = stores.get_routine(&routine_id) else {
            return Ok(None);
        };
        if routine.language.as_deref() == Some("INTERNAL") {
            return Ok(None);
        }
        if routine.kind == kalamdb_commons::models::RoutineKind::Procedure {
            return Err(KalamDbError::InvalidSql(format!(
                "{} is a procedure, not a function; invoke it with CALL",
                routine_id
            )));
        }

        if let sqlparser::ast::FunctionArguments::List(list) = &mut func.args {
            for arg in &mut list.args {
                if let sqlparser::ast::FunctionArg::Unnamed(
                    sqlparser::ast::FunctionArgExpr::Expr(arg_expr),
                ) = arg
                {
                    self.evaluate_catalog_functions_in_expr(
                        arg_expr,
                        exec_ctx,
                        params,
                        had_catalog_func,
                    )
                    .await?;
                }
            }
        }

        let call = kalamdb_sql::ddl::column_default::function_to_routine_call_with_placeholders(
            func,
            &exec_ctx.default_namespace(),
        )
        .map_err(KalamDbError::InvalidSql)?;

        let rv = crate::functions::FunctionService::execute_scalar_function_call(
            Arc::clone(&self.app_context),
            exec_ctx,
            &call,
            params,
        )
        .await?;
        *had_catalog_func = true;

        if matches!(
            &rv.value,
            ScalarValue::Struct(_)
                | ScalarValue::List(_)
                | ScalarValue::LargeList(_)
                | ScalarValue::FixedSizeList(_)
        ) {
            let array = rv
                .value
                .to_array()
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            let field = arrow::datatypes::Field::new(col_name, array.data_type().clone(), true);
            let schema = Arc::new(arrow::datatypes::Schema::new(vec![field]));
            let batch = arrow::array::RecordBatch::try_new(Arc::clone(&schema), vec![array])
                .map_err(|error| KalamDbError::ExecutionError(error.to_string()))?;
            return Ok(Some(ExecutionResult::Rows {
                row_count: batch.num_rows(),
                batches:   vec![batch],
                schema:    Some(schema),
            }));
        }

        let ast_expr = Self::routine_value_to_ast_expr(&rv, &routine)?;
        select.projection[0] = sqlparser::ast::SelectItem::ExprWithAlias {
            expr:  ast_expr,
            alias: sqlparser::ast::Ident::new(col_name),
        };
        Ok(None)
    }

    async fn evaluate_catalog_functions_in_statement(
        &self,
        statement: &mut Statement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<bool, KalamDbError> {
        if !self.app_context.system_tables().catalog_stores().has_user_routines() {
            return Ok(false);
        }
        let mut changed = false;
        self.evaluate_catalog_functions_in_statement_inner(
            statement,
            exec_ctx,
            params,
            &mut changed,
        )
        .await?;
        Ok(changed)
    }

    async fn evaluate_catalog_functions_in_statement_inner(
        &self,
        statement: &mut Statement,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match statement {
            Statement::Query(query) => {
                self.evaluate_catalog_functions_in_query(query, exec_ctx, params, changed).await?;
            },
            Statement::Insert(insert) => {
                if let Some(source) = &mut insert.source {
                    self.evaluate_catalog_functions_in_query(source, exec_ctx, params, changed)
                        .await?;
                }
                if let Some(returning) = &mut insert.returning {
                    for item in returning {
                        self.evaluate_catalog_functions_in_select_item(
                            item, exec_ctx, params, changed,
                        )
                        .await?;
                    }
                }
            },
            Statement::Update(update) => {
                for assignment in &mut update.assignments {
                    self.evaluate_catalog_functions_in_expr(
                        &mut assignment.value,
                        exec_ctx,
                        params,
                        changed,
                    )
                    .await?;
                }
                if let Some(selection) = &mut update.selection {
                    self.evaluate_catalog_functions_in_expr(selection, exec_ctx, params, changed)
                        .await?;
                }
                if let Some(returning) = &mut update.returning {
                    for item in returning {
                        self.evaluate_catalog_functions_in_select_item(
                            item, exec_ctx, params, changed,
                        )
                        .await?;
                    }
                }
            },
            Statement::Delete(delete) => {
                if let Some(selection) = &mut delete.selection {
                    self.evaluate_catalog_functions_in_expr(selection, exec_ctx, params, changed)
                        .await?;
                }
                if let Some(returning) = &mut delete.returning {
                    for item in returning {
                        self.evaluate_catalog_functions_in_select_item(
                            item, exec_ctx, params, changed,
                        )
                        .await?;
                    }
                }
            },
            _ => {},
        }
        Ok(())
    }

    async fn evaluate_catalog_functions_in_query(
        &self,
        query: &mut sqlparser::ast::Query,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        if let Some(with) = &mut query.with {
            for cte in &mut with.cte_tables {
                Box::pin(self.evaluate_catalog_functions_in_query(
                    &mut cte.query,
                    exec_ctx,
                    params,
                    changed,
                ))
                .await?;
            }
        }
        Box::pin(self.evaluate_catalog_functions_in_set_expr(
            &mut query.body,
            exec_ctx,
            params,
            changed,
        ))
        .await?;
        Ok(())
    }

    async fn evaluate_catalog_functions_in_set_expr(
        &self,
        set_expr: &mut sqlparser::ast::SetExpr,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match set_expr {
            sqlparser::ast::SetExpr::Select(select) => {
                for item in &mut select.projection {
                    self.evaluate_catalog_functions_in_select_item(item, exec_ctx, params, changed)
                        .await?;
                }
                if let Some(selection) = &mut select.selection {
                    self.evaluate_catalog_functions_in_expr(selection, exec_ctx, params, changed)
                        .await?;
                }
                if let Some(having) = &mut select.having {
                    self.evaluate_catalog_functions_in_expr(having, exec_ctx, params, changed)
                        .await?;
                }
            },
            sqlparser::ast::SetExpr::Values(values) => {
                for row in &mut values.rows {
                    for expr in row.iter_mut() {
                        self.evaluate_catalog_functions_in_expr(expr, exec_ctx, params, changed)
                            .await?;
                    }
                }
            },
            sqlparser::ast::SetExpr::Query(inner) => {
                Box::pin(self.evaluate_catalog_functions_in_query(
                    inner, exec_ctx, params, changed,
                ))
                .await?;
            },
            sqlparser::ast::SetExpr::SetOperation { left, right, .. } => {
                Box::pin(self.evaluate_catalog_functions_in_set_expr(
                    left, exec_ctx, params, changed,
                ))
                .await?;
                Box::pin(self.evaluate_catalog_functions_in_set_expr(
                    right, exec_ctx, params, changed,
                ))
                .await?;
            },
            _ => {},
        }
        Ok(())
    }

    async fn evaluate_catalog_functions_in_select_item(
        &self,
        item: &mut sqlparser::ast::SelectItem,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match item {
            sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
                let alias = if let sqlparser::ast::Expr::Function(func) = expr {
                    func.name
                        .0
                        .last()
                        .map(|part| part.to_string().trim_matches('"').to_ascii_lowercase())
                } else {
                    None
                };
                let before = *changed;
                self.evaluate_catalog_functions_in_expr(expr, exec_ctx, params, changed).await?;
                if *changed != before {
                    if let Some(alias_name) = alias {
                        let replaced = std::mem::replace(
                            expr,
                            sqlparser::ast::Expr::Value(sqlparser::ast::Value::Null.into()),
                        );
                        *item = sqlparser::ast::SelectItem::ExprWithAlias {
                            expr:  replaced,
                            alias: sqlparser::ast::Ident::new(alias_name),
                        };
                    }
                }
            },
            sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => {
                self.evaluate_catalog_functions_in_expr(expr, exec_ctx, params, changed).await?;
            },
            _ => {},
        }
        Ok(())
    }

    async fn evaluate_catalog_functions_in_expr(
        &self,
        expr: &mut sqlparser::ast::Expr,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
        changed: &mut bool,
    ) -> Result<(), KalamDbError> {
        match expr {
            sqlparser::ast::Expr::Nested(inner) => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::BinaryOp { left, right, .. } => {
                Box::pin(self.evaluate_catalog_functions_in_expr(left, exec_ctx, params, changed))
                    .await?;
                Box::pin(self.evaluate_catalog_functions_in_expr(right, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::UnaryOp { expr: inner, .. } => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::Cast { expr: inner, .. } => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::IsNull(inner) | sqlparser::ast::Expr::IsNotNull(inner) => {
                Box::pin(self.evaluate_catalog_functions_in_expr(inner, exec_ctx, params, changed))
                    .await?;
            },
            sqlparser::ast::Expr::Function(func) => {
                if let sqlparser::ast::FunctionArguments::List(list) = &mut func.args {
                    for arg in &mut list.args {
                        if let sqlparser::ast::FunctionArg::Unnamed(
                            sqlparser::ast::FunctionArgExpr::Expr(arg_expr),
                        ) = arg
                        {
                            Box::pin(self.evaluate_catalog_functions_in_expr(
                                arg_expr, exec_ctx, params, changed,
                            ))
                            .await?;
                        }
                    }
                }
                if let Some(routine_id) =
                    Self::resolve_catalog_routine_id(&func.name, &exec_ctx.default_namespace())
                {
                    let stores = self.app_context.system_tables().catalog_stores();
                    if let Ok(Some(routine)) = stores.get_routine(&routine_id) {
                        if routine.language.as_deref() != Some("INTERNAL") {
                            if routine.kind == kalamdb_commons::models::RoutineKind::Procedure {
                                return Err(KalamDbError::InvalidSql(format!(
                                    "{} is a procedure, not a function; invoke it with CALL",
                                    routine_id
                                )));
                            }
                            let call = kalamdb_sql::ddl::column_default::function_to_routine_call_with_placeholders(
                                func,
                                &exec_ctx.default_namespace(),
                            )
                            .map_err(KalamDbError::InvalidSql)?;
                            let rv = Box::pin(
                                crate::functions::FunctionService::execute_scalar_function_call(
                                    Arc::clone(&self.app_context),
                                    exec_ctx,
                                    &call,
                                    params,
                                ),
                            )
                            .await?;
                            *expr = Self::routine_value_to_ast_expr(&rv, &routine)?;
                            *changed = true;
                        }
                    }
                }
            },
            _ => {},
        }
        Ok(())
    }

    fn resolve_catalog_routine_id(
        name: &sqlparser::ast::ObjectName,
        default_namespace: &SchemaId,
    ) -> Option<kalamdb_commons::models::RoutineId> {
        let parts: Vec<String> = name
            .0
            .iter()
            .filter_map(|part| match part {
                sqlparser::ast::ObjectNamePart::Identifier(ident) => {
                    if ident.quote_style.is_some() {
                        Some(ident.value.clone())
                    } else {
                        Some(ident.value.to_ascii_lowercase())
                    }
                },
                _ => None,
            })
            .collect();
        match parts.as_slice() {
            [unqualified] => {
                if kalamdb_commons::models::is_builtin_default_name(unqualified) {
                    None
                } else {
                    Some(kalamdb_commons::models::RoutineId::from_parts(
                        Some(default_namespace),
                        unqualified,
                    ))
                }
            },
            [schema, unqualified] => Some(kalamdb_commons::models::RoutineId::from_parts(
                Some(&SchemaId::new(schema)),
                unqualified,
            )),
            _ => None,
        }
    }

    fn routine_value_to_ast_expr(
        rv: &kalamdb_functions::RoutineValue,
        routine: &kalamdb_system::CatalogRoutine,
    ) -> Result<sqlparser::ast::Expr, KalamDbError> {
        use kalamdb_commons::KalamDataType;
        use sqlparser::ast::{DataType as AstDataType, Expr as AstExpr, Value as AstValue};
        let scalar = &rv.value;
        let base_expr = match scalar {
            ScalarValue::Null
            | ScalarValue::Boolean(None)
            | ScalarValue::Int8(None)
            | ScalarValue::Int16(None)
            | ScalarValue::Int32(None)
            | ScalarValue::Int64(None)
            | ScalarValue::UInt8(None)
            | ScalarValue::UInt16(None)
            | ScalarValue::UInt32(None)
            | ScalarValue::UInt64(None)
            | ScalarValue::Float32(None)
            | ScalarValue::Float64(None)
            | ScalarValue::Utf8(None)
            | ScalarValue::LargeUtf8(None)
            | ScalarValue::Utf8View(None) => match routine.return_data_type {
                Some(KalamDataType::Boolean) => AstExpr::Cast {
                    kind:      sqlparser::ast::CastKind::Cast,
                    expr:      Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Boolean,
                    array:     false,
                    format:    None,
                },
                Some(KalamDataType::SmallInt) => AstExpr::Cast {
                    kind:      sqlparser::ast::CastKind::Cast,
                    expr:      Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::SmallInt(None),
                    array:     false,
                    format:    None,
                },
                Some(KalamDataType::Int) => AstExpr::Cast {
                    kind:      sqlparser::ast::CastKind::Cast,
                    expr:      Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Int(None),
                    array:     false,
                    format:    None,
                },
                Some(KalamDataType::BigInt) => AstExpr::Cast {
                    kind:      sqlparser::ast::CastKind::Cast,
                    expr:      Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::BigInt(None),
                    array:     false,
                    format:    None,
                },
                Some(KalamDataType::Double | KalamDataType::Float) => AstExpr::Cast {
                    kind:      sqlparser::ast::CastKind::Cast,
                    expr:      Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Double(sqlparser::ast::ExactNumberInfo::None),
                    array:     false,
                    format:    None,
                },
                _ => AstExpr::Cast {
                    kind:      sqlparser::ast::CastKind::Cast,
                    expr:      Box::new(AstExpr::Value(AstValue::Null.into())),
                    data_type: AstDataType::Text,
                    array:     false,
                    format:    None,
                },
            },
            ScalarValue::Boolean(Some(b)) => AstExpr::Value(AstValue::Boolean(*b).into()),
            ScalarValue::Int8(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Int16(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Int32(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Int64(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt8(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt16(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt32(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::UInt64(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Float32(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Float64(Some(n)) => {
                AstExpr::Value(AstValue::Number(n.to_string(), false).into())
            },
            ScalarValue::Utf8(Some(s))
            | ScalarValue::LargeUtf8(Some(s))
            | ScalarValue::Utf8View(Some(s)) => {
                AstExpr::Value(AstValue::SingleQuotedString(s.clone()).into())
            },
            other => {
                if let Some(json) = crate::functions::routine_value_as_json(rv) {
                    match json {
                        serde_json::Value::Null => {
                            AstExpr::Value(AstValue::Null.into())
                        },
                        serde_json::Value::Bool(b) => {
                            AstExpr::Value(AstValue::Boolean(b).into())
                        },
                        serde_json::Value::Number(num) => {
                            AstExpr::Value(AstValue::Number(num.to_string(), false).into())
                        },
                        serde_json::Value::String(s) => {
                            AstExpr::Value(AstValue::SingleQuotedString(s).into())
                        },
                        other_json => {
                            AstExpr::Value(AstValue::SingleQuotedString(other_json.to_string()).into())
                        },
                    }
                } else {
                    AstExpr::Value(AstValue::SingleQuotedString(other.to_string()).into())
                }
            },
        };
        Ok(base_expr)
    }

    /// Parse `sql` once. Planning and retries reuse the statement.
    fn parse_planned_statement(sql: &str) -> Result<Statement, KalamDbError> {
        kalamdb_sql::parser::KalamParser::parse_one(sql)
            .map_err(|error| KalamDbError::InvalidSql(error.to_string()))
    }

    /// Plan an already-parsed statement and optimize it for execution.
    async fn dataframe_from_statement(
        session: &SessionContext,
        statement: &Statement,
    ) -> Result<DataFrame, datafusion::error::DataFusionError> {
        let plan = session
            .state()
            .statement_to_plan(DfStatement::Statement(Box::new(statement.clone())))
            .await?;
        session.execute_logical_plan(plan).await
    }

    /// One unoptimized plan for prepared parameter and result metadata.
    pub async fn logical_plan(&self, sql: &str) -> Result<LogicalPlan, KalamDbError> {
        let execution_sql = kalamdb_sql::rewrite_context_functions_for_datafusion(sql);
        let statement = Self::parse_planned_statement(&execution_sql)?;
        self.app_context
            .base_session_context()
            .state()
            .statement_to_plan(DfStatement::Statement(Box::new(statement)))
            .await
            .map_err(Self::datafusion_to_execution_error)
    }

    /// One unoptimized plan for prepared parameter and result metadata using
    /// the caller's prepared statement metadata and session execution context.
    pub async fn logical_plan_for_metadata(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<LogicalPlan, KalamDbError> {
        let session = self.create_session_with_transaction_context(exec_ctx)?;
        if let Some(statement) = metadata.parsed_dml.as_deref() {
            return session
                .state()
                .statement_to_plan(DfStatement::Statement(Box::new(statement.clone())))
                .await
                .map_err(Self::datafusion_to_execution_error);
        }
        let execution_sql =
            kalamdb_sql::rewrite_context_functions_for_datafusion(metadata.sql.as_str());
        let statement = Self::parse_planned_statement(&execution_sql)?;
        session
            .state()
            .statement_to_plan(DfStatement::Statement(Box::new(statement)))
            .await
            .map_err(Self::datafusion_to_execution_error)
    }

    fn try_project_simple_returning_batch(
        table_schema: &SchemaRef,
        batch: &RecordBatch,
        returning: &[sqlparser::ast::SelectItem],
    ) -> Option<(SchemaRef, Vec<RecordBatch>)> {
        if matches!(returning, [sqlparser::ast::SelectItem::Wildcard(_)]) {
            let batches = if batch.num_rows() == 0 {
                Vec::new()
            } else {
                vec![batch.clone()]
            };
            return Some((Arc::clone(table_schema), batches));
        }

        let mut projected_fields = Vec::with_capacity(returning.len());
        let mut projected_columns = Vec::with_capacity(returning.len());

        for item in returning {
            let (ident, alias) = match item {
                sqlparser::ast::SelectItem::UnnamedExpr(sqlparser::ast::Expr::Identifier(
                    ident,
                )) => (ident, None),
                sqlparser::ast::SelectItem::ExprWithAlias {
                    expr: sqlparser::ast::Expr::Identifier(ident),
                    alias,
                } => (ident, Some(alias)),
                _ => return None,
            };

            let col_idx = if ident.quote_style.is_some() {
                table_schema.index_of(&ident.value).ok()?
            } else {
                table_schema.index_of(&ident.value).ok().or_else(|| {
                    table_schema
                        .fields()
                        .iter()
                        .position(|field| field.name().eq_ignore_ascii_case(&ident.value))
                })?
            };

            let source_field = &table_schema.fields()[col_idx];
            let output_field = match alias {
                Some(alias_ident) => {
                    Arc::new(source_field.as_ref().clone().with_name(&alias_ident.value))
                },
                None => Arc::clone(source_field),
            };
            projected_fields.push(output_field);
            projected_columns.push(Arc::clone(batch.column(col_idx)));
        }

        let projected_schema = Arc::new(arrow::datatypes::Schema::new_with_metadata(
            projected_fields,
            table_schema.metadata().clone(),
        ));
        if batch.num_rows() == 0 {
            return Some((projected_schema, Vec::new()));
        }
        let projected_batch =
            RecordBatch::try_new(Arc::clone(&projected_schema), projected_columns).ok()?;
        Some((projected_schema, vec![projected_batch]))
    }

    async fn build_on_conflict_returning_result(
        &self,
        table_id: &TableId,
        returning: &[sqlparser::ast::SelectItem],
        returned_rows: Vec<Row>,
    ) -> Result<ExecutionResult, KalamDbError> {
        let cached_table = self.app_context.schema_registry().get(table_id).ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Table schema not found: {}", table_id))
        })?;
        let table_schema = nullable_arrow_schema(&cached_table.arrow_schema()?);
        let batch = json_rows_to_arrow_batch(&table_schema, returned_rows).map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to build RETURNING rows: {}", error))
        })?;

        if let Some((schema, batches)) =
            Self::try_project_simple_returning_batch(&table_schema, &batch, returning)
        {
            let row_count = batches.iter().map(|b| b.num_rows()).sum();
            return Ok(ExecutionResult::Rows {
                batches,
                row_count,
                schema: Some(schema),
            });
        }

        let session = SessionContext::new_with_config(SessionConfig::new().set_str(
            "datafusion.sql_parser.dialect",
            crate::sql::datafusion_session::SQL_PARSER_DIALECT,
        ));
        let returning_table_name = "__kalamdb_returning";
        let mem_table = MemTable::try_new(table_schema, vec![vec![batch]])
            .map_err(Self::datafusion_to_execution_error)?;
        session
            .register_table(returning_table_name, Arc::new(mem_table))
            .map_err(Self::datafusion_to_execution_error)?;

        let returning_items =
            returning.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
        let returning_sql = format!(
            "SELECT {} FROM {} AS {}",
            returning_items,
            returning_table_name,
            quote_sql_identifier(table_id.table_name().as_str())
        );
        let statement = Self::parse_planned_statement(&returning_sql)?;
        let df = Self::dataframe_from_statement(&session, &statement)
            .await
            .map_err(Self::datafusion_to_execution_error)?;
        let schema = Arc::new(df.schema().as_arrow().clone());
        let batches = df.collect().await.map_err(Self::datafusion_to_execution_error)?;
        let row_count = batches.iter().map(|batch| batch.num_rows()).sum();

        Ok(ExecutionResult::Rows {
            batches,
            row_count,
            schema: Some(schema),
        })
    }

    async fn try_execute_update_or_delete_returning(
        &self,
        sql: &str,
        metadata: &PreparedExecutionStatement,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
        dml_kind: DmlKind,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        let Some(parsed_stmt) = metadata.parsed_dml.as_deref() else {
            return Ok(None);
        };
        let has_returning = match parsed_stmt {
            Statement::Update(u) => u.returning.is_some(),
            Statement::Delete(d) => d.returning.is_some(),
            _ => false,
        };
        if !has_returning {
            return Ok(None);
        }

        let Some(table_id) = metadata.table_id.as_ref() else {
            return Ok(None);
        };

        let execution_sql = kalamdb_sql::rewrite_context_functions_for_datafusion(sql);
        let mut stmt = Self::parse_planned_statement(&execution_sql)?;
        self.evaluate_catalog_functions_in_statement(&mut stmt, exec_ctx, params).await?;

        let (returning, selection_sql, stripped_sql) = match &mut stmt {
            Statement::Update(u) => {
                let Some(ret) = u.returning.take() else {
                    return Ok(None);
                };
                let sel = u.selection.as_ref().map(ToString::to_string);
                (ret, sel, stmt.to_string())
            },
            Statement::Delete(d) => {
                let Some(ret) = d.returning.take() else {
                    return Ok(None);
                };
                let sel = d.selection.as_ref().map(ToString::to_string);
                (ret, sel, stmt.to_string())
            },
            _ => return Ok(None),
        };

        let owned_exec_ctx;
        let dml_exec_ctx = if exec_ctx.request_id().is_some() {
            exec_ctx
        } else {
            owned_exec_ctx =
                exec_ctx.clone().with_request_id(format!("sql-dml-returning-{}", Uuid::now_v7()));
            &owned_exec_ctx
        };

        let should_autocommit = self.should_stage_autocommit_dml(metadata, dml_exec_ctx)?;
        let res = if should_autocommit {
            self.run_with_request_transaction(
                dml_exec_ctx,
                "sql-dml-returning",
                "RETURNING DML",
                |ctx, _tx_id| async move {
                    self.execute_update_or_delete_returning_inner(
                        &stripped_sql,
                        metadata,
                        table_id,
                        &returning,
                        selection_sql.as_deref(),
                        params,
                        ctx,
                        dml_kind,
                    )
                    .await
                },
            )
            .await?
        } else {
            self.execute_update_or_delete_returning_inner(
                &stripped_sql,
                metadata,
                table_id,
                &returning,
                selection_sql.as_deref(),
                params,
                dml_exec_ctx,
                dml_kind,
            )
            .await?
        };
        Ok(Some(res))
    }

    async fn execute_update_or_delete_returning_inner(
        &self,
        stripped_sql: &str,
        metadata: &PreparedExecutionStatement,
        table_id: &TableId,
        returning: &[sqlparser::ast::SelectItem],
        selection_sql: Option<&str>,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
        dml_kind: DmlKind,
    ) -> Result<ExecutionResult, KalamDbError> {
        let qualified_table = format!(
            "{}.{}",
            quote_sql_identifier(table_id.namespace_id().as_str()),
            quote_sql_identifier(table_id.table_name().as_str())
        );
        let pre_select_sql = match selection_sql {
            Some(sel) => format!("SELECT * FROM {} WHERE {}", qualified_table, sel),
            None => format!("SELECT * FROM {}", qualified_table),
        };
        let pre_rows = self.query_table_rows_for_returning(&pre_select_sql, params, exec_ctx).await?;

        if pre_rows.is_empty() {
            return self.build_on_conflict_returning_result(table_id, returning, Vec::new()).await;
        }

        if matches!(dml_kind, DmlKind::Delete) {
            self.execute_dml_via_datafusion_inner(
                stripped_sql,
                metadata,
                params.to_vec(),
                exec_ctx,
                dml_kind,
            )
            .await?;
            return self.build_on_conflict_returning_result(table_id, returning, pre_rows).await;
        }

        let cached_table = self.app_context.schema_registry().get(table_id).ok_or_else(|| {
            KalamDbError::InvalidOperation(format!("Table schema not found: {}", table_id))
        })?;
        let pk_col = cached_table
            .table
            .columns
            .iter()
            .find(|c| c.is_primary_key)
            .map(|c| c.column_name.clone())
            .unwrap_or_else(|| "id".to_string());

        let mut target_pks = Vec::with_capacity(pre_rows.len());
        for row in &pre_rows {
            if let Some(pk_val) = row.values.get(&pk_col) {
                target_pks.push(pk_val.clone());
            }
        }

        self.execute_dml_via_datafusion_inner(
            stripped_sql,
            metadata,
            params.to_vec(),
            exec_ctx,
            dml_kind,
        )
        .await?;

        let (post_select_sql, post_params) = if !target_pks.is_empty()
            && target_pks.len() <= 100
            && target_pks
                .iter()
                .all(|pk| !pk.is_null() && validate_params(std::slice::from_ref(pk)).is_ok())
        {
            let quoted_pk = quote_sql_identifier(&pk_col);
            if target_pks.len() == 1 {
                (
                    format!("SELECT * FROM {} WHERE {} = $1", qualified_table, quoted_pk),
                    target_pks.clone(),
                )
            } else {
                let placeholders = (1..=target_pks.len())
                    .map(|idx| format!("${idx}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    format!(
                        "SELECT * FROM {} WHERE {} IN ({})",
                        qualified_table, quoted_pk, placeholders
                    ),
                    target_pks.clone(),
                )
            }
        } else {
            (format!("SELECT * FROM {}", qualified_table), Vec::new())
        };
        let all_post_rows =
            self.query_table_rows_for_returning(&post_select_sql, &post_params, exec_ctx).await?;
        let mut post_by_pk = HashMap::with_capacity(all_post_rows.len());
        for row in all_post_rows {
            if let Some(pk_val) = row.values.get(&pk_col) {
                post_by_pk.insert(pk_val.to_string(), row);
            }
        }

        let mut returned_rows = Vec::with_capacity(target_pks.len());
        for pk in target_pks {
            if let Some(row) = post_by_pk.remove(&pk.to_string()) {
                returned_rows.push(row);
            }
        }

        self.build_on_conflict_returning_result(table_id, returning, returned_rows).await
    }

    async fn query_table_rows_for_returning(
        &self,
        select_sql: &str,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
    ) -> Result<Vec<Row>, KalamDbError> {
        let session = self.create_session_with_transaction_context(exec_ctx)?;
        let stmt = Self::parse_planned_statement(select_sql)?;
        let df = Self::dataframe_from_statement(&session, &stmt)
            .await
            .map_err(Self::datafusion_to_execution_error)?;
        let df = if params.is_empty() {
            df
        } else {
            let bound = replace_placeholders_in_plan(df.logical_plan().clone(), params)?;
            session
                .execute_logical_plan(bound)
                .await
                .map_err(Self::datafusion_to_execution_error)?
        };
        let batches = df.collect().await.map_err(Self::datafusion_to_execution_error)?;
        let mut rows = Vec::new();
        for batch in &batches {
            let schema = batch.schema();
            for row_idx in 0..batch.num_rows() {
                let mut values = std::collections::BTreeMap::new();
                for (col_idx, field) in schema.fields().iter().enumerate() {
                    if field.name().starts_with('_') {
                        continue;
                    }
                    let val = arrow_value_to_scalar(batch.column(col_idx).as_ref(), row_idx)
                        .map_err(Self::datafusion_to_execution_error)?;
                    values.insert(field.name().clone(), val);
                }
                rows.push(Row::new(values));
            }
        }
        Ok(rows)
    }

    /// True when the plan already encodes an explicit row bound.
    ///
    /// DataFusion often rewrites `ORDER BY ... LIMIT N` to `Sort { fetch: Some(N) }`
    /// (and may push `LIMIT` into `TableScan.fetch`) without leaving a top-level
    /// `Limit` node. Treat those fetch bounds as explicit so we do not also apply
    /// `default_query_limit`.
    fn logical_plan_has_limit(plan: &LogicalPlan) -> bool {
        match plan {
            LogicalPlan::Limit(_) => true,
            LogicalPlan::Sort(sort) if sort.fetch.is_some() => true,
            LogicalPlan::TableScan(scan) if scan.fetch.is_some() => true,
            _ => plan.inputs().iter().any(|input| Self::logical_plan_has_limit(input)),
        }
    }

    fn logical_plan_is_client_catalog_introspection(plan: &LogicalPlan) -> bool {
        let mut saw_scan = false;
        let mut only_catalog = true;
        let walked = plan.apply(|node| {
            if let LogicalPlan::TableScan(scan) = node {
                saw_scan = true;
                if !table_ref_is_client_catalog(&scan.table_name) {
                    only_catalog = false;
                }
            }
            Ok(TreeNodeRecursion::Continue)
        });
        walked.is_ok() && saw_scan && only_catalog
    }

    fn apply_select_limits(
        &self,
        df: datafusion::dataframe::DataFrame,
        sql: &str,
    ) -> Result<datafusion::dataframe::DataFrame, KalamDbError> {
        let max_query_limit = self.app_context.config().limits.max_query_limit;
        let default_query_limit = self.app_context.config().limits.default_query_limit;
        let has_explicit_limit = Self::logical_plan_has_limit(df.logical_plan());

        if !has_explicit_limit && default_query_limit > 0 {
            if !Self::logical_plan_is_client_catalog_introspection(df.logical_plan()) {
                let effective_default_limit = if max_query_limit > 0 {
                    default_query_limit.min(max_query_limit)
                } else {
                    default_query_limit
                };

                log::debug!(
                    target: "sql::exec",
                    "Applying default query limit {} to unbounded SELECT | sql='{}'",
                    effective_default_limit,
                    sql
                );

                return df
                    .limit(0, Some(effective_default_limit))
                    .map_err(Self::datafusion_to_execution_error);
            }
        }

        if max_query_limit > 0 {
            return df.limit(0, Some(max_query_limit)).map_err(Self::datafusion_to_execution_error);
        }

        Ok(df)
    }

    fn dml_operation_name(dml_kind: DmlKind) -> &'static str {
        match dml_kind {
            DmlKind::Insert => "INSERT",
            DmlKind::Update => "UPDATE",
            DmlKind::Delete => "DELETE",
        }
    }

    fn block_system_namespace_dml(
        &self,
        table_id: Option<&TableId>,
        dml_kind: DmlKind,
    ) -> Result<(), KalamDbError> {
        let Some(table_id) = table_id else {
            return Ok(());
        };

        if table_id.namespace_id().is_system_namespace()
            && SystemTable::from_name(table_id.table_name().as_str())
                .map(|table| table != SystemTable::Migrations)
                .unwrap_or(true)
        {
            let op = Self::dml_operation_name(dml_kind);
            return Err(KalamDbError::InvalidOperation(format!(
                "Cannot {} system table '{}.{}'",
                op.to_lowercase(),
                table_id.namespace_id().as_str(),
                table_id.table_name().as_str()
            )));
        }
        Ok(())
    }

    fn map_classification_error(
        err: kalamdb_sql::classifier::StatementClassificationError,
    ) -> KalamDbError {
        match err {
            kalamdb_sql::classifier::StatementClassificationError::Unauthorized(msg) => {
                KalamDbError::Unauthorized(msg)
            },
            kalamdb_sql::classifier::StatementClassificationError::InvalidSql {
                sql: _,
                message,
            } => KalamDbError::InvalidSql(message),
        }
    }

    fn is_ddl_statement(kind: &SqlStatementKind) -> bool {
        matches!(
            kind,
            SqlStatementKind::AlterSchema(_)
                | SqlStatementKind::DropSchema(_)
                | SqlStatementKind::CreateStorage(_)
                | SqlStatementKind::AlterStorage(_)
                | SqlStatementKind::DropStorage(_)
                | SqlStatementKind::CreateTable(_)
                | SqlStatementKind::CreateView(_)
                | SqlStatementKind::AlterTable(_)
                | SqlStatementKind::DropTable(_)
                | SqlStatementKind::CreatePolicy(_)
                | SqlStatementKind::AlterPolicy(_)
                | SqlStatementKind::DropPolicy(_)
                | SqlStatementKind::CreateType(_)
                | SqlStatementKind::AlterType(_)
                | SqlStatementKind::DropType(_)
                | SqlStatementKind::CreateDomain(_)
                | SqlStatementKind::AlterDomain(_)
                | SqlStatementKind::DropDomain(_)
                | SqlStatementKind::CreateProcedure(_)
                | SqlStatementKind::AlterRoutine(_)
                | SqlStatementKind::DropProcedure(_)
                | SqlStatementKind::CommentOn(_)
                | SqlStatementKind::CreateTrigger(_)
                | SqlStatementKind::CreateSchedule(_)
                | SqlStatementKind::DropTrigger(_)
                | SqlStatementKind::DropSchedule(_)
                | SqlStatementKind::AlterTrigger(_)
                | SqlStatementKind::AlterSchedule(_)
                | SqlStatementKind::GrantExecute(_)
                | SqlStatementKind::RevokeExecute(_)
                | SqlStatementKind::CreateSchema(_)
        )
    }

    fn request_transaction_state<'a>(
        &self,
        exec_ctx: &'a ExecutionContext,
    ) -> Result<Option<RequestTransactionState<'a>>, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id());
        if let Some(state) = request_state.as_mut() {
            state.sync(&request_transaction_coordinator);
        }
        Ok(request_state)
    }

    fn active_request_transaction_id(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<TransactionId>, KalamDbError> {
        if let Some(tx_id) = exec_ctx.transaction_id() {
            return Ok(Some(tx_id.clone()));
        }
        Ok(self
            .request_transaction_state(exec_ctx)?
            .and_then(|state| state.active_transaction_id().cloned()))
    }

    async fn resolve_prepared_table_type(
        &self,
        metadata: &PreparedExecutionStatement,
    ) -> Result<Option<TableType>, KalamDbError> {
        if let Some(table_type) = metadata.table_type {
            return Ok(Some(table_type));
        }

        let Some(table_id) = metadata.table_id.as_ref() else {
            return Ok(None);
        };

        if let Some(cached) = self.app_context.schema_registry().get(table_id) {
            return Ok(Some(cached.table_entry().table_type));
        }

        Ok(self
            .app_context
            .schema_registry()
            .get_table_if_exists_async(table_id)
            .await?
            .map(|table_def| table_def.table_type))
    }

    fn transaction_query_context_for_request(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<TransactionQueryContext>, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(transaction_id) => Some(transaction_id),
            None => self.active_request_transaction_id(exec_ctx)?,
        };
        let Some(transaction_id) = transaction_id else {
            return Ok(None);
        };

        let coordinator = self.app_context.transaction_coordinator();
        let handle = coordinator.get_handle(&transaction_id).ok_or_else(|| {
            KalamDbError::InvalidOperation(format!(
                "active SQL transaction '{}' has no handle",
                transaction_id
            ))
        })?;

        if !handle.state.is_open() {
            return Err(KalamDbError::InvalidOperation(format!(
                "transaction '{}' is {}",
                transaction_id, handle.state
            )));
        }

        Ok(Some(TransactionQueryContext::new(
            transaction_id.clone(),
            Arc::clone(&handle.snapshot_log_index),
            Arc::new(crate::transactions::CoordinatorOverlayView::new(
                Arc::clone(&coordinator),
                transaction_id.clone(),
            )),
            Arc::new(crate::transactions::CoordinatorMutationSink::new(coordinator)),
            Arc::new(CoordinatorAccessValidator::new(self.app_context.transaction_coordinator())),
        )))
    }

    fn user_session_state(
        exec_ctx: &ExecutionContext,
        transaction: Option<TransactionQueryContext>,
    ) -> SessionState {
        let mut state = exec_ctx.build_user_session_state();
        if let Some(transaction) = transaction {
            install_extension(&mut state, TransactionQueryExtension::new(transaction));
        }
        state
    }

    fn create_session_with_transaction_context(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<SessionContext, KalamDbError> {
        let Some(transaction) = self.transaction_query_context_for_request(exec_ctx)? else {
            return Ok(exec_ctx.create_session_with_user());
        };
        Ok(SessionContext::new_with_state(Self::user_session_state(
            exec_ctx,
            Some(transaction),
        )))
    }

    fn point_read_session_state(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<Arc<SessionState>, KalamDbError> {
        if let Some(transaction) = self.transaction_query_context_for_request(exec_ctx)? {
            return Ok(Arc::new(Self::user_session_state(exec_ctx, Some(transaction))));
        }

        let key = PointReadSessionCacheKey::new(
            exec_ctx.user_id().clone(),
            exec_ctx.user_role(),
            exec_ctx.default_namespace(),
            exec_ctx.read_context(),
        );
        if let Some(state) = self.point_read_session_cache.get(&key) {
            return Ok(state);
        }

        let state = Arc::new(Self::user_session_state(exec_ctx, None));
        self.point_read_session_cache.insert(key, Arc::clone(&state));
        Ok(state)
    }

    fn apply_set_search_path(
        &self,
        stmt: &kalamdb_sql::ddl::SetSearchPathStatement,
        exec_ctx: &ExecutionContext,
    ) {
        let target_schema = stmt
            .schemas
            .iter()
            .find_map(|schema| {
                if schema.as_str() == "$user" {
                    return None;
                }
                if schema.is_default_namespace()
                    || matches!(schema.as_str(), "pg_catalog" | "information_schema" | "system")
                {
                    return Some(schema.clone());
                }
                if self
                    .app_context
                    .system_tables()
                    .namespaces()
                    .get_namespace(schema)
                    .ok()
                    .flatten()
                    .is_some()
                {
                    return Some(schema.clone());
                }
                Some(schema.clone())
            })
            .unwrap_or_else(|| SchemaId::new("default"));

        let session = exec_ctx.create_session_with_user();
        session
            .state_ref()
            .write()
            .config_mut()
            .options_mut()
            .catalog
            .default_schema = target_schema.as_str().to_string();
    }

    async fn abort_active_transaction_on_statement_failure(
        &self,
        classified: &SqlStatement,
        metadata: &PreparedExecutionStatement,
        error: &KalamDbError,
        exec_ctx: &ExecutionContext,
    ) {
        if Self::is_ddl_statement(classified.kind()) {
            return;
        }
        if let Ok(Some(TableType::Stream | TableType::System)) =
            self.resolve_prepared_table_type(metadata).await
        {
            return;
        }
        if error.to_string().contains("is already bound to data raft group") {
            return;
        }
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => Some(tx_id),
            None => self.active_request_transaction_id(exec_ctx).ok().flatten(),
        };
        if let Some(transaction_id) = transaction_id {
            self.app_context
                .transaction_coordinator()
                .mark_transaction_aborted(&transaction_id);
        }
    }

    fn ensure_active_transaction_not_aborted(
        &self,
        classified: &SqlStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        if matches!(
            classified.kind(),
            SqlStatementKind::RollbackTransaction | SqlStatementKind::RollbackToSavepoint(_)
        ) {
            return Ok(());
        }
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => Some(tx_id),
            None => self.active_request_transaction_id(exec_ctx).ok().flatten(),
        };
        if let Some(transaction_id) = transaction_id {
            if let Some(handle) =
                self.app_context.transaction_coordinator().get_handle(&transaction_id)
            {
                if matches!(handle.state, kalamdb_commons::models::TransactionState::Aborted) {
                    return Err(KalamDbError::InvalidOperation(
                        "current transaction is aborted, commands ignored until end of transaction block (SQLSTATE 25P02)"
                            .to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn execute_savepoint(
        &self,
        name: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => tx_id,
            None => self.active_request_transaction_id(exec_ctx)?.ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "SAVEPOINT can only be used in transaction blocks".to_string(),
                )
            })?,
        };
        self.app_context.transaction_coordinator().savepoint(&transaction_id, name)?;
        Ok(ExecutionResult::Success {
            message: "SAVEPOINT".to_string(),
        })
    }

    fn execute_release_savepoint(
        &self,
        name: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => tx_id,
            None => self.active_request_transaction_id(exec_ctx)?.ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "RELEASE SAVEPOINT can only be used in transaction blocks".to_string(),
                )
            })?,
        };
        self.app_context
            .transaction_coordinator()
            .release_savepoint(&transaction_id, name)?;
        Ok(ExecutionResult::Success {
            message: "RELEASE".to_string(),
        })
    }

    fn execute_rollback_to_savepoint(
        &self,
        name: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let transaction_id = match exec_ctx.transaction_id().cloned() {
            Some(tx_id) => tx_id,
            None => self.active_request_transaction_id(exec_ctx)?.ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "ROLLBACK TO SAVEPOINT can only be used in transaction blocks".to_string(),
                )
            })?,
        };
        self.app_context
            .transaction_coordinator()
            .rollback_to_savepoint(&transaction_id, name)?;
        Ok(ExecutionResult::Success {
            message: "ROLLBACK".to_string(),
        })
    }

    async fn execute_begin_transaction(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "BEGIN requires a request-scoped execution context".to_string(),
                )
            })?;
        request_state.sync(&request_transaction_coordinator);
        let transaction_id = request_state
            .begin(&request_transaction_coordinator)
            .map_err(map_request_transaction_error)?;
        Ok(ExecutionResult::Success {
            message: format!("Transaction started ({})", transaction_id),
        })
    }

    async fn execute_commit_transaction(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "COMMIT requires a request-scoped execution context".to_string(),
                )
            })?;
        request_state.sync(&request_transaction_coordinator);
        let transaction_id = request_state
            .commit(&request_transaction_coordinator)
            .await
            .map_err(map_request_transaction_error)?;
        Ok(ExecutionResult::Success {
            message: format!("Transaction committed ({})", transaction_id),
        })
    }

    fn execute_rollback_transaction(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "ROLLBACK requires a request-scoped execution context".to_string(),
                )
            })?;
        request_state.sync(&request_transaction_coordinator);
        let transaction_id = request_state
            .rollback(&request_transaction_coordinator)
            .map_err(map_request_transaction_error)?;
        Ok(ExecutionResult::Success {
            message: format!("Transaction rolled back ({})", transaction_id),
        })
    }

    fn reject_ddl_in_active_request_transaction(
        &self,
        classified: &SqlStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        if !Self::is_ddl_statement(classified.kind()) {
            return Ok(());
        }

        if let Some(transaction_id) = self.active_request_transaction_id(exec_ctx)? {
            self.app_context
                .transaction_coordinator()
                .reject_ddl_in_transaction(&transaction_id)?;
        }

        Ok(())
    }

    async fn reject_unsupported_dml_in_active_request_transaction(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<(), KalamDbError> {
        let Some(transaction_id) = self.active_request_transaction_id(exec_ctx)? else {
            return Ok(());
        };

        match self.resolve_prepared_table_type(metadata).await? {
            Some(TableType::Stream) => Err(KalamDbError::InvalidOperation(format!(
                "transaction '{}' failed: stream tables are not supported inside explicit \
                 transactions",
                transaction_id
            ))),
            Some(TableType::System) => Err(KalamDbError::InvalidOperation(format!(
                "transaction '{}' failed: system tables are not supported inside explicit \
                 transactions",
                transaction_id
            ))),
            _ => Ok(()),
        }
    }

    async fn should_autocommit_stream_dml(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<bool, KalamDbError> {
        if !exec_ctx.allows_stream_autocommit() {
            return Ok(false);
        }
        if matches!(self.resolve_prepared_table_type(metadata).await?, Some(TableType::Stream)) {
            return Ok(true);
        }
        self.is_stream_target_sql(metadata.sql.as_str(), metadata.table_id.as_ref(), exec_ctx)
            .await
    }

    async fn is_stream_target_sql(
        &self,
        sql: &str,
        table_id: Option<&TableId>,
        exec_ctx: &ExecutionContext,
    ) -> Result<bool, KalamDbError> {
        if !exec_ctx.allows_stream_autocommit() {
            return Ok(false);
        }
        let Some(table_id) = table_id
            .cloned()
            .or_else(|| extract_sql_target_table_id(sql, &exec_ctx.default_namespace()))
        else {
            return Ok(false);
        };
        Ok(self
            .app_context
            .schema_registry()
            .get_table_if_exists_async(&table_id)
            .await?
            .is_some_and(|table| table.table_type == TableType::Stream))
    }

    fn isolated_stream_exec_ctx(exec_ctx: &ExecutionContext) -> ExecutionContext {
        // Procedure SQL runs on the meta leader, which is often a follower of the
        // user shard. Stream DML waits until that shard applies the entry locally
        // before returning, so the read in the same procedure must use that replica
        // instead of failing the leader check.
        exec_ctx
            .clone()
            .without_transaction_id()
            .with_request_id(format!("stream-{}", Uuid::now_v7()))
            .with_read_context(ReadContext::Internal)
    }

    async fn execute_table_dml(
        &self,
        sql: &str,
        metadata: &PreparedExecutionStatement,
        params: Vec<ScalarValue>,
        exec_ctx: &ExecutionContext,
        dml_kind: DmlKind,
    ) -> Result<ExecutionResult, KalamDbError> {
        let isolated;
        let dml_ctx = if self.should_autocommit_stream_dml(metadata, exec_ctx).await? {
            isolated = Self::isolated_stream_exec_ctx(exec_ctx);
            &isolated
        } else {
            exec_ctx
        };

        self.reject_unsupported_dml_in_active_request_transaction(metadata, dml_ctx)
            .await?;

        if matches!(dml_kind, DmlKind::Update | DmlKind::Delete) {
            if let Some(result) = self
                .try_execute_update_or_delete_returning(sql, metadata, &params, dml_ctx, dml_kind)
                .await?
            {
                return Ok(result);
            }
        }

        if matches!(dml_kind, DmlKind::Insert) {
            match kalamdb_observability::kdb_await_in_info_span!(
                self.try_execute_literal_insert_via_applier(sql, metadata, dml_ctx, &params),
                "sql.insert_literal"
            ) {
                Ok(Some(result)) => Ok(result),
                Ok(None) => {
                    self.execute_dml_via_datafusion(sql, metadata, params, dml_ctx, dml_kind).await
                },
                Err(error) => Err(error),
            }
        } else {
            self.execute_dml_via_datafusion(sql, metadata, params, dml_ctx, dml_kind).await
        }
    }

    /// Construct a new executor with a pre-built handler registry.
    pub fn new(
        app_context: std::sync::Arc<crate::app_context::AppContext>,
        handler_registry: Arc<HandlerRegistry>,
    ) -> Self {
        let plan_max_entries = app_context.config().execution.sql_plan_cache_max_entries;
        let plan_idle_ttl =
            Duration::from_secs(app_context.config().execution.sql_plan_cache_ttl_seconds);
        let sql_cache_registry = Arc::new(SqlCacheRegistry::new(SqlCacheRegistryConfig::new(
            plan_max_entries,
            plan_idle_ttl,
        )));
        let prepared_statement_cache = moka::sync::Cache::builder()
            .max_capacity(plan_max_entries)
            .time_to_idle(plan_idle_ttl)
            .build();
        let point_read_session_cache = moka::sync::Cache::builder()
            .max_capacity(plan_max_entries.min(64))
            .time_to_idle(plan_idle_ttl)
            .build();
        Self {
            app_context,
            handler_registry,
            sql_cache_registry,
            prepared_statement_cache,
            point_read_session_cache,
            #[cfg(test)]
            point_get_fast_path_hits: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Clear SQL caches that may become stale after DDL operations.
    pub fn clear_plan_cache(&self) {
        self.sql_cache_registry.clear();
        self.prepared_statement_cache.invalidate_all();
        self.point_read_session_cache.invalidate_all();
        self.prepared_statement_cache.run_pending_tasks();
        self.point_read_session_cache.run_pending_tasks();
    }

    /// Get current plan cache size (diagnostics/testing)
    pub fn plan_cache_len(&self) -> usize {
        self.sql_cache_registry.plan_cache().len()
    }

    /// Get current prepared-statement metadata cache size (diagnostics/testing).
    pub fn prepared_statement_cache_len(&self) -> usize {
        self.prepared_statement_cache.run_pending_tasks();
        self.prepared_statement_cache.entry_count() as usize
    }

    #[cfg(test)]
    pub fn point_read_session_cache_len(&self) -> usize {
        self.point_read_session_cache.run_pending_tasks();
        self.point_read_session_cache.entry_count() as usize
    }

    #[cfg(test)]
    pub fn point_get_fast_path_hits(&self) -> u64 {
        self.point_get_fast_path_hits.load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn optimized_plan_for_cache(
        &self,
        session: &SessionContext,
        data_frame: &DataFrame,
        default_namespace: &SchemaId,
    ) -> Result<LogicalPlan, KalamDbError> {
        let ordered = apply_default_order_by(
            data_frame.logical_plan().clone(),
            &self.app_context,
            default_namespace,
        )
        .await?;
        session.state().optimize(&ordered).map_err(Self::datafusion_to_execution_error)
    }

    fn unqualify_scan_filter(filter: DataFusionExpr) -> Result<DataFusionExpr, KalamDbError> {
        filter
            .transform_up(|expr| {
                if let DataFusionExpr::Column(mut column) = expr {
                    column.relation = None;
                    Ok(Transformed::yes(DataFusionExpr::Column(column)))
                } else {
                    Ok(Transformed::no(expr))
                }
            })
            .data()
            .map_err(Self::datafusion_to_execution_error)
    }

    /// Peel default ORDER BY wrappers only.
    ///
    /// Projection is the user-requested output schema. Dropping it makes
    /// `SELECT file_ref WHERE path = $1` return `path` as the first column.
    fn unwrap_default_order_wrappers(plan: &LogicalPlan) -> &LogicalPlan {
        match plan {
            LogicalPlan::Sort(sort) => Self::unwrap_default_order_wrappers(sort.input.as_ref()),
            LogicalPlan::Limit(limit) => Self::unwrap_default_order_wrappers(limit.input.as_ref()),
            other => other,
        }
    }

    fn project_point_get_batches(
        batches: Vec<RecordBatch>,
        source_schema: SchemaRef,
        target_schema: SchemaRef,
    ) -> Option<Vec<RecordBatch>> {
        let mut indices = Vec::with_capacity(target_schema.fields().len());
        for field in target_schema.fields() {
            indices.push(source_schema.index_of(field.name()).ok()?);
        }
        if indices.len() == source_schema.fields().len()
            && indices.iter().copied().eq(0..indices.len())
        {
            return Some(batches);
        }
        if batches.is_empty() {
            return Some(vec![RecordBatch::new_empty(target_schema)]);
        }
        batches.into_iter().map(|batch| batch.project(&indices).ok()).collect()
    }

    /// Source column name for a cached point-get projection expr.
    ///
    /// Only column refs (plus Alias/Cast wrappers) can skip Arrow. Computed
    /// projections fall back to DataFusion.
    fn point_get_source_column_name(expr: &DataFusionExpr) -> Option<&str> {
        match expr {
            DataFusionExpr::Column(column) => Some(column.name.as_str()),
            DataFusionExpr::Alias(alias) => Self::point_get_source_column_name(alias.expr.as_ref()),
            DataFusionExpr::Cast(cast) => Self::point_get_source_column_name(cast.expr.as_ref()),
            DataFusionExpr::TryCast(try_cast) => {
                Self::point_get_source_column_name(try_cast.expr.as_ref())
            },
            _ => None,
        }
    }

    fn project_point_get_scalar_rows(
        mut rows: Vec<Row>,
        source_schema: &SchemaRef,
        target_schema: SchemaRef,
        projection_exprs: Option<&[DataFusionExpr]>,
    ) -> Option<Vec<Row>> {
        let names_already_match = source_schema.fields().len() == target_schema.fields().len()
            && source_schema
                .fields()
                .iter()
                .zip(target_schema.fields())
                .all(|(left, right)| left.name() == right.name());
        if names_already_match && projection_exprs.is_none() {
            return Some(rows);
        }

        // Scan rows keep catalog names (`name`). The logical Projection may
        // alias them (`title`) and the scan may still carry extra columns
        // because `id = $1` is Exact. Looking up `title` in the unmapped Row
        // yields Null in JSON. Use projection exprs when present; otherwise
        // positional remap only if field counts match. Missing mappings return
        // None so we fall back to Arrow instead of inventing Nulls.
        // Regression: `cached_point_get_projects_non_pk_columns_across_table_types`.
        let source_names: Vec<&str> = if let Some(exprs) = projection_exprs {
            if exprs.len() != target_schema.fields().len() {
                return None;
            }
            exprs
                .iter()
                .map(Self::point_get_source_column_name)
                .collect::<Option<Vec<_>>>()?
        } else if source_schema.fields().len() == target_schema.fields().len() {
            source_schema.fields().iter().map(|field| field.name().as_str()).collect()
        } else {
            target_schema.fields().iter().map(|field| field.name().as_str()).collect()
        };

        for row in &mut rows {
            let mut projected = Row::new(std::collections::BTreeMap::new());
            for (target_field, source_name) in target_schema.fields().iter().zip(&source_names) {
                let value = row
                    .values
                    .remove(*source_name)
                    .or_else(|| row.values.remove(target_field.name()));
                let Some(value) = value else {
                    return None;
                };
                projected.values.insert(target_field.name().clone(), value);
            }
            *row = projected;
        }
        Some(rows)
    }

    /// Execute a cached, optimized single-PK table scan without rebuilding a
    /// DataFusion logical/physical plan. Provider `scan` still enforces access,
    /// leader routing, transaction snapshots/overlays, MVCC, and tombstones.
    ///
    /// Placeholders are bound on the scan filters only — the cached template
    /// plan is not cloned.
    async fn try_execute_cached_point_get(
        &self,
        plan: &LogicalPlan,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        let (scan, requested_schema, projection_exprs) =
            match Self::unwrap_default_order_wrappers(plan) {
                LogicalPlan::TableScan(scan) => (scan, None, None),
                LogicalPlan::Projection(projection) => {
                    let LogicalPlan::TableScan(scan) =
                        Self::unwrap_default_order_wrappers(projection.input.as_ref())
                    else {
                        return Ok(None);
                    };
                    (
                        scan,
                        Some(Arc::new(projection.schema.as_arrow().clone())),
                        // Needed so `name AS title` remaps scan-row keys.
                        Some(projection.expr.as_slice()),
                    )
                },
                _ => return Ok(None),
            };

        let default_ns = exec_ctx.default_namespace();
        let namespace = match scan.table_name.schema() {
            Some(schema) => SchemaId::from_session_schema(schema, Some(&default_ns)),
            None => default_ns,
        };
        let table_id = TableId::from_namespace(namespace, scan.table_name.table());
        let Some(cached_table) = self.app_context.schema_registry().get(&table_id) else {
            return Ok(None);
        };
        if !matches!(
            cached_table.table.table_type,
            TableType::User | TableType::Shared | TableType::Stream
        ) {
            return Ok(None);
        }

        let primary_keys = cached_table.table.get_primary_key_columns();
        let [primary_key] = primary_keys.as_slice() else {
            return Ok(None);
        };

        let mut filters = Vec::with_capacity(scan.filters.len());
        {
            let _span = kalamdb_observability::kdb_info_span_entered!("sql.point_get.bind");
            for filter in &scan.filters {
                let bound = match bind_placeholders_in_expr(filter.clone(), params) {
                    Ok(expr) => expr,
                    Err(_) => return Ok(None),
                };
                filters.push(Self::unqualify_scan_filter(bound)?);
            }
            if !filters.iter().any(|filter| {
                kalamdb_tables::utils::base::extract_pk_equality_literal(filter, primary_key)
                    .is_some()
            }) {
                return Ok(None);
            }
        }

        let Some(provider) = cached_table.get_provider() else {
            return Ok(None);
        };
        let state = {
            let _span = kalamdb_observability::kdb_info_span_entered!("sql.point_get.session");
            self.point_read_session_state(exec_ctx)?
        };
        let limit = Some(scan.fetch.unwrap_or(1).min(1));
        let physical_plan = kalamdb_observability::kdb_await_in_info_span!(
            provider.scan(state.as_ref(), scan.projection.as_ref(), &filters, limit),
            "sql.point_get.scan"
        )
        .map_err(Self::datafusion_to_execution_error)?;
        // HTTP `/v1/api/sql` serializes ScalarRows without Arrow. If this
        // returns None for a simple `pk = $1` scan, the bake-off falls back
        // to RecordBatch + JSON and regresses (~1.6s / ~25µs p50 on 1M reads).
        // Keep this Some-path; do not scan again on success.
        if let Some(deferred) = physical_plan.downcast_ref::<DeferredBatchExec>() {
            if let Some((schema, rows)) = kalamdb_observability::kdb_await_in_info_span!(
                deferred.produce_scalar_rows_direct(),
                "sql.point_get.scalar"
            )
            .map_err(Self::datafusion_to_execution_error)?
            {
                let (rows, schema) = if let Some(target_schema) = requested_schema {
                    let _span =
                        kalamdb_observability::kdb_info_span_entered!("sql.point_get.project");
                    let Some(projected) = Self::project_point_get_scalar_rows(
                        rows,
                        &schema,
                        Arc::clone(&target_schema),
                        projection_exprs,
                    ) else {
                        return Ok(None);
                    };
                    (projected, target_schema)
                } else {
                    (rows, schema)
                };
                let row_count = rows.len();
                #[cfg(test)]
                self.point_get_fast_path_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(Some(ExecutionResult::ScalarRows {
                    rows,
                    row_count,
                    schema,
                }));
            }
        }
        let schema = physical_plan.schema();
        let batches = if let Some(deferred) = physical_plan.downcast_ref::<DeferredBatchExec>() {
            vec![deferred
                .produce_batch_direct()
                .await
                .map_err(Self::datafusion_to_execution_error)?]
        } else {
            collect(physical_plan, state.task_ctx())
                .await
                .map_err(Self::datafusion_to_execution_error)?
        };
        let (batches, schema) = if let Some(target_schema) = requested_schema {
            let Some(projected) =
                Self::project_point_get_batches(batches, schema, Arc::clone(&target_schema))
            else {
                return Ok(None);
            };
            let schema = projected.first().map(|batch| batch.schema()).unwrap_or(target_schema);
            (projected, schema)
        } else {
            (batches, schema)
        };
        let row_count = batches.iter().map(|batch| batch.num_rows()).sum();

        #[cfg(test)]
        self.point_get_fast_path_hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        Ok(Some(ExecutionResult::Rows {
            batches,
            row_count,
            schema: Some(schema),
        }))
    }

    /// Batch-execute multiple INSERT statements targeting the same table in an
    /// active explicit transaction via the transaction batch insert path.
    ///
    /// Returns `Ok(Some(results))` with per-statement `ExecutionResult::Inserted`,
    /// `Ok(None)` if the batch path is not applicable (caller should fall back to
    /// per-statement execution), or `Err(e)` on execution failure.
    pub async fn try_batch_insert_in_transaction(
        &self,
        statements: &[&PreparedExecutionStatement],
        exec_ctx: &ExecutionContext,
        transaction_id: &TransactionId,
    ) -> Result<Option<Vec<crate::sql::ExecutionResult>>, KalamDbError> {
        let parsed_stmts: Option<Vec<&sqlparser::ast::Statement>> =
            statements.iter().map(|statement| statement.parsed_dml.as_deref()).collect();
        let parsed_stmts = match parsed_stmts {
            Some(stmts) => stmts,
            None => return Ok(None),
        };

        let table_id = match statements[0].table_id.as_ref() {
            Some(id) => id,
            None => return Ok(None),
        };

        match super::transaction_batch_insert::try_batch_inserts_in_transaction(
            &parsed_stmts,
            Arc::clone(&self.app_context),
            self.sql_cache_registry.as_ref(),
            exec_ctx,
            table_id,
            transaction_id,
        )
        .await?
        {
            Some(counts) => Ok(Some(
                counts
                    .into_iter()
                    .map(|rows_affected| crate::sql::ExecutionResult::Inserted { rows_affected })
                    .collect(),
            )),
            None => Ok(None),
        }
    }

    pub fn prepare_statement_metadata(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<PreparedExecutionStatement, StatementClassificationError> {
        self.prepare_statement_metadata_for_role_with_options(
            sql,
            &exec_ctx.default_namespace(),
            exec_ctx.user_role(),
            true,
        )
    }

    /// Classify SQL and resolve DML table metadata without retaining a sqlparser AST.
    ///
    /// Used for FILE() placeholder SQL that will be fully prepared again after substitution.
    pub fn prepare_statement_metadata_light(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<PreparedExecutionStatement, StatementClassificationError> {
        self.prepare_statement_metadata_for_role_with_options(
            sql,
            &exec_ctx.default_namespace(),
            exec_ctx.user_role(),
            false,
        )
    }

    pub fn prepare_statement_metadata_for_role(
        &self,
        sql: &str,
        default_namespace: &SchemaId,
        role: Role,
    ) -> Result<PreparedExecutionStatement, StatementClassificationError> {
        self.prepare_statement_metadata_for_role_with_options(sql, default_namespace, role, true)
    }

    fn prepare_statement_metadata_for_role_with_options(
        &self,
        sql: &str,
        default_namespace: &SchemaId,
        role: Role,
        include_dml_ast: bool,
    ) -> Result<PreparedExecutionStatement, StatementClassificationError> {
        let cache_key = (PlanCacheKey::new(default_namespace.clone(), role, sql), include_dml_ast);
        if let Some(prepared) = self.prepared_statement_cache.get(&cache_key) {
            return Ok(prepared);
        }

        let classified = SqlStatement::classify_and_parse(sql, default_namespace, role)?;
        let (table_id, parsed_dml) = if include_dml_ast {
            Self::parse_dml_metadata(sql, classified.kind(), default_namespace)?
        } else {
            Self::extract_dml_table_id_only(sql, classified.kind(), default_namespace)
        };
        let table_type = table_id.as_ref().and_then(|table_id| {
            self.app_context
                .schema_registry()
                .get(table_id)
                .map(|cached| cached.table_entry().table_type)
        });

        let track_slow_query = classified.is_slow_query_trackable();

        let prepared = PreparedExecutionStatement::new(
            sql.to_string(),
            table_id,
            table_type,
            Some(classified),
            track_slow_query,
            parsed_dml,
        );
        self.prepared_statement_cache.insert(cache_key, prepared.clone());
        Ok(prepared)
    }

    fn parse_dml_metadata(
        sql: &str,
        kind: &SqlStatementKind,
        default_namespace: &SchemaId,
    ) -> Result<(Option<TableId>, Option<Statement>), StatementClassificationError> {
        match kind {
            SqlStatementKind::Insert(_)
            | SqlStatementKind::Update(_)
            | SqlStatementKind::Delete(_) => {
                let statement =
                    kalamdb_sql::parser::KalamParser::parse_one(sql).map_err(|error| {
                        StatementClassificationError::InvalidSql {
                            sql:     sql.to_string(),
                            message: error.to_string(),
                        }
                    })?;
                let table_id =
                    kalamdb_sql::extract_dml_table_id_from_statement(&statement, default_namespace);
                Ok((table_id, Some(statement)))
            },
            _ => Ok((None, None)),
        }
    }

    fn extract_dml_table_id_only(
        sql: &str,
        kind: &SqlStatementKind,
        default_namespace: &SchemaId,
    ) -> (Option<TableId>, Option<Statement>) {
        let table_id = match kind {
            SqlStatementKind::Insert(_)
            | SqlStatementKind::Update(_)
            | SqlStatementKind::Delete(_) => {
                kalamdb_sql::extract_dml_table_id_fast(sql, default_namespace)
                    .or_else(|| kalamdb_sql::extract_dml_table_id(sql, default_namespace))
            },
            _ => None,
        };
        (table_id, None)
    }

    /// Execute a statement without request metadata.
    pub async fn execute(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
        params: Vec<ScalarValue>,
    ) -> Result<ExecutionResult, KalamDbError> {
        // Step 0: Check SQL query length to prevent DoS attacks
        if sql.len() > kalamdb_commons::constants::MAX_SQL_QUERY_LENGTH {
            log::warn!(
                "SQL query rejected: length {} bytes exceeds maximum {} bytes",
                sql.len(),
                kalamdb_commons::constants::MAX_SQL_QUERY_LENGTH
            );
            return Err(KalamDbError::InvalidSql(format!(
                "SQL query too long: {} bytes (maximum {} bytes)",
                sql.len(),
                kalamdb_commons::constants::MAX_SQL_QUERY_LENGTH
            )));
        }

        let metadata = self
            .prepare_statement_metadata(sql, exec_ctx)
            .map_err(Self::map_classification_error)?;

        self.execute_with_metadata(&metadata, exec_ctx, params).await
    }

    /// Execute a statement with prepared metadata.
    pub async fn execute_with_metadata(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
        params: Vec<ScalarValue>,
    ) -> Result<ExecutionResult, KalamDbError> {
        let sql = metadata.sql.as_str();
        kalamdb_observability::kdb_await_in_info_span!(
            async {
            let query_start = Instant::now();
            let classified = match metadata.classified_statement.as_deref() {
                Some(classified) => classified,
                None => {
                    kalamdb_observability::observe_query(
                        kalamdb_observability::QueryMetricKind::Other,
                        query_start.elapsed(),
                        true);
                    return Err(KalamDbError::InvalidSql(
                        "Missing pre-classified statement metadata for SQL execution".to_string()));
                },
            };

            #[cfg(feature = "traceability")]
            {
                let command_label = format!("{:?}", classified.kind());
                kalamdb_observability::kdb_record_current_span!("command", command_label.as_str());
            }
            let query_kind = query_metric_kind(classified.kind());
            let observe_query_metrics = if matches!(exec_ctx.user_role(), Role::Dba | Role::System)
            {
                if let Some(table_id) = metadata.table_id.as_ref() {
                    kalamdb_observability::should_observe_query_namespace(
                        table_id.namespace_id().as_str())
                } else if !kalamdb_observability::should_observe_query_namespace(
                    exec_ctx.default_namespace().as_str()) {
                    false
                } else if matches!(classified.kind(), SqlStatementKind::Select) {
                    !contains_internal_namespace_hint(metadata.sql.as_str())
                } else {
                    true
                }
            } else {
                true
            };

            // Step 2: Route based on statement type
            let result = if let Err(error) =
                self.ensure_active_transaction_not_aborted(&classified, exec_ctx)
            {
                Err(error)
            } else if let Err(error) =
                self.reject_ddl_in_active_request_transaction(&classified, exec_ctx)
            {
                Err(error)
            } else {
                match classified.kind() {
                    SqlStatementKind::BeginTransaction => {
                        self.execute_begin_transaction(exec_ctx).await
                    },
                    SqlStatementKind::CommitTransaction => {
                        self.execute_commit_transaction(exec_ctx).await
                    },
                    SqlStatementKind::RollbackTransaction => {
                        self.execute_rollback_transaction(exec_ctx)
                    },
                    SqlStatementKind::Savepoint(name) => {
                        self.execute_savepoint(name, exec_ctx)
                    },
                    SqlStatementKind::ReleaseSavepoint(name) => {
                        self.execute_release_savepoint(name, exec_ctx)
                    },
                    SqlStatementKind::RollbackToSavepoint(name) => {
                        self.execute_rollback_to_savepoint(name, exec_ctx)
                    },
                    SqlStatementKind::SetTransaction => Ok(ExecutionResult::Success {
                        message: "SET TRANSACTION".to_string(),
                    }),
                    SqlStatementKind::Call(statement) => {
                        Box::pin(crate::functions::FunctionService::execute_call(
                            Arc::clone(&self.app_context),
                            exec_ctx,
                            statement,
                            &params,
                        ))
                        .await
                    },

                    // Hot path: SELECT queries use DataFusion
                    // Tables are already registered in base session, we just inject user_id
                    SqlStatementKind::Select => {
                        Box::pin(self.execute_via_datafusion(
                            classified.as_str(),
                            params,
                            exec_ctx,
                        ))
                        .await
                    },

                    // DataFusion meta commands (EXPLAIN, SET, SHOW, etc.) - admin only
                    // No caching needed - these are diagnostic/config commands
                    // Authorization already checked in classifier
                    SqlStatementKind::DataFusionMetaCommand => {
                        self.execute_meta_command(sql, exec_ctx).await
                    },
                    // SET/RESET search_path is a session clause (contract/USE
                    // canonicalization) with no DDL handler. GUI clients such as
                    // Tabularis require it to succeed; wire persist applies the schema.
                    SqlStatementKind::SetSearchPath(stmt) => {
                        self.apply_set_search_path(stmt, exec_ctx);
                        Ok(ExecutionResult::Success {
                            message: "SET".to_string(),
                        })
                    },

                    // Native DataFusion DML path (provider insert/update/delete hooks)
                    SqlStatementKind::Insert(_) => {
                        Box::pin(self.execute_table_dml(
                            classified.as_str(),
                            metadata,
                            params,
                            exec_ctx,
                            DmlKind::Insert,
                        ))
                        .await
                    },
                    SqlStatementKind::Update(_) => {
                        Box::pin(self.execute_table_dml(
                            classified.as_str(),
                            metadata,
                            params,
                            exec_ctx,
                            DmlKind::Update,
                        ))
                        .await
                    },
                    SqlStatementKind::Delete(_) => {
                        Box::pin(self.execute_table_dml(
                            classified.as_str(),
                            metadata,
                            params,
                            exec_ctx,
                            DmlKind::Delete,
                        ))
                        .await
                    },

                    // DDL operations that modify table/view structure require plan cache invalidation
                    // This prevents stale cached plans from referencing dropped/altered tables
                    SqlStatementKind::CreateTable(_)
                    | SqlStatementKind::DropTable(_)
                    | SqlStatementKind::AlterTable(_)
                    | SqlStatementKind::CreateView(_)
                    | SqlStatementKind::CreatePolicy(_)
                    | SqlStatementKind::AlterPolicy(_)
                    | SqlStatementKind::DropPolicy(_)
                    | SqlStatementKind::CreateSchema(_)
                    | SqlStatementKind::DropSchema(_)
                    | SqlStatementKind::CreateType(_)
                    | SqlStatementKind::AlterType(_)
                    | SqlStatementKind::DropType(_)
                    | SqlStatementKind::CreateDomain(_)
                    | SqlStatementKind::AlterDomain(_)
                    | SqlStatementKind::DropDomain(_) => {
                        let result = self
                            .handler_registry
                            .handle(classified.clone(), params, exec_ctx)
                            .await;
                        // Clear plan cache after DDL to invalidate any cached plans
                        // that may reference the modified schema
                        if result.is_ok() {
                            self.sql_cache_registry.clear();
                            log::debug!("SQL caches cleared after DDL operation");
                        }
                        result
                    },

                    // All other statements: Delegate to handler registry (no cache invalidation needed)
                    _ => self.handler_registry.handle(classified.clone(), params, exec_ctx).await,
                }
            };
            if let Err(ref error) = result {
                if !matches!(
                    classified.kind(),
                    SqlStatementKind::BeginTransaction
                        | SqlStatementKind::CommitTransaction
                        | SqlStatementKind::RollbackTransaction
                        | SqlStatementKind::Savepoint(_)
                        | SqlStatementKind::ReleaseSavepoint(_)
                        | SqlStatementKind::RollbackToSavepoint(_)
                ) {
                    self.abort_active_transaction_on_statement_failure(
                        &classified,
                        metadata,
                        error,
                        exec_ctx,
                    )
                    .await;
                }
            }
            if observe_query_metrics {
                kalamdb_observability::observe_query(
                    query_kind,
                    query_start.elapsed(),
                    result.is_err());
            }

            #[cfg(feature = "traceability")]
            if let Ok(ref res) = result {
                let rows = match res {
                    ExecutionResult::Rows { row_count, .. }
                    | ExecutionResult::ScalarRows { row_count, .. } => *row_count,
                    ExecutionResult::Inserted { rows_affected } => *rows_affected,
                    ExecutionResult::Updated { rows_affected } => *rows_affected,
                    ExecutionResult::Deleted { rows_affected } => *rows_affected,
                    _ => 0,
                };
                kalamdb_observability::kdb_record_current_span!("rows", rows);
            }

            result
        },
            "sql.execute",
            user_id = %exec_ctx.user_id().as_str(),
            namespace = %exec_ctx.default_namespace().as_str(),
            command = tracing::field::Empty,
            rows = tracing::field::Empty)
    }

    fn should_stage_autocommit_dml(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<bool, KalamDbError> {
        if exec_ctx.transaction_id().is_some() {
            return Ok(false);
        }

        if self.active_request_transaction_id(exec_ctx)?.is_some() {
            return Ok(false);
        }

        let Some(table_id) = metadata.table_id.as_ref() else {
            return Ok(false);
        };

        let Some(cached_table) = self.app_context.schema_registry().get(table_id) else {
            return Ok(false);
        };

        let table_type: TableType = cached_table.table.table_type.into();
        Ok(matches!(table_type, TableType::User | TableType::Shared))
    }

    async fn execute_autocommit_dml_via_transaction(
        &self,
        sql: &str,
        metadata: &PreparedExecutionStatement,
        params: Vec<ScalarValue>,
        exec_ctx: &ExecutionContext,
        dml_kind: DmlKind,
    ) -> Result<ExecutionResult, KalamDbError> {
        let owned_exec_ctx;
        let dml_exec_ctx = if exec_ctx.request_id().is_some() {
            exec_ctx
        } else {
            owned_exec_ctx =
                exec_ctx.clone().with_request_id(format!("sql-autocommit-{}", Uuid::now_v7()));
            &owned_exec_ctx
        };

        self.run_with_request_transaction(
            dml_exec_ctx,
            "sql-autocommit",
            "autocommit DML",
            |ctx, _tx_id| async move {
                self.execute_dml_via_datafusion_inner(sql, metadata, params, ctx, dml_kind).await
            },
        )
        .await
    }

    async fn execute_dml_via_datafusion(
        &self,
        sql: &str,
        metadata: &PreparedExecutionStatement,
        params: Vec<ScalarValue>,
        exec_ctx: &ExecutionContext,
        dml_kind: DmlKind,
    ) -> Result<ExecutionResult, KalamDbError> {
        if self.should_stage_autocommit_dml(metadata, exec_ctx)? {
            return self
                .execute_autocommit_dml_via_transaction(sql, metadata, params, exec_ctx, dml_kind)
                .await;
        }

        self.execute_dml_via_datafusion_inner(sql, metadata, params, exec_ctx, dml_kind)
            .await
    }

    async fn plan_dml_with_provider_reload(
        &self,
        execution_sql: &str,
        pre_parsed: Option<&Statement>,
        original_sql: &str,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<(SessionContext, DataFrame, bool), KalamDbError> {
        let session = self.create_session_with_transaction_context(exec_ctx)?;
        let mut statement = match pre_parsed {
            Some(stmt) => stmt.clone(),
            None => Self::parse_planned_statement(execution_sql)?,
        };
        let had_catalog_func =
            self.evaluate_catalog_functions_in_statement(&mut statement, exec_ctx, params).await?;
        #[cfg(feature = "traceability")]
        let plan_start = std::time::Instant::now();

        match Self::dataframe_from_statement(&session, &statement).await {
            Ok(df) => {
                kalamdb_observability::kdb_debug!(
                    plan_ms = (plan_start.elapsed().as_micros() as f64 / 1000.0),
                    "sql.dml_plan"
                );
                Ok((session, df, had_catalog_func))
            },
            Err(error) if Self::is_table_not_found_error(&error) => {
                if let Err(load_err) = self.load_existing_tables().await {
                    log::warn!(
                        target: "sql::dml",
                        "⚠️  Failed to reload table providers after missing table in DML | sql='{}' | error='{}'",
                        original_sql,
                        load_err
                    );
                }

                let retry_session = self.create_session_with_transaction_context(exec_ctx)?;
                #[cfg(feature = "traceability")]
                let retry_start = std::time::Instant::now();
                let retry_df =
                    Self::dataframe_from_statement(&retry_session, &statement).await.map_err(
                        |retry_error| self.log_sql_error(original_sql, exec_ctx, retry_error),
                    )?;
                kalamdb_observability::kdb_debug!(
                    plan_ms = (retry_start.elapsed().as_micros() as f64 / 1000.0),
                    reloaded_providers = true,
                    "sql.dml_plan"
                );
                Ok((retry_session, retry_df, had_catalog_func))
            },
            Err(error) => Err(self.log_sql_error(original_sql, exec_ctx, error)),
        }
    }

    fn cache_and_bind_dml_plan(
        &self,
        cache_key: &PlanCacheKey,
        planned_df: DataFrame,
        params: &[ScalarValue],
    ) -> Result<LogicalPlan, KalamDbError> {
        let template_plan = planned_df.logical_plan().clone();
        self.sql_cache_registry
            .plan_cache()
            .insert(cache_key.clone(), template_plan.clone());
        replace_placeholders_in_plan(template_plan, params)
    }

    #[cfg_attr(feature = "traceability", tracing::instrument(
        name = "sql.dml_datafusion",
        skip_all,
        fields(
            dml_kind = %Self::dml_operation_name(dml_kind),
            rows_affected = tracing::field::Empty)
    ))]
    async fn execute_dml_via_datafusion_inner(
        &self,
        sql: &str,
        metadata: &PreparedExecutionStatement,
        params: Vec<ScalarValue>,
        exec_ctx: &ExecutionContext,
        dml_kind: DmlKind,
    ) -> Result<ExecutionResult, KalamDbError> {
        if Self::is_system_migrations_table(metadata.table_id.as_ref()) && !params.is_empty() {
            return Err(KalamDbError::InvalidOperation(
                "Parameterized system.migrations DML is not supported".to_string(),
            ));
        }
        if params.is_empty() {
            if let Some(result) = self.try_execute_system_migrations_dml(metadata, dml_kind).await?
            {
                return Ok(result);
            }
        }

        self.block_system_namespace_dml(metadata.table_id.as_ref(), dml_kind)?;

        let execution_sql_cow = kalamdb_sql::rewrite_context_functions_for_datafusion(sql);
        let pre_parsed = match &execution_sql_cow {
            std::borrow::Cow::Borrowed(_) if sql == metadata.sql.as_str() => {
                metadata.parsed_dml.as_deref()
            },
            _ => None,
        };
        let execution_sql: &str = &execution_sql_cow;

        if !params.is_empty() {
            validate_params(&params)?;
        }

        // Parameterized DML: reuse cached template plans and only bind placeholders per request.
        // This avoids reparsing/replanning the same INSERT/UPDATE/DELETE shape repeatedly.
        let df = if params.is_empty() {
            self.plan_dml_with_provider_reload(execution_sql, pre_parsed, sql, exec_ctx, &params)
                .await?
                .1
        } else {
            let cache_key = PlanCacheKey::new(
                exec_ctx.default_namespace(),
                exec_ctx.user_role(),
                execution_sql,
            );
            let session = self.create_session_with_transaction_context(exec_ctx)?;

            if let Some(template_plan) = self.sql_cache_registry.plan_cache().get(&cache_key) {
                let bound_plan = replace_placeholders_in_plan((*template_plan).clone(), &params)?;
                match session.execute_logical_plan(bound_plan).await {
                    Ok(df) => df,
                    Err(error) => {
                        self.sql_cache_registry.plan_cache().bump_dependency_epoch();
                        if let Some(not_leader_err) = Self::try_not_leader_error(&error) {
                            return Err(not_leader_err);
                        }
                        if !Self::is_table_not_found_error(&error) {
                            return Err(Self::datafusion_to_execution_error(error));
                        }

                        log::warn!(
                            target: "sql::dml",
                            "Failed to execute cached DML plan due to missing table, reparsing SQL: {}",
                            error
                        );

                        let (plan_session, planned_df, had_catalog_func) = self
                            .plan_dml_with_provider_reload(
                                execution_sql,
                                pre_parsed,
                                sql,
                                exec_ctx,
                                &params,
                            )
                            .await?;
                        let rebound_plan = if had_catalog_func {
                            replace_placeholders_in_plan(planned_df.logical_plan().clone(), &params)?
                        } else {
                            self.cache_and_bind_dml_plan(&cache_key, planned_df, &params)?
                        };
                        plan_session
                            .execute_logical_plan(rebound_plan)
                            .await
                            .map_err(Self::datafusion_to_execution_error)?
                    },
                }
            } else {
                let (plan_session, planned_df, had_catalog_func) = self
                    .plan_dml_with_provider_reload(
                        execution_sql,
                        pre_parsed,
                        sql,
                        exec_ctx,
                        &params,
                    )
                    .await?;
                let bound_plan = if had_catalog_func {
                    replace_placeholders_in_plan(planned_df.logical_plan().clone(), &params)?
                } else {
                    self.cache_and_bind_dml_plan(&cache_key, planned_df, &params)?
                };
                plan_session
                    .execute_logical_plan(bound_plan)
                    .await
                    .map_err(Self::datafusion_to_execution_error)?
            }
        };

        #[cfg(feature = "traceability")]
        let collect_start = std::time::Instant::now();
        let batches = df.collect().await.map_err(Self::datafusion_to_execution_error)?;
        kalamdb_observability::kdb_debug!(
            collect_ms = collect_start.elapsed().as_secs_f64() * 1000.0,
            "sql.dml_collect"
        );

        let rows_affected = Self::extract_rows_affected(&batches)?;
        kalamdb_observability::kdb_record_current_span!("rows_affected", rows_affected);

        Ok(match dml_kind {
            DmlKind::Insert => ExecutionResult::Inserted { rows_affected },
            DmlKind::Update => ExecutionResult::Updated { rows_affected },
            DmlKind::Delete => ExecutionResult::Deleted { rows_affected },
        })
    }

    /// Execute SELECT via DataFusion with per-user session
    #[cfg_attr(feature = "traceability", tracing::instrument(
        name = "sql.select_datafusion",
        skip_all,
        fields(row_count = tracing::field::Empty)
    ))]
    async fn execute_via_datafusion(
        &self,
        sql: &str,
        params: Vec<ScalarValue>,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let isolated;
        let exec_ctx = if self.is_stream_target_sql(sql, None, exec_ctx).await? {
            isolated = Self::isolated_stream_exec_ctx(exec_ctx);
            &isolated
        } else {
            exec_ctx
        };
        let execution_sql = kalamdb_sql::rewrite_context_functions_for_datafusion(sql);
        let execution_sql: &str = &execution_sql;
        let default_namespace = exec_ctx.default_namespace();

        // Validate parameters if present
        if !params.is_empty() {
            validate_params(&params)?;
        }

        // Try cached template plan first (works for both plain and parameterized SQL).
        // Key excludes user_id because LogicalPlan is user-agnostic - filtering happens at scan
        // time.
        let cache_key =
            PlanCacheKey::new(default_namespace.clone(), exec_ctx.user_role(), execution_sql);

        let df = if let Some(template_plan) = self.sql_cache_registry.plan_cache().get(&cache_key) {
            if let Some(result) = kalamdb_observability::kdb_await_in_info_span!(
                self.try_execute_cached_point_get(template_plan.as_ref(), &params, exec_ctx),
                "sql.point_get"
            )? {
                return Ok(result);
            }

            let executable_plan = if params.is_empty() {
                (*template_plan).clone()
            } else {
                replace_placeholders_in_plan((*template_plan).clone(), &params)?
            };

            let session = self.create_session_with_transaction_context(exec_ctx)?;
            match session.execute_logical_plan(executable_plan).await {
                Ok(df) => df,
                Err(e) => {
                    log::warn!("Failed to execute cached plan, reparsing SQL: {}", e);
                    let mut statement = Self::parse_planned_statement(execution_sql)?;
                    let had_catalog_func = self
                        .evaluate_catalog_functions_in_statement(&mut statement, exec_ctx, &params)
                        .await?;
                    let planned_df = match Self::dataframe_from_statement(&session, &statement)
                        .await
                    {
                        Ok(df) => df,
                        Err(e) => {
                            if Self::is_table_not_found_error(&e) {
                                log::warn!(
                                    target: "sql::plan",
                                    "⚠️  Table not found during planning; reloading table providers and retrying once | sql='{}'",
                                    sql
                                );
                                if let Err(e) = self.load_existing_tables().await {
                                    log::warn!(
                                        target: "sql::plan",
                                        "⚠️  Failed to reload table providers after missing table | sql='{}' | error='{}'",
                                        sql,
                                        e
                                    );
                                }
                                let retry_session =
                                    self.create_session_with_transaction_context(exec_ctx)?;
                                match Self::dataframe_from_statement(&retry_session, &statement)
                                    .await
                                {
                                    Ok(df) => df,
                                    Err(e2) => {
                                        return Err(self.log_sql_error(sql, exec_ctx, e2));
                                    },
                                }
                            } else {
                                return Err(self.log_sql_error(sql, exec_ctx, e));
                            }
                        },
                    };

                    let template_plan = self
                        .optimized_plan_for_cache(&session, &planned_df, &default_namespace)
                        .await?;
                    if !had_catalog_func {
                        self.sql_cache_registry
                            .plan_cache()
                            .insert(cache_key.clone(), template_plan.clone());
                    }

                    let executable_plan = if params.is_empty() {
                        template_plan
                    } else {
                        replace_placeholders_in_plan(template_plan, &params)?
                    };

                    match session.execute_logical_plan(executable_plan).await {
                        Ok(df) => df,
                        Err(e) => {
                            if let Some(not_leader_err) = Self::try_not_leader_error(&e) {
                                return Err(not_leader_err);
                            }
                            log::error!(
                                target: "sql::exec",
                                "❌ SQL execution failed after replan | sql='{}' | params={} | error='{}'",
                                sql,
                                params.len(),
                                e
                            );
                            return Err(Self::datafusion_to_execution_error(e));
                        },
                    }
                },
            }
        } else {
            let session = self.create_session_with_transaction_context(exec_ctx)?;
            let mut statement = Self::parse_planned_statement(execution_sql)?;
            let mut had_catalog_func = false;
            if let Some(direct_result) = self
                .try_execute_standalone_catalog_function_select(
                    &mut statement,
                    exec_ctx,
                    &params,
                    &mut had_catalog_func,
                )
                .await?
            {
                return Ok(direct_result);
            }
            had_catalog_func |= self
                .evaluate_catalog_functions_in_statement(&mut statement, exec_ctx, &params)
                .await?;
            let planned_df = match Self::dataframe_from_statement(&session, &statement).await {
                Ok(df) => df,
                Err(e) => {
                    if Self::is_table_not_found_error(&e) {
                        log::warn!(
                            target: "sql::plan",
                            "⚠️  Table not found during planning; reloading table providers and retrying once | sql='{}'",
                            sql
                        );
                        if let Err(e) = self.load_existing_tables().await {
                            log::warn!(
                                target: "sql::plan",
                                "⚠️  Failed to reload table providers after missing table | sql='{}' | error='{}'",
                                sql,
                                e
                            );
                        }
                        let retry_session =
                            self.create_session_with_transaction_context(exec_ctx)?;
                        match Self::dataframe_from_statement(&retry_session, &statement).await {
                            Ok(df) => df,
                            Err(e2) => {
                                return Err(self.log_sql_error(sql, exec_ctx, e2));
                            },
                        }
                    } else {
                        return Err(self.log_sql_error(sql, exec_ctx, e));
                    }
                },
            };

            let template_plan =
                self.optimized_plan_for_cache(&session, &planned_df, &default_namespace).await?;
            if !had_catalog_func {
                self.sql_cache_registry.plan_cache().insert(cache_key, template_plan.clone());
            }

            let executable_plan = if params.is_empty() {
                template_plan
            } else {
                replace_placeholders_in_plan(template_plan, &params)?
            };

            match session.execute_logical_plan(executable_plan).await {
                Ok(df) => df,
                Err(e) => {
                    if let Some(not_leader_err) = Self::try_not_leader_error(&e) {
                        return Err(not_leader_err);
                    }
                    log::error!(
                        target: "sql::exec",
                        "❌ SQL execution failed | sql='{}' | params={} | error='{}'",
                        sql,
                        params.len(),
                        e
                    );
                    return Err(Self::datafusion_to_execution_error(e));
                },
            }
        };

        let df = self.apply_select_limits(df, sql)?;

        // Capture schema before collecting (needed for 0 row results)
        // DFSchema -> Arrow Schema via inner() method
        let schema: arrow::datatypes::SchemaRef =
            std::sync::Arc::new(df.schema().as_arrow().clone());

        // Execute and collect results (log execution errors)
        let batches = match df.collect().await {
            Ok(batches) => batches,
            Err(e) => {
                // Propagate NOT_LEADER as a typed error so the HTTP layer can forward to leader.
                if let Some(not_leader_err) = Self::try_not_leader_error(&e) {
                    return Err(not_leader_err);
                }
                return Err(self.log_sql_error(sql, exec_ctx, e));
            },
        };

        // Calculate total row count
        let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();
        kalamdb_observability::kdb_record_current_span!("row_count", row_count);

        // Return batches with row count and schema (schema is needed when batches is empty)
        Ok(ExecutionResult::Rows {
            batches,
            row_count,
            schema: Some(schema),
        })
    }

    /// Execute DataFusion meta commands (EXPLAIN, SET, SHOW, etc.)
    ///
    /// PostgreSQL / JDBC `EXPLAIN (option, ...)` is adapted for DataFusion first
    /// (JSON → pgjson, strip `BUFFERS`). DataFusion owns METRICS / LEVEL / FORMAT.
    /// No plan caching is performed since these are diagnostic/config commands.
    /// Authorization is already checked in the classifier (admin only).
    #[cfg_attr(
        feature = "traceability",
        tracing::instrument(name = "sql.meta_command", skip_all)
    )]
    async fn execute_meta_command(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        // PostgreSQL GUI `SET` (search_path, client_encoding, …) is owned by
        // `kalamdb-postgres-wire::client_catalog`. Accept non-DataFusion SET as a
        // no-op here so HTTP admin sessions do not error; wire applies search_path.
        if Self::is_postgres_client_set(sql) {
            return Ok(ExecutionResult::Success {
                message: "SET".to_string(),
            });
        }
        if let Some(result) = Self::postgres_show_result(sql) {
            return Ok(result);
        }

        let mut execution_sql = match Self::rewrite_describe_shorthand(sql, exec_ctx) {
            Some(rewritten) => rewritten,
            None => kalamdb_sql::rewrite_context_functions_for_datafusion(sql).to_string(),
        };
        let postgres_explain = match rewrite_explain_for_datafusion(&execution_sql) {
            Ok(rewritten) => rewritten,
            Err(error) => return Err(KalamDbError::InvalidSql(error)),
        };
        if let Some(rewritten) = postgres_explain.as_ref() {
            execution_sql = rewritten.sql.clone();
        }
        let explain_analyze = postgres_explain
            .as_ref()
            .map(|rewritten| rewritten.analyze)
            .unwrap_or_else(|| Self::is_explain_analyze(sql));
        // Create per-request SessionContext with user_id injected
        let session = self.create_session_with_transaction_context(exec_ctx)?;
        let session = if explain_analyze {
            let mut state = session.state();
            install_extension(&mut state, ScanDiagnosticsContext::enabled());
            SessionContext::new_with_state(state)
        } else {
            session
        };

        let statement = Self::parse_planned_statement(&execution_sql)?;
        let df = match Self::dataframe_from_statement(&session, &statement).await {
            Ok(df) => df,
            Err(e) => {
                log::error!(
                    target: "sql::meta",
                    "❌ Meta command failed | sql='{}' | user='{}' | role='{:?}' | error='{}'",
                    sql,
                    exec_ctx.user_id().as_str(),
                    exec_ctx.user_role(),
                    e
                );
                return Err(Self::datafusion_to_execution_error(e));
            },
        };

        // Capture schema before collecting
        let schema: arrow::datatypes::SchemaRef =
            std::sync::Arc::new(df.schema().as_arrow().clone());

        // Execute and collect results
        let batches = match df.collect().await {
            Ok(batches) => batches,
            Err(e) => {
                if let Some(not_leader_err) = Self::try_not_leader_error(&e) {
                    return Err(not_leader_err);
                }
                return Err(self.log_sql_error(sql, exec_ctx, e));
            },
        };

        let (batches, schema) = if let Some(rewritten) = postgres_explain.as_ref() {
            Self::postgres_explain_query_plan(rewritten.format, batches)?
        } else {
            (batches, schema)
        };
        let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();

        Ok(ExecutionResult::Rows {
            batches,
            row_count,
            schema: Some(schema),
        })
    }

    /// Log SQL errors with appropriate level (warn for user errors, error for system errors)
    fn log_sql_error(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
        e: datafusion::error::DataFusionError,
    ) -> KalamDbError {
        let mapped_error = Self::classify_datafusion_error(&e);

        match &mapped_error {
            KalamDbError::TableNotFound(_) => {
                log::warn!(
                    target: "sql::plan",
                    "⚠️  Table not found | sql='{}' | user='{}' | role='{:?}' | error='{}'",
                    sql,
                    exec_ctx.user_id().as_str(),
                    exec_ctx.user_role(),
                    e
                );
            },
            KalamDbError::PermissionDenied(_) => {
                log::warn!(
                    target: "sql::plan",
                    "⚠️  SQL permission denied | sql='{}' | user='{}' | role='{:?}' | error='{}'",
                    sql,
                    exec_ctx.user_id().as_str(),
                    exec_ctx.user_role(),
                    e
                );
            },
            KalamDbError::InvalidOperation(_) => {
                log::warn!(
                    target: "sql::plan",
                    "⚠️  SQL column validation failed | sql='{}' | user='{}' | role='{:?}' | error='{}'",
                    sql,
                    exec_ctx.user_id().as_str(),
                    exec_ctx.user_role(),
                    e
                );
            },
            KalamDbError::AlreadyExists(_) => {
                log::warn!(
                    target: "sql::plan",
                    "⚠️  SQL constraint validation failed | sql='{}' | user='{}' | role='{:?}' | error='{}'",
                    sql,
                    exec_ctx.user_id().as_str(),
                    exec_ctx.user_role(),
                    e
                );
            },
            _ => {
                log::error!(
                    target: "sql::plan",
                    "❌ SQL planning failed | sql='{}' | user='{}' | role='{:?}' | error='{}'",
                    sql,
                    exec_ctx.user_id().as_str(),
                    exec_ctx.user_role(),
                    e
                );
            },
        }

        mapped_error
    }

    fn extract_rows_affected(batches: &[RecordBatch]) -> Result<usize, KalamDbError> {
        let mut total: usize = 0;

        for batch in batches {
            if batch.num_columns() == 0 || batch.num_rows() == 0 {
                continue;
            }

            let count_column = batch.column_by_name("count").unwrap_or_else(|| batch.column(0));

            if let Some(u64_arr) =
                count_column.as_any().downcast_ref::<arrow::array::UInt64Array>()
            {
                for row_idx in 0..batch.num_rows() {
                    if !u64_arr.is_null(row_idx) {
                        let v = u64_arr.value(row_idx);
                        total += usize::try_from(v).map_err(|_| {
                            KalamDbError::ExecutionError(format!(
                                "DML count {} exceeds platform usize range",
                                v
                            ))
                        })?;
                    }
                }
                continue;
            }

            for row_idx in 0..batch.num_rows() {
                let count_value =
                    arrow_value_to_scalar(count_column.as_ref(), row_idx).map_err(|e| {
                        KalamDbError::ExecutionError(format!(
                            "Failed to decode DML count value from result batch: {}",
                            e
                        ))
                    })?;
                total += Self::scalar_count_to_usize(count_value)?;
            }
        }

        Ok(total)
    }

    fn scalar_count_to_usize(value: ScalarValue) -> Result<usize, KalamDbError> {
        let invalid = |v: ScalarValue| {
            KalamDbError::ExecutionError(format!(
                "DML result does not contain a valid count value: {:?}",
                v
            ))
        };

        match value {
            ScalarValue::UInt64(Some(v)) => usize::try_from(v).map_err(|_| {
                KalamDbError::ExecutionError(format!(
                    "DML count {} exceeds platform usize range",
                    v
                ))
            }),
            ScalarValue::UInt32(Some(v)) => Ok(v as usize),
            ScalarValue::UInt16(Some(v)) => Ok(v as usize),
            ScalarValue::UInt8(Some(v)) => Ok(v as usize),
            ScalarValue::Int64(Some(v)) if v >= 0 => usize::try_from(v as u64).map_err(|_| {
                KalamDbError::ExecutionError(format!(
                    "DML count {} exceeds platform usize range",
                    v
                ))
            }),
            ScalarValue::Int32(Some(v)) if v >= 0 => Ok(v as usize),
            ScalarValue::Int16(Some(v)) if v >= 0 => Ok(v as usize),
            ScalarValue::Int8(Some(v)) if v >= 0 => Ok(v as usize),
            ScalarValue::Utf8(Some(v)) | ScalarValue::LargeUtf8(Some(v)) => {
                let parsed = v.parse::<u64>().map_err(|e| {
                    KalamDbError::ExecutionError(format!(
                        "Failed to parse DML count '{}' as number: {}",
                        v, e
                    ))
                })?;
                usize::try_from(parsed).map_err(|_| {
                    KalamDbError::ExecutionError(format!(
                        "DML count {} exceeds platform usize range",
                        parsed
                    ))
                })
            },
            other => Err(invalid(other)),
        }
    }

    /// Load existing tables from system.tables and register providers
    ///
    /// Called during server startup to restore table access after restart.
    /// Loads table definitions from the store and creates/registers:
    /// - UserTableShared instances for USER tables
    /// - SharedTableProvider instances for SHARED tables
    /// - StreamTableProvider instances for STREAM tables
    ///
    /// # Returns
    /// Ok on success, error if table loading fails
    pub async fn load_existing_tables(&self) -> Result<(), KalamDbError> {
        let app_context = &self.app_context;
        // Delegate to unified SchemaRegistry initialization
        app_context.schema_registry().initialize_tables()
    }
}

#[cfg(test)]
mod tests {
    use datafusion::{
        common::DFSchema,
        logical_expr::{lit, EmptyRelation, Limit, Sort, SortExpr},
    };
    use kalamdb_commons::{
        datatypes::KalamDataType,
        schemas::{ColumnDefinition, TableDefinition, TableOptions},
        BoundExprShape, PolicyCommand, PolicyId, PolicyProgram, PolicyTarget, PrincipalExpr,
        TableName, TablePolicy,
    };
    use kalamdb_tables::utils::KalamTableProvider;

    use super::*;

    fn result_row_count(result: ExecutionResult) -> usize {
        match result {
            ExecutionResult::Rows { row_count, .. }
            | ExecutionResult::ScalarRows { row_count, .. } => row_count,
            other => panic!("expected rows, got {other:?}"),
        }
    }

    fn empty_plan() -> LogicalPlan {
        LogicalPlan::EmptyRelation(EmptyRelation {
            produce_one_row: false,
            schema:          Arc::new(DFSchema::empty()),
        })
    }

    #[test]
    fn internal_namespace_hint_matches_qualified_system_and_dba_queries() {
        assert!(contains_internal_namespace_hint("SELECT * FROM system.stats LIMIT 100"));
        assert!(contains_internal_namespace_hint("select * from DBA.notifications"));
        assert!(!contains_internal_namespace_hint("SELECT * FROM default.events LIMIT 100"));
    }

    #[test]
    fn logical_plan_has_limit_detects_sort_fetch_and_limit_nodes() {
        let unlimited = empty_plan();
        assert!(!SqlExecutor::logical_plan_has_limit(&unlimited));

        let with_limit = LogicalPlan::Limit(Limit {
            skip:  None,
            fetch: Some(Box::new(lit(5000_i64))),
            input: Arc::new(empty_plan()),
        });
        assert!(SqlExecutor::logical_plan_has_limit(&with_limit));

        let with_sort_fetch = LogicalPlan::Sort(Sort {
            expr:  vec![SortExpr::new(lit(1_i64), true, false)],
            input: Arc::new(empty_plan()),
            fetch: Some(5000),
        });
        assert!(SqlExecutor::logical_plan_has_limit(&with_sort_fetch));

        let sort_without_fetch = LogicalPlan::Sort(Sort {
            expr:  vec![SortExpr::new(lit(1_i64), true, false)],
            input: Arc::new(empty_plan()),
            fetch: None,
        });
        assert!(!SqlExecutor::logical_plan_has_limit(&sort_without_fetch));
    }

    #[test]
    fn unwrap_default_order_wrappers_peels_sort_and_limit_only() {
        let sorted = LogicalPlan::Sort(Sort {
            expr:  vec![SortExpr::new(lit(1_i64), true, false)],
            input: Arc::new(empty_plan()),
            fetch: None,
        });
        assert!(matches!(
            SqlExecutor::unwrap_default_order_wrappers(&sorted),
            LogicalPlan::EmptyRelation(_)
        ));

        let limited = LogicalPlan::Limit(Limit {
            skip:  None,
            fetch: Some(Box::new(lit(1_i64))),
            input: Arc::new(sorted),
        });
        assert!(matches!(
            SqlExecutor::unwrap_default_order_wrappers(&limited),
            LogicalPlan::EmptyRelation(_)
        ));
    }

    #[test]
    fn project_point_get_batches_selects_non_leading_columns_by_name() {
        use arrow::{array::StringArray, datatypes::DataType};

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("file_ref", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![
                Arc::new(StringArray::from(vec!["notes.md"])),
                Arc::new(StringArray::from(vec!["sha-abc"])),
            ],
        )
        .unwrap();
        let target_schema =
            Arc::new(Schema::new(vec![Field::new("file_ref", DataType::Utf8, true)]));

        let projected = SqlExecutor::project_point_get_batches(
            vec![batch],
            source_schema,
            Arc::clone(&target_schema),
        )
        .expect("projection should keep file_ref");

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].schema().fields().len(), 1);
        assert_eq!(projected[0].schema().field(0).name(), "file_ref");
        let values = projected[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(values.value(0), "sha-abc");
    }

    #[test]
    fn project_point_get_batches_reorders_columns_by_name() {
        use arrow::{array::StringArray, datatypes::DataType};

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("file_ref", DataType::Utf8, true),
            Field::new("body", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![
                Arc::new(StringArray::from(vec!["notes.md"])),
                Arc::new(StringArray::from(vec!["sha-abc"])),
                Arc::new(StringArray::from(vec!["hello"])),
            ],
        )
        .unwrap();
        let target_schema = Arc::new(Schema::new(vec![
            Field::new("file_ref", DataType::Utf8, true),
            Field::new("path", DataType::Utf8, false),
        ]));

        let projected = SqlExecutor::project_point_get_batches(
            vec![batch],
            source_schema,
            Arc::clone(&target_schema),
        )
        .expect("reordered projection");

        assert_eq!(projected[0].schema().field(0).name(), "file_ref");
        assert_eq!(projected[0].schema().field(1).name(), "path");
        let file_ref = projected[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
        let path = projected[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(file_ref.value(0), "sha-abc");
        assert_eq!(path.value(0), "notes.md");
    }

    #[test]
    fn project_point_get_batches_returns_none_for_unknown_column() {
        use arrow::{array::StringArray, datatypes::DataType};

        let source_schema = Arc::new(Schema::new(vec![Field::new("path", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(
            Arc::clone(&source_schema),
            vec![Arc::new(StringArray::from(vec!["notes.md"]))],
        )
        .unwrap();
        let target_schema =
            Arc::new(Schema::new(vec![Field::new("file_ref", DataType::Utf8, true)]));

        assert!(
            SqlExecutor::project_point_get_batches(vec![batch], source_schema, target_schema)
                .is_none()
        );
    }

    #[test]
    fn project_point_get_batches_preserves_empty_target_schema() {
        use arrow::datatypes::DataType;

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("path", DataType::Utf8, false),
            Field::new("file_ref", DataType::Utf8, true),
        ]));
        let target_schema =
            Arc::new(Schema::new(vec![Field::new("file_ref", DataType::Utf8, true)]));

        let projected = SqlExecutor::project_point_get_batches(
            Vec::new(),
            source_schema,
            Arc::clone(&target_schema),
        )
        .expect("empty projection");

        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].num_rows(), 0);
        assert_eq!(projected[0].schema().field(0).name(), "file_ref");
    }

    #[test]
    fn project_point_get_scalar_rows_remaps_alias_when_scan_keeps_pk() {
        use arrow::datatypes::DataType;
        use datafusion::logical_expr::col;

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let target_schema = Arc::new(Schema::new(vec![Field::new("title", DataType::Utf8, true)]));
        let rows = vec![Row::from_vec(vec![
            ("id".to_string(), ScalarValue::Int64(Some(1))),
            ("name".to_string(), ScalarValue::Utf8(Some("alpha".to_string()))),
        ])];
        let exprs = vec![col("name").alias("title")];

        let projected = SqlExecutor::project_point_get_scalar_rows(
            rows,
            &source_schema,
            target_schema,
            Some(exprs.as_slice()),
        )
        .expect("aliased projection");

        assert_eq!(
            projected[0].values.get("title"),
            Some(&ScalarValue::Utf8(Some("alpha".to_string())))
        );
        assert!(!projected[0].values.contains_key("name"));
        assert!(!projected[0].values.contains_key("id"));
    }

    #[test]
    fn project_point_get_scalar_rows_returns_none_for_unknown_alias_without_exprs() {
        use arrow::datatypes::DataType;

        let source_schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ]));
        let target_schema = Arc::new(Schema::new(vec![Field::new("title", DataType::Utf8, true)]));
        let rows = vec![Row::from_vec(vec![
            ("id".to_string(), ScalarValue::Int64(Some(1))),
            ("name".to_string(), ScalarValue::Utf8(Some("alpha".to_string()))),
        ])];

        assert!(
            SqlExecutor::project_point_get_scalar_rows(rows, &source_schema, target_schema, None)
                .is_none(),
            "unmapped aliases must fall back to Arrow instead of JSON null"
        );
    }

    #[tokio::test]
    async fn cached_point_plan_binds_rls_principal_per_execution() {
        let app_context = crate::test_helpers::test_app_context_simple();
        let mut table = TableDefinition::new(
            SchemaId::new("security"),
            TableName::new("cached_documents"),
            TableType::Shared,
            vec![
                ColumnDefinition::primary_key(1, "id", 1, KalamDataType::Text),
                ColumnDefinition::simple(2, "owner_id", 2, KalamDataType::Text),
            ],
            TableOptions::shared(),
            None,
        )
        .unwrap();
        app_context.system_columns_service().add_system_columns(&mut table).unwrap();
        app_context.schema_registry().register_table(table).unwrap();
        let table_id = TableId::from_strings("security", "cached_documents");
        let provider = app_context.schema_registry().get_provider(&table_id).unwrap();
        let provider = (provider.as_ref() as &dyn std::any::Any)
            .downcast_ref::<SharedTableProvider>()
            .unwrap();
        provider
            .insert_rows(
                &UserId::new("system"),
                vec![
                    Row::from_vec(vec![
                        ("id".to_string(), ScalarValue::Utf8(Some("doc-a".to_string()))),
                        ("owner_id".to_string(), ScalarValue::Utf8(Some("alice".to_string()))),
                    ]),
                    Row::from_vec(vec![
                        ("id".to_string(), ScalarValue::Utf8(Some("doc-b".to_string()))),
                        ("owner_id".to_string(), ScalarValue::Utf8(Some("bob".to_string()))),
                    ]),
                ],
                &[
                    kalamdb_commons::ids::VersionId::from(1_i64),
                    kalamdb_commons::ids::VersionId::from(2_i64),
                ],
            )
            .await
            .unwrap();

        app_context
            .system_tables()
            .table_policies()
            .create_policy(TablePolicy::new(
                PolicyId::new(table_id.clone(), "owner_read").unwrap(),
                table_id,
                "owner_read",
                PolicyCommand::Select,
                vec![PolicyTarget::Role(Role::User)],
                Some("owner_id = CURRENT_USER".to_string()),
                None,
                Some(PolicyProgram::RowLocal {
                    expr: BoundExprShape::ColumnEqualsPrincipal {
                        column_id: 2,
                        principal: PrincipalExpr::CurrentUser,
                    },
                }),
                None,
                0,
                1,
            ))
            .await
            .unwrap();

        let executor = SqlExecutor::new(app_context.clone(), Arc::new(HandlerRegistry::new()));
        let alice = ExecutionContext::new(
            UserId::new("alice"),
            Role::User,
            app_context.base_session_context(),
        );
        let bob = ExecutionContext::new(
            UserId::new("bob"),
            Role::User,
            app_context.base_session_context(),
        );
        let sql = "SELECT id FROM security.cached_documents WHERE id = $1";

        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &alice, vec![ScalarValue::Utf8(Some("doc-a".to_string()))])
                    .await
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &bob, vec![ScalarValue::Utf8(Some("doc-a".to_string()))])
                    .await
                    .unwrap()
            ),
            0
        );
        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &bob, vec![ScalarValue::Utf8(Some("doc-b".to_string()))])
                    .await
                    .unwrap()
            ),
            1
        );
        assert_eq!(
            result_row_count(
                executor
                    .execute(sql, &alice, vec![ScalarValue::Utf8(Some("doc-b".to_string()))])
                    .await
                    .unwrap()
            ),
            0
        );
        assert_eq!(executor.plan_cache_len(), 1);
        assert!(executor.point_get_fast_path_hits() >= 3);
    }

    #[tokio::test]
    async fn postgres_set_search_path_succeeds_without_a_ddl_handler() {
        let app_context = crate::test_helpers::test_app_context_simple();
        let executor = SqlExecutor::new(app_context.clone(), Arc::new(HandlerRegistry::new()));
        let ctx = ExecutionContext::new(
            UserId::from("tabularis"),
            Role::User,
            app_context.base_session_context(),
        );

        let result = executor
            .execute("SET search_path TO public", &ctx, Vec::new())
            .await
            .expect("Tabularis SET search_path must not require a registered DDL handler");
        match result {
            ExecutionResult::Success { message } => assert_eq!(message, "SET"),
            other => panic!("expected SET success, got {other:?}"),
        }

        let reset = executor
            .execute("RESET search_path", &ctx, Vec::new())
            .await
            .expect("RESET search_path should succeed");
        assert!(matches!(reset, ExecutionResult::Success { .. }));
    }
}
