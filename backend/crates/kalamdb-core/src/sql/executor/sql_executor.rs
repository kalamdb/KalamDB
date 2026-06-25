use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use arrow::{
    array::RecordBatch,
    datatypes::{Field, Schema, SchemaRef},
};
use datafusion::{
    dataframe::DataFrame, datasource::MemTable, logical_expr::LogicalPlan, prelude::SessionContext,
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::arrow_json_conversion::{arrow_value_to_scalar, json_rows_to_arrow_batch},
    models::{rows::row::Row, MigrationId, NamespaceId, TableId, TransactionId, UserId},
    schemas::TableType,
    Role, SystemTable,
};
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind, StatementClassificationError};
use kalamdb_system::Migration;
use kalamdb_tables::{SharedTableProvider, UserTableProvider};
use kalamdb_transactions::{TransactionQueryContext, TransactionQueryExtension};
use sqlparser::ast::{
    Assignment, AssignmentTarget, BinaryOperator, Expr, ObjectName, ObjectNamePart, Statement,
    TableFactor, TableObject, Value,
};
use uuid::Uuid;

use super::{PreparedExecutionStatement, SqlExecutor};
use crate::{
    error::KalamDbError,
    sql::{
        executor::{
            handler_registry::HandlerRegistry,
            parameter_binding::{replace_placeholders_in_plan, validate_params},
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
enum DmlKind {
    Insert,
    Update,
    Delete,
}

struct OnConflictStageResult {
    rows_affected: usize,
    returned_rows: Vec<Row>,
}

struct OnConflictExistingRow {
    row: Row,
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

fn quote_sql_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
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

        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_transaction_state =
            RequestTransactionState::from_request_id(exec_ctx.request_id());
        if let Some(state) = request_transaction_state.as_mut() {
            state.sync(&request_transaction_coordinator);
            if state.is_active() && !contains_ignore_ascii_case(sql, "ON CONFLICT") {
                return Ok(None);
            }
        }

        let dialect = sqlparser::dialect::GenericDialect {};
        let parsed_statements = kalamdb_sql::parser::utils::parse_sql_statements(sql, &dialect)
            .map_err(|error| KalamDbError::InvalidSql(error.to_string()))?;
        if parsed_statements.len() != 1 {
            return Ok(None);
        }

        // ON CONFLICT must be handled before the active-transaction bail-out below.
        if let Statement::Insert(insert) = &parsed_statements[0] {
            if kalamdb_sql::insert_has_on_conflict(insert) {
                if let Some(on_conflict_rows) =
                    super::transaction_batch_insert::try_build_literal_on_conflict_update_rows(
                        &parsed_statements[0],
                        self.app_context.as_ref(),
                        self.sql_cache_registry.as_ref(),
                        exec_ctx,
                        table_id,
                        params,
                    )?
                {
                    return self
                        .execute_literal_on_conflict_update(table_id, exec_ctx, on_conflict_rows)
                        .await
                        .map(Some);
                }
            }
        }

        if let Some(state) = request_transaction_state.as_mut() {
            if state.is_active() {
                return Ok(None);
            }
        }

        let Some(insert_rows) = super::transaction_batch_insert::try_build_literal_insert_rows(
            &parsed_statements[0],
            self.app_context.as_ref(),
            self.sql_cache_registry.as_ref(),
            exec_ctx,
            table_id,
            params,
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

    async fn execute_literal_on_conflict_update(
        &self,
        table_id: &TableId,
        exec_ctx: &ExecutionContext,
        on_conflict_rows: super::transaction_batch_insert::LiteralOnConflictUpdateRows,
    ) -> Result<ExecutionResult, KalamDbError> {
        super::transaction_batch_insert::reject_system_table_dml(on_conflict_rows.table_type)?;

        let owned_exec_ctx;
        let dml_exec_ctx = if exec_ctx.request_id().is_some() {
            exec_ctx
        } else {
            owned_exec_ctx =
                exec_ctx.clone().with_request_id(format!("sql-on-conflict-{}", Uuid::now_v7()));
            &owned_exec_ctx
        };

        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(dml_exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "ON CONFLICT DO UPDATE requires a request-scoped execution context".to_string(),
                )
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
            KalamDbError::InvalidOperation(
                "ON CONFLICT DO UPDATE could not start a transaction".to_string(),
            )
        })?;

        let result = self
            .stage_literal_on_conflict_update(
                table_id,
                dml_exec_ctx,
                &transaction_id,
                &on_conflict_rows,
            )
            .await;

        match result {
            Ok(stage_result) => {
                if started_transaction {
                    if let Err(error) = request_state.commit(&request_transaction_coordinator).await
                    {
                        let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                        return Err(map_request_transaction_error(error));
                    }
                }

                if let Some(returning) = on_conflict_rows.returning.as_ref() {
                    self.build_on_conflict_returning_result(
                        table_id,
                        returning,
                        stage_result.returned_rows,
                    )
                    .await
                } else {
                    Ok(ExecutionResult::Inserted {
                        rows_affected: stage_result.rows_affected,
                    })
                }
            },
            Err(error) => {
                if started_transaction {
                    let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                }
                Err(error)
            },
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
        let mut row_state_by_pk: HashMap<String, OnConflictExistingRow> =
            HashMap::with_capacity(on_conflict_rows.rows.len());
        for row in &on_conflict_rows.rows {
            let primary_key =
                row.values.get(&on_conflict_rows.primary_key_column).ok_or_else(|| {
                    KalamDbError::InvalidOperation(format!(
                        "ON CONFLICT requires primary key column '{}'",
                        on_conflict_rows.primary_key_column
                    ))
                })?;
            let primary_key_string = primary_key.to_string();
            if !row_state_by_pk.contains_key(&primary_key_string) {
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
                    row_state_by_pk.insert(primary_key_string.clone(), existing_row);
                }
            }

            let existing_row = row_state_by_pk.get(&primary_key_string);
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
                primary_key_string,
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
                        row: entry.payload.clone(),
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
                        row: row.fields,
                        mutation_user_id: Some(user_id),
                    }
                }))
            },
            (TableType::Shared, OnConflictTableProvider::Shared(provider)) => {
                Ok(provider.find_by_pk(primary_key).await?.map(|(_, row)| OnConflictExistingRow {
                    row: row.fields,
                    mutation_user_id: None,
                }))
            },
            _ => Err(KalamDbError::InvalidOperation(format!(
                "Provider type mismatch for table {}",
                table_id
            ))),
        }
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

        let session = SessionContext::new();
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
        let df = session.sql(&returning_sql).await.map_err(Self::datafusion_to_execution_error)?;
        let schema = Arc::new(df.schema().as_arrow().clone());
        let batches = df.collect().await.map_err(Self::datafusion_to_execution_error)?;
        let row_count = batches.iter().map(|batch| batch.num_rows()).sum();

        Ok(ExecutionResult::Rows {
            batches,
            row_count,
            schema: Some(schema),
        })
    }

    fn logical_plan_has_limit(plan: &datafusion::logical_expr::LogicalPlan) -> bool {
        matches!(plan, datafusion::logical_expr::LogicalPlan::Limit(_))
            || plan.inputs().iter().any(|input| Self::logical_plan_has_limit(input))
    }

    fn apply_select_limits(
        &self,
        df: datafusion::dataframe::DataFrame,
    ) -> Result<datafusion::dataframe::DataFrame, KalamDbError> {
        let max_query_limit = self.app_context.config().limits.max_query_limit;
        let default_query_limit = self.app_context.config().limits.default_query_limit;
        let has_explicit_limit = Self::logical_plan_has_limit(df.logical_plan());

        if !has_explicit_limit && default_query_limit > 0 {
            let effective_default_limit = if max_query_limit > 0 {
                default_query_limit.min(max_query_limit)
            } else {
                default_query_limit
            };

            log::debug!(
                target: "sql::exec",
                "Applying default query limit {} to unbounded SELECT",
                effective_default_limit
            );

            return df
                .limit(0, Some(effective_default_limit))
                .map_err(Self::datafusion_to_execution_error);
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
                table_id.table_name().as_str(),
            )));
        }
        Ok(())
    }

    fn is_system_migrations_table(table_id: Option<&TableId>) -> bool {
        table_id
            .filter(|table_id| {
                table_id.namespace_id().is_system_namespace()
                    && SystemTable::from_name(table_id.table_name().as_str())
                        .is_ok_and(|table| table == SystemTable::Migrations)
            })
            .is_some()
    }

    fn parse_single_statement(sql: &str) -> Result<Statement, KalamDbError> {
        let dialect = sqlparser::dialect::GenericDialect {};
        let mut statements = kalamdb_sql::parser::utils::parse_sql_statements(sql, &dialect)
            .map_err(|error| KalamDbError::InvalidSql(error.to_string()))?;
        if statements.len() != 1 {
            return Err(KalamDbError::InvalidSql(
                "Expected exactly one system.migrations statement".to_string(),
            ));
        }
        Ok(statements.remove(0))
    }

    fn object_name_parts(name: &ObjectName) -> Option<Vec<&str>> {
        name.0
            .iter()
            .map(|part| match part {
                ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
                ObjectNamePart::Function(_) => None,
            })
            .collect()
    }

    fn expect_system_migrations_object(name: &ObjectName) -> Result<(), KalamDbError> {
        let parts = Self::object_name_parts(name).ok_or_else(|| {
            KalamDbError::InvalidSql("Invalid system.migrations table name".to_string())
        })?;
        if parts.len() == 2
            && parts[0].eq_ignore_ascii_case("system")
            && parts[1].eq_ignore_ascii_case("migrations")
        {
            return Ok(());
        }
        Err(KalamDbError::InvalidSql(format!(
            "Expected system.migrations table, got {}",
            name
        )))
    }

    fn assignment_column_name(assignment: &Assignment) -> Result<String, KalamDbError> {
        match &assignment.target {
            AssignmentTarget::ColumnName(name) => {
                let parts = Self::object_name_parts(name).ok_or_else(|| {
                    KalamDbError::InvalidSql("Invalid assignment target".to_string())
                })?;
                if parts.len() == 1 {
                    Ok(parts[0].to_ascii_lowercase())
                } else {
                    Err(KalamDbError::InvalidSql(format!(
                        "Expected unqualified system.migrations column, got {}",
                        name
                    )))
                }
            },
            AssignmentTarget::Tuple(_) => Err(KalamDbError::InvalidSql(
                "Tuple assignments are not supported for system.migrations".to_string(),
            )),
        }
    }

    fn expr_string(expr: &Expr, column: &str) -> Result<String, KalamDbError> {
        match expr {
            Expr::Value(value) => match &value.value {
                Value::SingleQuotedString(value)
                | Value::DoubleQuotedString(value)
                | Value::TripleSingleQuotedString(value)
                | Value::TripleDoubleQuotedString(value)
                | Value::EscapedStringLiteral(value)
                | Value::UnicodeStringLiteral(value)
                | Value::NationalStringLiteral(value)
                | Value::HexStringLiteral(value) => Ok(value.clone()),
                Value::Number(value, _) => Ok(value.clone()),
                Value::Null => Err(KalamDbError::InvalidSql(format!(
                    "system.migrations column '{}' cannot be NULL",
                    column
                ))),
                other => Err(KalamDbError::InvalidSql(format!(
                    "Unsupported literal for system.migrations column '{}': {}",
                    column, other
                ))),
            },
            other => Err(KalamDbError::InvalidSql(format!(
                "Expected literal for system.migrations column '{}', got {}",
                column, other
            ))),
        }
    }

    fn expr_optional_string(expr: &Expr, column: &str) -> Result<Option<String>, KalamDbError> {
        match expr {
            Expr::Value(value) if matches!(value.value, Value::Null) => Ok(None),
            _ => Self::expr_string(expr, column).map(Some),
        }
    }

    fn expr_optional_i64(expr: &Expr, column: &str) -> Result<Option<i64>, KalamDbError> {
        match expr {
            Expr::Value(value) => match &value.value {
                Value::Null => Ok(None),
                Value::Number(value, _) => value.parse::<i64>().map(Some).map_err(|error| {
                    KalamDbError::InvalidSql(format!(
                        "Invalid integer for system.migrations column '{}': {}",
                        column, error
                    ))
                }),
                other => Err(KalamDbError::InvalidSql(format!(
                    "Unsupported timestamp literal for system.migrations column '{}': {}",
                    column, other
                ))),
            },
            other => Err(KalamDbError::InvalidSql(format!(
                "Expected timestamp literal for system.migrations column '{}', got {}",
                column, other
            ))),
        }
    }

    fn migration_from_insert_values(
        values: &HashMap<String, &Expr>,
    ) -> Result<Migration, KalamDbError> {
        let required = |column: &str| -> Result<&Expr, KalamDbError> {
            values.get(column).copied().ok_or_else(|| {
                KalamDbError::InvalidSql(format!(
                    "Missing required system.migrations column '{}'",
                    column
                ))
            })
        };
        let optional = |column: &str| -> Option<&Expr> { values.get(column).copied() };

        Ok(Migration {
            migration_key: MigrationId::new(Self::expr_string(
                required("migration_key")?,
                "migration_key",
            )?),
            migration_id: Self::expr_string(required("migration_id")?, "migration_id")?,
            namespace: Self::expr_string(required("namespace")?, "namespace")?,
            name: Self::expr_string(required("name")?, "name")?,
            checksum: Self::expr_string(required("checksum")?, "checksum")?,
            status: Self::expr_string(required("status")?, "status")?,
            started_at: optional("started_at")
                .map(|expr| Self::expr_optional_i64(expr, "started_at"))
                .transpose()?
                .flatten(),
            finished_at: optional("finished_at")
                .map(|expr| Self::expr_optional_i64(expr, "finished_at"))
                .transpose()?
                .flatten(),
            error_message: optional("error_message")
                .map(|expr| Self::expr_optional_string(expr, "error_message"))
                .transpose()?
                .flatten(),
            source: optional("source")
                .map(|expr| Self::expr_optional_string(expr, "source"))
                .transpose()?
                .flatten(),
            kalam_version: optional("kalam_version")
                .map(|expr| Self::expr_optional_string(expr, "kalam_version"))
                .transpose()?
                .flatten(),
        })
    }

    fn apply_migration_assignment(
        migration: &mut Migration,
        assignment: &Assignment,
    ) -> Result<(), KalamDbError> {
        let column = Self::assignment_column_name(assignment)?;
        match column.as_str() {
            "migration_key" => {
                return Err(KalamDbError::InvalidOperation(
                    "system.migrations migration_key cannot be updated".to_string(),
                ));
            },
            "migration_id" => {
                return Err(KalamDbError::InvalidOperation(
                    "system.migrations migration_id cannot be updated".to_string(),
                ));
            },
            "namespace" => migration.namespace = Self::expr_string(&assignment.value, &column)?,
            "name" => migration.name = Self::expr_string(&assignment.value, &column)?,
            "checksum" => migration.checksum = Self::expr_string(&assignment.value, &column)?,
            "status" => migration.status = Self::expr_string(&assignment.value, &column)?,
            "started_at" => {
                migration.started_at = Self::expr_optional_i64(&assignment.value, &column)?
            },
            "finished_at" => {
                migration.finished_at = Self::expr_optional_i64(&assignment.value, &column)?
            },
            "error_message" => {
                migration.error_message = Self::expr_optional_string(&assignment.value, &column)?
            },
            "source" => migration.source = Self::expr_optional_string(&assignment.value, &column)?,
            "kalam_version" => {
                migration.kalam_version = Self::expr_optional_string(&assignment.value, &column)?
            },
            _ => {
                return Err(KalamDbError::InvalidSql(format!(
                    "Unknown system.migrations column '{}'",
                    column
                )));
            },
        }
        Ok(())
    }

    fn migration_key_from_update_selection(
        selection: Option<&Expr>,
    ) -> Result<MigrationId, KalamDbError> {
        let Some(selection) = selection else {
            return Err(KalamDbError::InvalidSql(
                "UPDATE system.migrations requires WHERE migration_key = <literal>".to_string(),
            ));
        };
        match selection {
            Expr::BinaryOp { left, op, right } if *op == BinaryOperator::Eq => {
                let column_is_migration_key = matches!(
                    left.as_ref(),
                    Expr::Identifier(ident) if ident.value.eq_ignore_ascii_case("migration_key")
                );
                if column_is_migration_key {
                    return Ok(MigrationId::new(Self::expr_string(right, "migration_key")?));
                }
                Err(KalamDbError::InvalidSql(
                    "UPDATE system.migrations WHERE clause must target migration_key".to_string(),
                ))
            },
            _ => Err(KalamDbError::InvalidSql(
                "UPDATE system.migrations requires WHERE migration_key = <literal>".to_string(),
            )),
        }
    }

    async fn try_execute_system_migrations_dml(
        &self,
        sql: &str,
        metadata: &PreparedExecutionStatement,
        dml_kind: DmlKind,
    ) -> Result<Option<ExecutionResult>, KalamDbError> {
        if !Self::is_system_migrations_table(metadata.table_id.as_ref()) {
            return Ok(None);
        }

        match dml_kind {
            DmlKind::Insert => {
                let statement = Self::parse_single_statement(sql)?;
                let Statement::Insert(insert) = statement else {
                    return Ok(None);
                };
                match &insert.table {
                    TableObject::TableName(name) => Self::expect_system_migrations_object(name)?,
                    TableObject::TableFunction(_) | TableObject::TableQuery(_) => {
                        return Err(KalamDbError::InvalidSql(
                            "INSERT system.migrations requires a table name".to_string(),
                        ));
                    },
                }

                let Some(source) = insert.source.as_ref() else {
                    return Err(KalamDbError::InvalidSql(
                        "INSERT system.migrations requires VALUES".to_string(),
                    ));
                };
                if source.with.is_some()
                    || source.order_by.is_some()
                    || source.limit_clause.is_some()
                {
                    return Err(KalamDbError::InvalidSql(
                        "INSERT system.migrations supports VALUES only".to_string(),
                    ));
                }
                let row = kalamdb_sql::single_values_insert_row(&insert).map_err(|reason| {
                    KalamDbError::InvalidSql(match reason {
                        "insert requires values" => {
                            "INSERT system.migrations requires VALUES".to_string()
                        },
                        "insert supports exactly one row" => {
                            "INSERT system.migrations supports exactly one row".to_string()
                        },
                        other => other.to_string(),
                    })
                })?;
                if insert.columns.len() != row.len() {
                    return Err(KalamDbError::InvalidSql(
                        "INSERT system.migrations column count does not match value count"
                            .to_string(),
                    ));
                }

                let values = insert
                    .columns
                    .iter()
                    .zip(row.iter())
                    .map(|(column, value)| {
                        (
                            kalamdb_sql::object_name_to_string(column)
                                .unwrap_or_default()
                                .to_ascii_lowercase(),
                            value,
                        )
                    })
                    .collect::<HashMap<_, _>>();
                let migration = Self::migration_from_insert_values(&values)?;
                self.app_context
                    .system_tables()
                    .migrations()
                    .upsert_migration_async(migration)
                    .await?;
                Ok(Some(ExecutionResult::Inserted { rows_affected: 1 }))
            },
            DmlKind::Update => {
                let statement = Self::parse_single_statement(sql)?;
                let Statement::Update(update) = statement else {
                    return Ok(None);
                };
                if !update.table.joins.is_empty() {
                    return Err(KalamDbError::InvalidSql(
                        "UPDATE system.migrations does not support joins".to_string(),
                    ));
                }
                match &update.table.relation {
                    TableFactor::Table { name, .. } => Self::expect_system_migrations_object(name)?,
                    _ => {
                        return Err(KalamDbError::InvalidSql(
                            "UPDATE system.migrations requires a table name".to_string(),
                        ));
                    },
                }

                let migration_key =
                    Self::migration_key_from_update_selection(update.selection.as_ref())?;
                let Some(mut migration) = self
                    .app_context
                    .system_tables()
                    .migrations()
                    .get_migration_async(&migration_key)
                    .await?
                else {
                    return Ok(Some(ExecutionResult::Updated { rows_affected: 0 }));
                };

                for assignment in &update.assignments {
                    Self::apply_migration_assignment(&mut migration, assignment)?;
                }
                self.app_context
                    .system_tables()
                    .migrations()
                    .upsert_migration_async(migration)
                    .await?;
                Ok(Some(ExecutionResult::Updated { rows_affected: 1 }))
            },
            DmlKind::Delete => Err(KalamDbError::InvalidOperation(
                "DELETE system.migrations is not supported".to_string(),
            )),
        }
    }

    /// Try to extract a typed `KalamDbError::NotLeader` from a `DataFusionError`.
    ///
    /// Table providers wrap [`kalamdb_commons::NotLeaderError`] inside
    /// `DataFusionError::External(...)`.  This method downcasts back to the
    /// concrete type — no string parsing required.
    fn try_not_leader_error(e: &datafusion::error::DataFusionError) -> Option<KalamDbError> {
        if let datafusion::error::DataFusionError::External(inner) = e {
            if let Some(nle) = inner.downcast_ref::<kalamdb_commons::NotLeaderError>() {
                return Some(KalamDbError::NotLeader {
                    leader_addr: nle.leader_addr.clone(),
                });
            }
        }
        None
    }

    /// Convert a `DataFusionError` into a `KalamDbError`, preserving
    /// `NotLeader` semantics when the error wraps a [`kalamdb_commons::NotLeaderError`].
    fn datafusion_to_execution_error(e: datafusion::error::DataFusionError) -> KalamDbError {
        Self::classify_datafusion_error(&e)
    }

    fn is_table_not_found_error(e: &datafusion::error::DataFusionError) -> bool {
        Self::is_table_not_found_msg(&e.to_string().to_lowercase())
    }

    fn is_table_not_found_msg(msg: &str) -> bool {
        (msg.contains("table") && msg.contains("not found"))
            || (msg.contains("relation") && msg.contains("does not exist"))
            || msg.contains("unknown table")
    }

    fn is_permission_msg(msg: &str) -> bool {
        msg.contains("access denied")
            || msg.contains("permission denied")
            || msg.contains("unauthorized")
            || msg.contains("not authorized")
            || msg.contains("forbidden")
            || msg.contains("insufficient privileges")
    }

    fn is_column_not_found_msg(msg: &str) -> bool {
        (msg.contains("column") && msg.contains("not found"))
            || (msg.contains("field") && msg.contains("not found"))
            || msg.contains("no field named")
            || msg.contains("schema error: no field named")
    }

    fn is_constraint_violation_msg(msg: &str) -> bool {
        msg.contains("primary key")
            || msg.contains("constraint violation")
            || msg.contains("already exists")
            || msg.contains("duplicate")
            || msg.contains("unique constraint")
            || msg.contains("unique index")
    }

    fn datafusion_user_message(e: &datafusion::error::DataFusionError) -> String {
        match e {
            datafusion::error::DataFusionError::SQL(parser_error, _) => parser_error.to_string(),
            datafusion::error::DataFusionError::SchemaError(schema_error, _) => {
                schema_error.to_string()
            },
            datafusion::error::DataFusionError::Plan(message)
            | datafusion::error::DataFusionError::Execution(message)
            | datafusion::error::DataFusionError::Internal(message)
            | datafusion::error::DataFusionError::ResourcesExhausted(message)
            | datafusion::error::DataFusionError::NotImplemented(message) => message.clone(),
            datafusion::error::DataFusionError::Context(context, source) => {
                format!("{context}: {}", Self::datafusion_user_message(source))
            },
            datafusion::error::DataFusionError::ArrowError(error, _) => error.to_string(),
            datafusion::error::DataFusionError::External(error) => error.to_string(),
            _ => e.to_string(),
        }
    }

    /// Classify a DataFusionError into a KalamDbError while preserving the
    /// underlying user-facing parser/planner/execution message when available.
    fn classify_datafusion_error(e: &datafusion::error::DataFusionError) -> KalamDbError {
        if let Some(not_leader) = Self::try_not_leader_error(e) {
            return not_leader;
        }

        let error_msg = Self::datafusion_user_message(e);
        let lower = error_msg.to_lowercase();

        if Self::is_table_not_found_msg(&lower) {
            return KalamDbError::TableNotFound(error_msg);
        }

        if Self::is_permission_msg(&lower) {
            return KalamDbError::PermissionDenied(error_msg);
        }

        if Self::is_column_not_found_msg(&lower) {
            return KalamDbError::InvalidOperation(error_msg);
        }

        if Self::is_constraint_violation_msg(&lower) {
            return KalamDbError::AlreadyExists(error_msg);
        }

        KalamDbError::ExecutionError(error_msg)
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
            SqlStatementKind::CreateNamespace(_)
                | SqlStatementKind::AlterNamespace(_)
                | SqlStatementKind::DropNamespace(_)
                | SqlStatementKind::CreateStorage(_)
                | SqlStatementKind::AlterStorage(_)
                | SqlStatementKind::DropStorage(_)
                | SqlStatementKind::CreateTable(_)
                | SqlStatementKind::CreateView(_)
                | SqlStatementKind::AlterTable(_)
                | SqlStatementKind::DropTable(_)
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
        let Some(transaction_id) = self.active_request_transaction_id(exec_ctx)? else {
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
            handle.snapshot_commit_seq,
            Arc::new(crate::transactions::CoordinatorOverlayView::new(
                Arc::clone(&coordinator),
                transaction_id.clone(),
            )),
            Arc::new(crate::transactions::CoordinatorMutationSink::new(coordinator)),
            Arc::new(CoordinatorAccessValidator::new(self.app_context.transaction_coordinator())),
        )))
    }

    fn create_session_with_transaction_context(
        &self,
        exec_ctx: &ExecutionContext,
    ) -> Result<SessionContext, KalamDbError> {
        let Some(transaction_query_context) =
            self.transaction_query_context_for_request(exec_ctx)?
        else {
            return Ok(exec_ctx.create_session_with_user());
        };

        let mut state = exec_ctx.build_user_session_state();
        state
            .config_mut()
            .options_mut()
            .extensions
            .insert(TransactionQueryExtension::new(transaction_query_context));
        Ok(SessionContext::new_with_state(state))
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
                "transaction '{}' failed: stream tables are not supported inside explicit transactions",
                transaction_id
            ))),
            Some(TableType::System) => Err(KalamDbError::InvalidOperation(format!(
                "transaction '{}' failed: system tables are not supported inside explicit transactions",
                transaction_id
            ))),
            _ => Ok(()),
        }
    }

    /// Construct a new executor with a pre-built handler registry.
    pub fn new(
        app_context: std::sync::Arc<crate::app_context::AppContext>,
        handler_registry: Arc<HandlerRegistry>,
    ) -> Self {
        let sql_cache_registry = Arc::new(SqlCacheRegistry::new(SqlCacheRegistryConfig::new(
            app_context.config().execution.sql_plan_cache_max_entries,
            Duration::from_secs(app_context.config().execution.sql_plan_cache_ttl_seconds),
        )));
        Self {
            app_context,
            handler_registry,
            sql_cache_registry,
        }
    }

    /// Clear SQL caches that may become stale after DDL operations.
    pub fn clear_plan_cache(&self) {
        self.sql_cache_registry.clear();
    }

    /// Get current plan cache size (diagnostics/testing)
    pub fn plan_cache_len(&self) -> usize {
        self.sql_cache_registry.plan_cache().len()
    }

    /// Batch-execute multiple INSERT statements targeting the same table in an
    /// active explicit transaction via the transaction batch insert path.
    ///
    /// Returns `Ok(Some(results))` with per-statement `ExecutionResult::Inserted`,
    /// `Ok(None)` if the batch path is not applicable (caller should fall back to
    /// per-statement execution), or `Err(e)` on execution failure.
    pub fn try_batch_insert_in_transaction(
        &self,
        statements: &[&PreparedExecutionStatement],
        exec_ctx: &ExecutionContext,
        transaction_id: &TransactionId,
    ) -> Result<Option<Vec<crate::sql::ExecutionResult>>, KalamDbError> {
        let batch_sql_len = statements.iter().map(|statement| statement.sql.len()).sum::<usize>()
            + statements.len().saturating_sub(1) * 2;
        let mut batch_sql = String::with_capacity(batch_sql_len);
        for (idx, statement) in statements.iter().enumerate() {
            if idx > 0 {
                batch_sql.push_str("; ");
            }
            batch_sql.push_str(statement.sql.as_str());
        }
        let dialect = sqlparser::dialect::GenericDialect {};
        let parsed_stmts_storage =
            kalamdb_sql::parser::utils::parse_sql_statements(&batch_sql, &dialect)
                .map_err(|error| KalamDbError::InvalidSql(error.to_string()))?;

        if parsed_stmts_storage.len() != statements.len() {
            return Ok(None);
        }

        let parsed_stmts: Vec<&sqlparser::ast::Statement> = parsed_stmts_storage.iter().collect();

        let table_id = match statements[0].table_id.as_ref() {
            Some(id) => id,
            None => return Ok(None),
        };

        match super::transaction_batch_insert::try_batch_inserts_in_transaction(
            &parsed_stmts,
            self.app_context.as_ref(),
            self.sql_cache_registry.as_ref(),
            exec_ctx,
            table_id,
            transaction_id,
        )? {
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
        self.prepare_statement_metadata_for_role(
            sql,
            &exec_ctx.default_namespace(),
            exec_ctx.user_role(),
        )
    }

    pub fn prepare_statement_metadata_for_role(
        &self,
        sql: &str,
        default_namespace: &NamespaceId,
        role: Role,
    ) -> Result<PreparedExecutionStatement, StatementClassificationError> {
        let classified = SqlStatement::classify_and_parse(sql, default_namespace, role)?;
        let table_id = match classified.kind() {
            SqlStatementKind::Insert(_)
            | SqlStatementKind::Update(_)
            | SqlStatementKind::Delete(_) => {
                kalamdb_sql::extract_dml_table_id_fast(sql, default_namespace.as_str())
                    .or_else(|| kalamdb_sql::extract_dml_table_id(sql, default_namespace.as_str()))
            },
            _ => None,
        };
        let table_type = table_id.as_ref().and_then(|table_id| {
            self.app_context
                .schema_registry()
                .get(table_id)
                .map(|cached| cached.table_entry().table_type)
        });

        Ok(PreparedExecutionStatement::new(
            sql.to_string(),
            table_id,
            table_type,
            Some(classified),
        ))
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
            let classified = match metadata.classified_statement.as_ref() {
                Some(classified) => classified,
                None => {
                    kalamdb_observability::observe_query(
                        kalamdb_observability::QueryMetricKind::Other,
                        query_start.elapsed(),
                        true,
                    );
                    return Err(KalamDbError::InvalidSql(
                        "Missing pre-classified statement metadata for SQL execution".to_string(),
                    ));
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
                        table_id.namespace_id().as_str(),
                    )
                } else if !kalamdb_observability::should_observe_query_namespace(
                    exec_ctx.default_namespace().as_str(),
                ) {
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

                    // Hot path: SELECT queries use DataFusion
                    // Tables are already registered in base session, we just inject user_id
                    SqlStatementKind::Select => {
                        self.execute_via_datafusion(classified.as_str(), params, exec_ctx).await
                    },

                    // DataFusion meta commands (EXPLAIN, SET, SHOW, etc.) - admin only
                    // No caching needed - these are diagnostic/config commands
                    // Authorization already checked in classifier
                    SqlStatementKind::DataFusionMetaCommand => {
                        self.execute_meta_command(sql, exec_ctx).await
                    },

                    // Native DataFusion DML path (provider insert/update/delete hooks)
                    SqlStatementKind::Insert(_) => {
                        if let Err(error) = self
                            .reject_unsupported_dml_in_active_request_transaction(
                                metadata, exec_ctx,
                            )
                            .await
                        {
                            Err(error)
                        } else {
                            match self
                                .try_execute_literal_insert_via_applier(
                                    classified.as_str(),
                                    metadata,
                                    exec_ctx,
                                    &params,
                                )
                                .await
                            {
                                Ok(Some(result)) => Ok(result),
                                Ok(None) => {
                                    self.execute_dml_via_datafusion(
                                        classified.as_str(),
                                        metadata,
                                        params,
                                        exec_ctx,
                                        DmlKind::Insert,
                                    )
                                    .await
                                },
                                Err(error) => Err(error),
                            }
                        }
                    },
                    SqlStatementKind::Update(_) => {
                        if let Err(error) = self
                            .reject_unsupported_dml_in_active_request_transaction(
                                metadata, exec_ctx,
                            )
                            .await
                        {
                            Err(error)
                        } else {
                            self.execute_dml_via_datafusion(
                                classified.as_str(),
                                metadata,
                                params,
                                exec_ctx,
                                DmlKind::Update,
                            )
                            .await
                        }
                    },
                    SqlStatementKind::Delete(_) => {
                        if let Err(error) = self
                            .reject_unsupported_dml_in_active_request_transaction(
                                metadata, exec_ctx,
                            )
                            .await
                        {
                            Err(error)
                        } else {
                            self.execute_dml_via_datafusion(
                                classified.as_str(),
                                metadata,
                                params,
                                exec_ctx,
                                DmlKind::Delete,
                            )
                            .await
                        }
                    },

                    // DDL operations that modify table/view structure require plan cache invalidation
                    // This prevents stale cached plans from referencing dropped/altered tables
                    SqlStatementKind::CreateTable(_)
                    | SqlStatementKind::DropTable(_)
                    | SqlStatementKind::AlterTable(_)
                    | SqlStatementKind::CreateView(_)
                    | SqlStatementKind::CreateNamespace(_)
                    | SqlStatementKind::DropNamespace(_) => {
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
            if observe_query_metrics {
                kalamdb_observability::observe_query(
                    query_kind,
                    query_start.elapsed(),
                    result.is_err(),
                );
            }

            #[cfg(feature = "traceability")]
            if let Ok(ref res) = result {
                let rows = match res {
                    ExecutionResult::Rows { row_count, .. } => *row_count,
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
            rows = tracing::field::Empty,
        )
    }

    fn should_stage_autocommit_dml(
        &self,
        metadata: &PreparedExecutionStatement,
        exec_ctx: &ExecutionContext,
    ) -> Result<bool, KalamDbError> {
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

        let request_transaction_coordinator =
            AppContextRequestTransactionCoordinator::new(self.app_context.as_ref());
        let mut request_state = RequestTransactionState::from_request_id(dml_exec_ctx.request_id())
            .ok_or_else(|| {
                KalamDbError::InvalidOperation(
                    "autocommit DML requires a request-scoped execution context".to_string(),
                )
            })?;
        request_state.sync(&request_transaction_coordinator);

        if request_state.is_active() {
            return self
                .execute_dml_via_datafusion_inner(sql, metadata, params, dml_exec_ctx, dml_kind)
                .await;
        }

        request_state
            .begin(&request_transaction_coordinator)
            .map_err(map_request_transaction_error)?;
        let result = self
            .execute_dml_via_datafusion_inner(sql, metadata, params, dml_exec_ctx, dml_kind)
            .await;

        match result {
            Ok(result) => match request_state.commit(&request_transaction_coordinator).await {
                Ok(_) => Ok(result),
                Err(error) => {
                    let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                    Err(map_request_transaction_error(error))
                },
            },
            Err(error) => {
                let _ = request_state.rollback_if_active(&request_transaction_coordinator);
                Err(error)
            },
        }
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
        original_sql: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<(SessionContext, DataFrame), KalamDbError> {
        let session = self.create_session_with_transaction_context(exec_ctx)?;
        #[cfg(feature = "traceability")]
        let plan_start = std::time::Instant::now();

        match session.sql(execution_sql).await {
            Ok(df) => {
                kalamdb_observability::kdb_debug!(
                    plan_ms = (plan_start.elapsed().as_micros() as f64 / 1000.0),
                    "sql.dml_plan"
                );
                Ok((session, df))
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
                let retry_df = retry_session.sql(execution_sql).await.map_err(|retry_error| {
                    self.log_sql_error(original_sql, exec_ctx, retry_error)
                })?;
                kalamdb_observability::kdb_debug!(
                    plan_ms = (retry_start.elapsed().as_micros() as f64 / 1000.0),
                    reloaded_providers = true,
                    "sql.dml_plan"
                );
                Ok((retry_session, retry_df))
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
            rows_affected = tracing::field::Empty,
        )
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
            if let Some(result) =
                self.try_execute_system_migrations_dml(sql, metadata, dml_kind).await?
            {
                return Ok(result);
            }
        }

        self.block_system_namespace_dml(metadata.table_id.as_ref(), dml_kind)?;

        let execution_sql = kalamdb_sql::rewrite_context_functions_for_datafusion(sql);
        let execution_sql: &str = &execution_sql;

        if !params.is_empty() {
            validate_params(&params)?;
        }

        // Parameterized DML: reuse cached template plans and only bind placeholders per request.
        // This avoids reparsing/replanning the same INSERT/UPDATE/DELETE shape repeatedly.
        let df = if params.is_empty() {
            self.plan_dml_with_provider_reload(execution_sql, sql, exec_ctx).await?.1
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
                        if let Some(not_leader_err) = Self::try_not_leader_error(&error) {
                            return Err(not_leader_err);
                        }

                        log::warn!(
                            target: "sql::dml",
                            "Failed to execute cached DML plan, reparsing SQL: {}",
                            error
                        );

                        let (plan_session, planned_df) = self
                            .plan_dml_with_provider_reload(execution_sql, sql, exec_ctx)
                            .await?;
                        let rebound_plan =
                            self.cache_and_bind_dml_plan(&cache_key, planned_df, &params)?;
                        plan_session
                            .execute_logical_plan(rebound_plan)
                            .await
                            .map_err(Self::datafusion_to_execution_error)?
                    },
                }
            } else {
                let (plan_session, planned_df) =
                    self.plan_dml_with_provider_reload(execution_sql, sql, exec_ctx).await?;
                let bound_plan = self.cache_and_bind_dml_plan(&cache_key, planned_df, &params)?;
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
        let execution_sql = kalamdb_sql::rewrite_context_functions_for_datafusion(sql);
        let execution_sql: &str = &execution_sql;

        // Validate parameters if present
        if !params.is_empty() {
            validate_params(&params)?;
        }

        // Try cached template plan first (works for both plain and parameterized SQL).
        // Key excludes user_id because LogicalPlan is user-agnostic - filtering happens at scan
        // time.
        let cache_key =
            PlanCacheKey::new(exec_ctx.default_namespace(), exec_ctx.user_role(), execution_sql);
        let session = self.create_session_with_transaction_context(exec_ctx)?;

        let df = if let Some(template_plan) = self.sql_cache_registry.plan_cache().get(&cache_key) {
            let executable_plan = if params.is_empty() {
                (*template_plan).clone()
            } else {
                replace_placeholders_in_plan((*template_plan).clone(), &params)?
            };

            match session.execute_logical_plan(executable_plan).await {
                Ok(df) => df,
                Err(e) => {
                    log::warn!("Failed to execute cached plan, reparsing SQL: {}", e);
                    let planned_df = match session.sql(execution_sql).await {
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
                                match retry_session.sql(execution_sql).await {
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

                    let template_plan = planned_df.logical_plan().clone();
                    self.sql_cache_registry
                        .plan_cache()
                        .insert(cache_key.clone(), template_plan.clone());

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
            let planned_df = match session.sql(execution_sql).await {
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
                        match retry_session.sql(execution_sql).await {
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

            // Cache the user-requested plan exactly. Implicit ordering is intentionally avoided on
            // the SELECT hot path because it can force full sorts on scans, filters, and Parquet
            // reads. Stable result order remains the caller's responsibility via explicit
            // ORDER BY.
            let template_plan = planned_df.logical_plan().clone();
            self.sql_cache_registry.plan_cache().insert(cache_key, template_plan.clone());

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

        let df = self.apply_select_limits(df)?;

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
    /// These commands are passed directly to DataFusion without custom parsing.
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
        let execution_sql = kalamdb_sql::rewrite_context_functions_for_datafusion(sql);
        let execution_sql: &str = &execution_sql;
        // Create per-request SessionContext with user_id injected
        let session = self.create_session_with_transaction_context(exec_ctx)?;

        // Execute the command directly via DataFusion
        let df = match session.sql(execution_sql).await {
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

        let row_count: usize = batches.iter().map(|b| b.num_rows()).sum();

        // log::debug!(
        //     target: "sql::meta",
        //     "✅ Meta command completed | sql='{}' | rows={}",
        //     sql,
        //     row_count
        // );

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
    use super::*;

    #[test]
    fn internal_namespace_hint_matches_qualified_system_and_dba_queries() {
        assert!(contains_internal_namespace_hint("SELECT * FROM system.stats LIMIT 100"));
        assert!(contains_internal_namespace_hint("select * from DBA.notifications"));
        assert!(!contains_internal_namespace_hint("SELECT * FROM default.events LIMIT 100"));
    }
}
