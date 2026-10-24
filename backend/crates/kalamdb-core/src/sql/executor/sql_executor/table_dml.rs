//! INSERT, UPDATE, and DELETE through DataFusion providers and appliers.

use std::sync::Arc;

use arrow::array::{Array, RecordBatch};
use datafusion::{
    dataframe::DataFrame, logical_expr::LogicalPlan, prelude::SessionContext, scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::arrow_json_conversion::arrow_value_to_scalar,
    models::{TableId, TransactionId},
    schemas::TableType,
    SystemTable,
};
use sqlparser::ast::Statement;
use uuid::Uuid;

use super::DmlKind;
use crate::{
    error::KalamDbError,
    sql::{
        executor::{
            parameter_binding::{replace_placeholders_in_plan, validate_params},
            PreparedExecutionStatement, SqlExecutor,
        },
        plan_cache::PlanCacheKey,
        ExecutionContext, ExecutionResult,
    },
};

impl SqlExecutor {
    pub(super) fn dml_operation_name(dml_kind: DmlKind) -> &'static str {
        match dml_kind {
            DmlKind::Insert => "INSERT",
            DmlKind::Update => "UPDATE",
            DmlKind::Delete => "DELETE",
        }
    }

    pub(super) fn block_system_namespace_dml(
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

    pub(super) async fn execute_table_dml(
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

        match crate::sql::executor::transaction_batch_insert::try_batch_inserts_in_transaction(
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

    pub(super) fn should_stage_autocommit_dml(
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

    pub(super) async fn execute_autocommit_dml_via_transaction(
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
                self.execute_dml_via_datafusion_inner(sql, metadata, params, ctx, dml_kind)
                    .await
            },
        )
        .await
    }

    pub(super) async fn execute_dml_via_datafusion(
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

    pub(super) async fn plan_dml_with_provider_reload(
        &self,
        pre_parsed: Option<&Statement>,
        original_sql: &str,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<(SessionContext, DataFrame, bool), KalamDbError> {
        let session = self.create_session_with_transaction_context(exec_ctx)?;
        let mut statement = match pre_parsed {
            Some(stmt) => stmt.clone(),
            None => Self::parse_statement_for_datafusion_exec(original_sql)?,
        };
        let had_catalog_func = self
            .evaluate_catalog_functions_in_statement(&mut statement, exec_ctx, params)
            .await?;
        let (session, dataframe) = self
            .dataframe_after_provider_reload(
                session,
                &statement,
                original_sql,
                exec_ctx,
                super::planning::ProviderReload::Dml,
            )
            .await?;
        Ok((session, dataframe, had_catalog_func))
    }

    /// Erased so the recursive sqlparser AST is not nested into every caller future.
    /// A concrete future here overflows the compiler `Sync` check.
    fn execute_replanned_dml_erased<'a>(
        &'a self,
        cache_key: PlanCacheKey,
        pre_parsed: Option<&'a Statement>,
        sql: &'a str,
        exec_ctx: &'a ExecutionContext,
        params: &'a [ScalarValue],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<DataFrame, KalamDbError>> + Send + 'a>,
    > {
        Box::pin(self.execute_replanned_dml(cache_key, pre_parsed, sql, exec_ctx, params))
    }

    async fn execute_replanned_dml(
        &self,
        cache_key: PlanCacheKey,
        pre_parsed: Option<&Statement>,
        sql: &str,
        exec_ctx: &ExecutionContext,
        params: &[ScalarValue],
    ) -> Result<DataFrame, KalamDbError> {
        let (plan_session, planned_df, had_catalog_func) =
            self.plan_dml_with_provider_reload(pre_parsed, sql, exec_ctx, params).await?;
        let bound_plan = if had_catalog_func {
            replace_placeholders_in_plan(planned_df.logical_plan().clone(), params)?
        } else {
            self.cache_and_bind_dml_plan(cache_key, planned_df, params)?
        };
        plan_session
            .execute_logical_plan(bound_plan)
            .await
            .map_err(Self::datafusion_to_execution_error)
    }

    pub(super) fn cache_and_bind_dml_plan(
        &self,
        cache_key: PlanCacheKey,
        planned_df: DataFrame,
        params: &[ScalarValue],
    ) -> Result<LogicalPlan, KalamDbError> {
        let template_plan = planned_df.logical_plan().clone();
        let bound_plan = replace_placeholders_in_plan(template_plan.clone(), params)?;
        self.sql_cache_registry.plan_cache().insert(cache_key, template_plan);
        Ok(bound_plan)
    }

    #[cfg_attr(feature = "traceability", tracing::instrument(
        name = "sql.dml_datafusion",
        skip_all,
        fields(
            dml_kind = %Self::dml_operation_name(dml_kind),
            rows_affected = tracing::field::Empty)
    ))]
    pub(super) async fn execute_dml_via_datafusion_inner(
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

        let pre_parsed = if sql == metadata.sql.as_str() {
            metadata.datafusion_statement.as_deref().or(metadata.parsed_dml.as_deref())
        } else {
            None
        };

        if !params.is_empty() {
            validate_params(&params)?;
        }

        // Parameterized DML: reuse cached template plans and only bind placeholders per request.
        // This avoids reparsing/replanning the same INSERT/UPDATE/DELETE shape repeatedly.
        let df = if params.is_empty() {
            self.plan_dml_with_provider_reload(pre_parsed, sql, exec_ctx, &params).await?.1
        } else {
            let cache_key =
                PlanCacheKey::new(exec_ctx.default_namespace(), exec_ctx.user_role(), sql);
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

                        self.execute_replanned_dml_erased(
                            cache_key, pre_parsed, sql, exec_ctx, &params,
                        )
                        .await?
                    },
                }
            } else {
                self.execute_replanned_dml_erased(cache_key, pre_parsed, sql, exec_ctx, &params)
                    .await?
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

    pub(super) fn extract_rows_affected(batches: &[RecordBatch]) -> Result<usize, KalamDbError> {
        let mut total: usize = 0;

        for batch in batches {
            if batch.num_columns() == 0 || batch.num_rows() == 0 {
                continue;
            }

            let count_column = batch.column_by_name("count").unwrap_or_else(|| batch.column(0));

            if let Some(u64_arr) = count_column.as_any().downcast_ref::<arrow::array::UInt64Array>()
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

    pub(super) fn scalar_count_to_usize(value: ScalarValue) -> Result<usize, KalamDbError> {
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
}
