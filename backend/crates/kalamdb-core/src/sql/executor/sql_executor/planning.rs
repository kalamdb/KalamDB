//! Statement parsing, DataFusion planning, and SELECT limit shaping.

use std::{collections::HashMap, sync::Arc};

use arrow::{array::RecordBatch, datatypes::SchemaRef};
use datafusion::{
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        TableReference,
    },
    dataframe::DataFrame,
    logical_expr::LogicalPlan,
    prelude::SessionContext,
    sql::parser::Statement as DfStatement,
};
use kalamdb_commons::{constants::SystemColumnNames, models::SchemaId};
use sqlparser::ast::{Ident, ObjectName, ObjectNamePart, Statement, TableObject};

/// Which caller is retrying a plan after a missing table provider.
pub(super) enum ProviderReload {
    Select,
    Dml,
}

use crate::{
    error::KalamDbError,
    sql::{
        executor::{
            default_ordering::apply_default_order_by, PreparedExecutionStatement, SqlExecutor,
        },
        ExecutionContext,
    },
};

/// PostgreSQL sessions omit storage columns from a star. The Kalam SQL API
/// keeps them, because live snapshots and row clients read `_version` there.
/// An explicit `SELECT _version` still resolves on either path.
///
/// `_row_id` is hidden from every star. An `INSERT` with no column list
/// targets the remaining user columns, so `INSERT INTO t VALUES (...)` and
/// `INSERT INTO t SELECT * FROM u` match PostgreSQL.
async fn hide_storage_columns_from_star(session: &SessionContext, statement: &mut Statement) {
    let postgres_star = session
        .state()
        .config()
        .options()
        .extensions
        .get::<kalamdb_session_datafusion::PostgresStar>()
        .is_some();
    let inserting = matches!(statement, Statement::Insert(_));
    if postgres_star || inserting {
        expand_stars(session, statement, false).await;
    } else {
        expand_stars(session, statement, true).await;
    }
    if inserting {
        assign_visible_insert_columns(session, statement).await;
    }
}

async fn expand_stars(session: &SessionContext, statement: &mut Statement, row_id_only: bool) {
    let tables = kalamdb_sql::storage_star_table_names(statement);
    if tables.is_empty() {
        return;
    }
    let mut columns_by_table = HashMap::with_capacity(tables.len());
    for table in tables {
        let Some(key) = kalamdb_sql::object_name_to_string(&table) else {
            continue;
        };
        let Some(reference) = table_reference_from_object_name(&table) else {
            continue;
        };
        let Ok(provider) = session.table_provider(reference).await else {
            continue;
        };
        let names = provider
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .collect();
        columns_by_table.insert(key, names);
    }
    if row_id_only {
        kalamdb_sql::expand_stars_without_row_id(statement, &columns_by_table);
    } else {
        kalamdb_sql::expand_stars_without_storage_columns(statement, &columns_by_table);
    }
}

async fn assign_visible_insert_columns(session: &SessionContext, statement: &mut Statement) {
    let table_name = match statement {
        Statement::Insert(insert) if insert.columns.is_empty() => match &insert.table {
            TableObject::TableName(name) => name.clone(),
            _ => return,
        },
        _ => return,
    };
    let Some(reference) = table_reference_from_object_name(&table_name) else {
        return;
    };
    let Ok(provider) = session.table_provider(reference).await else {
        return;
    };
    let field_count = provider.schema().fields().len();
    let visible: Vec<ObjectName> = provider
        .schema()
        .fields()
        .iter()
        .filter(|field| !SystemColumnNames::is_system_column(field.name()))
        .map(|field| ObjectName::from(vec![Ident::new(field.name())]))
        .collect();
    if visible.is_empty() || visible.len() == field_count {
        return;
    }
    if let Statement::Insert(insert) = statement {
        insert.columns = visible;
    }
}

