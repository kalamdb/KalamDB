//! Statement classification and prepared execution metadata.

use std::sync::Arc;

use kalamdb_commons::{
    models::{SchemaId, TableId},
    Role,
};
use kalamdb_sql::classifier::{SqlStatement, SqlStatementKind, StatementClassificationError};
use sqlparser::ast::Statement;

use crate::sql::{
    executor::{PreparedExecutionStatement, SqlExecutor},
    plan_cache::PlanCacheKey,
    ExecutionContext,
};

fn parsed_datafusion_statement(sql: &str) -> Option<Arc<Statement>> {
    kalamdb_sql::parse_statement_for_datafusion(sql).ok().map(Arc::new)
}

impl SqlExecutor {
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

    pub(super) fn prepare_statement_metadata_for_role_with_options(
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
        let (table_id, parsed_dml, datafusion_statement) = if include_dml_ast {
            Self::parse_prepared_statements(sql, classified.kind(), default_namespace)?
        } else {
            let table_id =
                Self::extract_dml_table_id_only(sql, classified.kind(), default_namespace);
            (table_id, None, None)
        };
        let table_type = table_id.as_ref().and_then(|table_id| {
            self.app_context
                .schema_registry()
                .get(table_id)
                .map(|cached| cached.table_entry().table_type)
        });

        let track_slow_query = classified.is_slow_query_trackable();

        let prepared = PreparedExecutionStatement::with_statements(
            sql.to_string(),
            table_id,
            table_type,
            Some(classified),
            track_slow_query,
            parsed_dml,
            datafusion_statement,
        );
        self.prepared_statement_cache.insert(cache_key, prepared.clone());
        Ok(prepared)
    }

    pub(super) fn parse_prepared_statements(
        sql: &str,
        kind: &SqlStatementKind,
        default_namespace: &SchemaId,
    ) -> Result<
        (Option<TableId>, Option<Arc<Statement>>, Option<Arc<Statement>>),
        StatementClassificationError,
    > {
        match kind {
            SqlStatementKind::Insert(_)
            | SqlStatementKind::Update(_)
            | SqlStatementKind::Delete(_) => {
                let statement =
                    kalamdb_sql::parser::KalamParser::parse_one(sql).map_err(|error| {
                        StatementClassificationError::InvalidSql {
                            sql: sql.to_string(),
                            message: error.to_string(),
                        }
                    })?;
                let table_id =
                    kalamdb_sql::extract_dml_table_id_from_statement(&statement, default_namespace);
                let parsed_dml = Arc::new(statement);
                let datafusion_statement = if !kalamdb_sql::sql_needs_wire_catalog_text_rewrite(sql)
                {
                    if !kalamdb_sql::needs_ast_operator_rewrite(sql) {
                        Some(Arc::clone(&parsed_dml))
                    } else {
                        let mut df_stmt = (*parsed_dml).clone();
                        kalamdb_sql::apply_datafusion_ast_rewrites(&mut df_stmt);
                        Some(Arc::new(df_stmt))
                    }
                } else {
                    parsed_datafusion_statement(sql)
                };
                Ok((table_id, Some(parsed_dml), datafusion_statement))
            },
            SqlStatementKind::Select => Ok((None, None, parsed_datafusion_statement(sql))),
            _ => Ok((None, None, None)),
        }
    }

    pub(super) fn extract_dml_table_id_only(
        sql: &str,
        kind: &SqlStatementKind,
        default_namespace: &SchemaId,
    ) -> Option<TableId> {
        match kind {
            SqlStatementKind::Insert(_)
            | SqlStatementKind::Update(_)
            | SqlStatementKind::Delete(_) => {
                kalamdb_sql::extract_dml_table_id_fast(sql, default_namespace)
                    .or_else(|| kalamdb_sql::extract_dml_table_id(sql, default_namespace))
            },
            _ => None,
        }
    }
}
