//! SQL dispatch for the shared session used by HTTP and pgwire.

use std::{sync::Arc, time::Instant};

use datafusion::{dataframe::DataFrame, prelude::SessionContext, scalar::ScalarValue};
use kalamdb_commons::{models::SchemaId, Role};
use kalamdb_session_datafusion::{install_extension, ScanDiagnosticsContext};
use kalamdb_sql::{classifier::SqlStatementKind, rewrite_explain_for_datafusion};
use sqlparser::ast::Statement;

use super::{contains_internal_namespace_hint, DmlKind};
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

enum SelectExecFailure {
    Initial,
    AfterReplan,
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

fn scalar_cache_tag(value: &ScalarValue) -> &'static str {
    match value {
        ScalarValue::Int64(_) => "i64",
        ScalarValue::Float64(_) => "f64",
        ScalarValue::Utf8(_) | ScalarValue::LargeUtf8(_) => "utf8",
        ScalarValue::Boolean(_) => "bool",
        _ => "other",
    }
}

impl SqlExecutor {
    /// Plan identity for one statement: namespace, role, SQL, search_path, epochs.
    pub(super) fn logical_plan_cache_key(
        &self,
        exec_ctx: &ExecutionContext,
        sql: &str,
    ) -> PlanCacheKey {
        let search_path = exec_ctx.session_config().0.get("search_path").unwrap_or_default();
        self.sql_cache_registry
            .plan_cache()
            .current_key(exec_ctx.default_namespace(), exec_ctx.user_role(), sql)
            .search_path(search_path)
    }

