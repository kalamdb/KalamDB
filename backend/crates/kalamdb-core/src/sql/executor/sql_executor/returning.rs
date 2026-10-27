//! RETURNING projection for applier writes.

use std::{collections::HashMap, sync::Arc};

use arrow::{
    array::RecordBatch,
    datatypes::{Field, Schema, SchemaRef},
};
use datafusion::{
    datasource::MemTable,
    prelude::{SessionConfig, SessionContext},
    scalar::ScalarValue,
};
use kalamdb_commons::{
    conversions::arrow_json_conversion::{arrow_value_to_scalar, json_rows_to_arrow_batch},
    models::{rows::row::Row, TableId},
    quote_sql_identifier,
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
        ExecutionContext, ExecutionResult,
    },
};

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
    pub(super) fn try_project_simple_returning_batch(
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

    pub(super) async fn build_on_conflict_returning_result(
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
        let (batches, schema) =
            Self::collect_dataframe(df).await.map_err(Self::datafusion_to_execution_error)?;
        let row_count = batches.iter().map(|batch| batch.num_rows()).sum();

        Ok(ExecutionResult::Rows {
            batches,
            row_count,
            schema: Some(schema),
        })
    }

    pub(super) async fn try_execute_update_or_delete_returning(
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

        let mut stmt = Self::statement_for_datafusion(sql, Some(metadata))?;
        self.evaluate_catalog_functions_in_statement(&mut stmt, exec_ctx, params)
            .await?;

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
            owned_exec_ctx = exec_ctx
                .clone()
                .with_request_id(format!("sql-dml-returning-{}", Uuid::now_v7()));
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

    pub(super) async fn execute_update_or_delete_returning_inner(
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
        let pre_rows =
            self.query_table_rows_for_returning(&pre_select_sql, params, exec_ctx).await?;

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
        let all_post_rows = self
            .query_table_rows_for_returning(&post_select_sql, &post_params, exec_ctx)
            .await?;
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

        self.build_on_conflict_returning_result(table_id, returning, returned_rows)
            .await
    }

    pub(super) async fn query_table_rows_for_returning(
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
}