fn table_reference_from_object_name(name: &ObjectName) -> Option<TableReference> {
    let parts: Vec<&str> = name
        .0
        .iter()
        .filter_map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(ident.value.as_str()),
            ObjectNamePart::Function(_) => None,
        })
        .collect();
    match *parts.as_slice() {
        [table] => Some(TableReference::bare(table.to_string())),
        [schema, table] => Some(TableReference::partial(schema.to_string(), table.to_string())),
        [catalog, schema, table] => {
            Some(TableReference::full(catalog.to_string(), schema.to_string(), table.to_string()))
        },
        _ => None,
    }
}

fn table_ref_is_client_catalog(table: &TableReference) -> bool {
    table.schema().is_some_and(kalamdb_commons::SchemaId::is_client_catalog_name)
}

impl SqlExecutor {
    /// Parse `sql` once. Planning and retries reuse the statement.
    pub(super) fn parse_planned_statement(sql: &str) -> Result<Statement, KalamDbError> {
        kalamdb_sql::parser::KalamParser::parse_one(sql)
            .map_err(|error| KalamDbError::InvalidSql(error.to_string()))
    }

    /// Parse `sql` once for DataFusion planning (including text-compat and AST operator rewrites)
    /// without an intermediate AST -> String -> AST round-trip.
    pub(super) fn parse_statement_for_datafusion_exec(
        sql: &str,
    ) -> Result<Statement, KalamDbError> {
        kalamdb_sql::parse_statement_for_datafusion(sql)
            .map_err(|error| KalamDbError::InvalidSql(error.to_string()))
    }

    /// Reuse a prepared DataFusion statement when it belongs to this SQL text.
    pub(super) fn statement_for_datafusion(
        sql: &str,
        metadata: Option<&PreparedExecutionStatement>,
    ) -> Result<Statement, KalamDbError> {
        match metadata
            .filter(|metadata| metadata.sql.as_str() == sql)
            .and_then(|metadata| metadata.datafusion_statement.as_deref())
        {
            Some(statement) => Ok(statement.clone()),
            None => Self::parse_statement_for_datafusion_exec(sql),
        }
    }

    /// Plan an already-parsed statement and optimize it for execution.
    pub(super) async fn dataframe_from_statement(
        session: &SessionContext,
        statement: &Statement,
    ) -> Result<DataFrame, datafusion::error::DataFusionError> {
        let mut statement = statement.clone();
        hide_storage_columns_from_star(session, &mut statement).await;
        let plan = session
            .state()
            .statement_to_plan(DfStatement::Statement(Box::new(statement)))
            .await?;
        session.execute_logical_plan(plan).await
    }

    /// Plan `statement`, reloading table providers once when the catalog is stale.
    ///
    /// The returned session is the one that produced the dataframe. Callers that
    /// execute the plan should use that session so a reloaded provider is visible.
    pub(super) async fn dataframe_after_provider_reload(
        &self,
        session: SessionContext,
        statement: &Statement,
        sql: &str,
        exec_ctx: &ExecutionContext,
        kind: ProviderReload,
    ) -> Result<(SessionContext, DataFrame), KalamDbError> {
        #[cfg(feature = "traceability")]
        let plan_start = matches!(kind, ProviderReload::Dml).then(std::time::Instant::now);

        match Self::dataframe_from_statement(&session, statement).await {
            Ok(dataframe) => {
                if matches!(kind, ProviderReload::Dml) {
                    kalamdb_observability::kdb_debug!(
                        plan_ms = (plan_start.unwrap().elapsed().as_micros() as f64 / 1000.0),
                        "sql.dml_plan"
                    );
                }
                Ok((session, dataframe))
            },
            Err(error) if Self::is_table_not_found_error(&error) => {
                if matches!(kind, ProviderReload::Select) {
                    log::warn!(
                        target: "sql::plan",
                        "⚠️  Table not found during planning; reloading table providers and retrying once | sql='{}'",
                        sql
                    );
                }
                if let Err(load_error) = self.load_existing_tables().await {
                    match kind {
                        ProviderReload::Select => log::warn!(
                            target: "sql::plan",
                            "⚠️  Failed to reload table providers after missing table | sql='{}' | error='{}'",
                            sql,
                            load_error
                        ),
                        ProviderReload::Dml => log::warn!(
                            target: "sql::dml",
                            "⚠️  Failed to reload table providers after missing table in DML | sql='{}' | error='{}'",
                            sql,
                            load_error
                        ),
                    }
                }
                let retry_session = self.create_session_with_transaction_context(exec_ctx)?;
                #[cfg(feature = "traceability")]
                let retry_start = std::time::Instant::now();
                let dataframe = Self::dataframe_from_statement(&retry_session, statement)
                    .await
                    .map_err(|retry_error| self.log_sql_error(sql, exec_ctx, retry_error))?;
                if matches!(kind, ProviderReload::Dml) {
                    kalamdb_observability::kdb_debug!(
                        plan_ms = (retry_start.elapsed().as_micros() as f64 / 1000.0),
                        reloaded_providers = true,
                        "sql.dml_plan"
                    );
                }
                Ok((retry_session, dataframe))
            },
            Err(error) => Err(self.log_sql_error(sql, exec_ctx, error)),
        }
    }