    /// Placeholder form of a statement whose only differences are scalar literals.
    ///
    /// `None` leaves the original statement on the exact-SQL plan path. User-defined
    /// routines stay there so their rewrite still sees the original literals.
    pub(super) fn literal_plan_shape(
        &self,
        mut statement: Statement,
    ) -> Option<(String, Vec<ScalarValue>, Statement)> {
        if self.app_context.system_tables().catalog_stores().has_user_routines() {
            return None;
        }
        let values = kalamdb_sql::parameterize_literals(&mut statement)?;
        let mut cache_sql = statement.to_string();
        cache_sql.push('\u{1}');
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                cache_sql.push(',');
            }
            cache_sql.push_str(scalar_cache_tag(value));
        }
        Some((cache_sql, values, statement))
    }

    fn select_literal_shape(
        &self,
        sql: &str,
        metadata: Option<&PreparedExecutionStatement>,
        params: &[ScalarValue],
    ) -> Option<(String, Vec<ScalarValue>, Statement)> {
        if !params.is_empty() {
            return None;
        }
        let statement = Self::statement_for_datafusion(sql, metadata).ok()?;
        self.literal_plan_shape(statement)
    }

    pub(super) fn map_classification_error(
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
                }};

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
                    SqlStatementKind::PostgresSetting(command) => Self::execute_postgres_setting(command, exec_ctx),
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
                        message: "SET TRANSACTION".to_string()}),
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
                            Some(metadata),
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
                            message: "SET".to_string()})
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
                    |                     SqlStatementKind::DropDomain(_) => {
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

                    SqlStatementKind::GrantExecute(_)
                    | SqlStatementKind::RevokeExecute(_)
                    | SqlStatementKind::CreateUser(_)
                    | SqlStatementKind::AlterUser(_)
                    | SqlStatementKind::DropUser(_) => {
                        let result = self
                            .handler_registry
                            .handle(classified.clone(), params, exec_ctx)
                            .await;
                        if result.is_ok() {
                            self.sql_cache_registry.plan_cache().bump_permission_epoch();
                        }
                        result
                    },

                    // All other statements: Delegate to handler registry (no cache invalidation needed)
                    _ => self.handler_registry.handle(classified.clone(), params, exec_ctx).await}
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
                    _ => 0};
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

    /// Erased so the recursive sqlparser AST is not nested into every caller future.
    /// A concrete future here overflows the compiler `Sync` check.
    fn plan_bind_and_execute_select_erased<'a>(
        &'a self,
        session: SessionContext,
        statement: Statement,
        sql: &'a str,
        params: &'a [ScalarValue],
        exec_ctx: &'a ExecutionContext,
        default_namespace: &'a SchemaId,
        cache_key: PlanCacheKey,
        had_catalog_func: bool,
        failure: SelectExecFailure,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<DataFrame, KalamDbError>> + Send + 'a>,
    > {
        Box::pin(self.plan_bind_and_execute_select(
            session,
            statement,
            sql,
            params,
            exec_ctx,
            default_namespace,
            cache_key,
            had_catalog_func,
            failure,
        ))
    }

    async fn plan_bind_and_execute_select(
        &self,
        session: SessionContext,
        mut statement: Statement,
        sql: &str,
        params: &[ScalarValue],
        exec_ctx: &ExecutionContext,
        default_namespace: &SchemaId,
        cache_key: PlanCacheKey,
        mut had_catalog_func: bool,
        failure: SelectExecFailure,
    ) -> Result<DataFrame, KalamDbError> {
        had_catalog_func |= self
            .evaluate_catalog_functions_in_statement(&mut statement, exec_ctx, params)
            .await?;
        let (session, planned_df) = self
            .dataframe_after_provider_reload(
                session,
                &statement,
                sql,
                exec_ctx,
                super::planning::ProviderReload::Select,
            )
            .await?;
        let template_plan =
            self.optimized_plan_for_cache(&session, &planned_df, default_namespace).await?;
        if !had_catalog_func {
            self.sql_cache_registry.plan_cache().insert(cache_key, template_plan.clone());
        }
        let executable_plan = replace_placeholders_in_plan(template_plan, params)?;
        match session.execute_logical_plan(executable_plan).await {
            Ok(dataframe) => Ok(dataframe),
            Err(error) => {
                if let Some(not_leader_err) = Self::try_not_leader_error(&error) {
                    return Err(not_leader_err);
                }
                match failure {
                    SelectExecFailure::Initial => log::error!(
                        target: "sql::exec",
                        "❌ SQL execution failed | sql='{}' | params={} | error='{}'",
                        sql,
                        params.len(),
                        error
                    ),
                    SelectExecFailure::AfterReplan => log::error!(
                        target: "sql::exec",
                        "❌ SQL execution failed after replan | sql='{}' | params={} | error='{}'",
                        sql,
                        params.len(),
                        error
                    ),
                }
                Err(Self::datafusion_to_execution_error(error))
            },
        }
    }

    /// Execute SELECT via DataFusion with per-user session
    #[cfg_attr(feature = "traceability", tracing::instrument(
        name = "sql.select_datafusion",
        skip_all,
        fields(row_count = tracing::field::Empty)
    ))]
    pub(super) async fn execute_via_datafusion(
        &self,
        sql: &str,
        metadata: Option<&PreparedExecutionStatement>,
        params: Vec<ScalarValue>,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let isolated;
        let exec_ctx = if self
            .is_stream_target_sql(sql, metadata.and_then(|m| m.table_id.as_ref()), exec_ctx)
            .await?
        {
            isolated = Self::isolated_stream_exec_ctx(exec_ctx);
            &isolated
        } else {
            exec_ctx
        };
        let default_namespace = exec_ctx.default_namespace();

        // Validate parameters if present
        if !params.is_empty() {
            validate_params(&params)?;
        }

        // Exact SQL hits skip parsing. A miss with only literal differences reuses the
        // placeholder shape. The key includes search_path and the current epochs.
        // It excludes user_id because filtering happens at scan time.
        let cache_key = self.logical_plan_cache_key(exec_ctx, sql);

        let df = if let Some(template_plan) = self.sql_cache_registry.plan_cache().get(&cache_key) {
            if let Some(result) = kalamdb_observability::kdb_await_in_info_span!(
                self.try_execute_cached_point_get(template_plan.as_ref(), &params, exec_ctx),
                "sql.point_get"
            )? {
                return Ok(result);
            }
            if let Some(result) = self
                .try_execute_cached_covering_aggregate(template_plan.as_ref(), &params, exec_ctx)
                .await?
            {
                return Ok(result);
            }

            let executable_plan = replace_placeholders_in_plan((*template_plan).clone(), &params)?;

            let session = self.create_session_with_transaction_context(exec_ctx)?;
            match session.execute_logical_plan(executable_plan).await {
                Ok(df) => df,
                Err(error) => {
                    log::warn!("Failed to execute cached plan, reparsing SQL: {}", error);
                    let statement = Self::statement_for_datafusion(sql, metadata)?;
                    self.plan_bind_and_execute_select_erased(
                        session,
                        statement,
                        sql,
                        &params,
                        exec_ctx,
                        &default_namespace,
                        cache_key,
                        false,
                        SelectExecFailure::AfterReplan,
                    )
                    .await?
                },
            }
        } else if let Some((template_sql, bound, mut statement)) =
            self.select_literal_shape(sql, metadata, &params)
        {
            let shape_key = self.logical_plan_cache_key(exec_ctx, &template_sql);
            if let Some(template_plan) = self.sql_cache_registry.plan_cache().get(&shape_key) {
                if let Some(result) = kalamdb_observability::kdb_await_in_info_span!(
                    self.try_execute_cached_point_get(template_plan.as_ref(), &bound, exec_ctx),
                    "sql.point_get"
                )? {
                    return Ok(result);
                }
                if let Some(result) = self
                    .try_execute_cached_covering_aggregate(
                        template_plan.as_ref(),
                        &bound,
                        exec_ctx,
                    )
                    .await?
                {
                    return Ok(result);
                }
                let executable_plan =
                    replace_placeholders_in_plan((*template_plan).clone(), &bound)?;
                let session = self.create_session_with_transaction_context(exec_ctx)?;
                match session.execute_logical_plan(executable_plan).await {
                    Ok(df) => df,
                    Err(error) => {
                        log::warn!("Failed to execute cached literal plan, replanning: {}", error);
                        self.plan_bind_and_execute_select_erased(
                            session,
                            statement,
                            sql,
                            &bound,
                            exec_ctx,
                            &default_namespace,
                            shape_key,
                            false,
                            SelectExecFailure::AfterReplan,
                        )
                        .await?
                    },
                }
            } else {
                let session = self.create_session_with_transaction_context(exec_ctx)?;
                let mut had_catalog_func = false;
                if let Some(direct_result) = self
                    .try_execute_standalone_catalog_function_select(
                        &mut statement,
                        exec_ctx,
                        &bound,
                        &mut had_catalog_func,
                    )
                    .await?
                {
                    return Ok(direct_result);
                }
                self.plan_bind_and_execute_select_erased(
                    session,
                    statement,
                    sql,
                    &bound,
                    exec_ctx,
                    &default_namespace,
                    shape_key,
                    had_catalog_func,
                    SelectExecFailure::Initial,
                )
                .await?
            }
        } else {
            let session = self.create_session_with_transaction_context(exec_ctx)?;
            let mut statement = Self::statement_for_datafusion(sql, metadata)?;
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
            self.plan_bind_and_execute_select_erased(
                session,
                statement,
                sql,
                &params,
                exec_ctx,
                &default_namespace,
                cache_key,
                had_catalog_func,
                SelectExecFailure::Initial,
            )
            .await?
        };

        let df = self.apply_select_limits(df, sql)?;
        let (batches, schema) = Self::collect_dataframe(df)
            .await
            .map_err(|error| self.map_collect_error(sql, exec_ctx, error))?;

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
    pub(super) async fn execute_meta_command(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<ExecutionResult, KalamDbError> {
        let mut execution_sql =
            Self::rewrite_describe_shorthand(sql, exec_ctx).unwrap_or_else(|| sql.to_string());
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

        let (batches, schema) = Self::collect_dataframe(df)
            .await
            .map_err(|error| self.map_collect_error(sql, exec_ctx, error))?;

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

    fn map_collect_error(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
        error: datafusion::error::DataFusionError,
    ) -> KalamDbError {
        if let Some(not_leader_err) = Self::try_not_leader_error(&error) {
            return not_leader_err;
        }
        self.log_sql_error(sql, exec_ctx, error)
    }

    /// Log SQL errors with appropriate level (warn for user errors, error for system errors)
    pub(super) fn log_sql_error(
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