    pub(super) async fn collect_dataframe(
        dataframe: DataFrame,
    ) -> Result<(Vec<RecordBatch>, SchemaRef), datafusion::error::DataFusionError> {
        let schema = Arc::new(dataframe.schema().as_arrow().clone());
        let batches = dataframe.collect().await?;
        Ok((batches, schema))
    }

    /// One unoptimized plan for prepared parameter and result metadata.
    pub async fn logical_plan(&self, sql: &str) -> Result<LogicalPlan, KalamDbError> {
        let session = self.app_context.base_session_context();
        let mut statement = Self::parse_statement_for_datafusion_exec(sql)?;
        hide_storage_columns_from_star(&session, &mut statement).await;
        session
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
        if let Some(statement) =
            metadata.datafusion_statement.as_deref().or(metadata.parsed_dml.as_deref())
        {
            let mut statement = statement.clone();
            hide_storage_columns_from_star(&session, &mut statement).await;
            return session
                .state()
                .statement_to_plan(DfStatement::Statement(Box::new(statement)))
                .await
                .map_err(Self::datafusion_to_execution_error);
        }
        let mut statement = Self::parse_statement_for_datafusion_exec(metadata.sql.as_str())?;
        hide_storage_columns_from_star(&session, &mut statement).await;
        session
            .state()
            .statement_to_plan(DfStatement::Statement(Box::new(statement)))
            .await
            .map_err(Self::datafusion_to_execution_error)
    }

    /// Plan `sql` in the caller's session so search_path matches the wire connection.
    pub async fn logical_plan_for_session(
        &self,
        sql: &str,
        exec_ctx: &ExecutionContext,
    ) -> Result<LogicalPlan, KalamDbError> {
        let session = self.create_session_with_transaction_context(exec_ctx)?;
        let mut statement = Self::parse_statement_for_datafusion_exec(sql)?;
        hide_storage_columns_from_star(&session, &mut statement).await;
        session
            .state()
            .statement_to_plan(DfStatement::Statement(Box::new(statement)))
            .await
            .map_err(Self::datafusion_to_execution_error)
    }

    /// True when the plan already encodes an explicit row bound.
    ///
    /// DataFusion often rewrites `ORDER BY ... LIMIT N` to `Sort { fetch: Some(N) }`
    /// (and may push `LIMIT` into `TableScan.fetch`) without leaving a top-level
    /// `Limit` node. Treat those fetch bounds as explicit so we do not also apply
    /// `default_query_limit`.
    pub(super) fn logical_plan_has_limit(plan: &LogicalPlan) -> bool {
        match plan {
            LogicalPlan::Limit(_) => true,
            LogicalPlan::Sort(sort) if sort.fetch.is_some() => true,
            LogicalPlan::TableScan(scan) if scan.fetch.is_some() => true,
            _ => plan.inputs().iter().any(|input| Self::logical_plan_has_limit(input)),
        }
    }

    pub(super) fn logical_plan_is_client_catalog_introspection(plan: &LogicalPlan) -> bool {
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

    pub(super) fn apply_select_limits(
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

    pub(super) async fn optimized_plan_for_cache(
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
}
