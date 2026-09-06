//! PostgreSQL wire/JDBC shims for DataFusion meta commands (`SET`, `SHOW`, `EXPLAIN`, `DESCRIBE`).

use std::sync::Arc;

use arrow::{
    array::{Array, RecordBatch, StringArray},
    datatypes::{DataType, Field, Schema, SchemaRef},
};
use kalamdb_sql::{rewrite_explain_for_datafusion, PostgresExplainFormat};

use crate::{
    error::KalamDbError,
    sql::{executor::SqlExecutor, ExecutionContext, ExecutionResult},
};

impl SqlExecutor {
    /// Rewrite `DESCRIBE namespace.table` / `DESC table` to query `information_schema.columns`
    /// with Kalam SQL type names instead of Arrow physical types.
    pub(super) fn rewrite_describe_shorthand(
        sql: &str,
        exec_ctx: &ExecutionContext,
    ) -> Option<String> {
        use kalamdb_sql::ddl::DescribeTableStatement;

        let stmt = DescribeTableStatement::parse_shorthand(sql).ok()?;
        if stmt.show_history {
            return None;
        }

        let namespace = stmt.namespace_id.unwrap_or_else(|| exec_ctx.default_namespace());

        Some(format!(
            "SELECT column_name, kdb_data_type AS data_type, is_nullable FROM \
             information_schema.columns WHERE table_schema = {} AND table_name = {} ORDER BY \
             ordinal_position",
            sql_string_literal(namespace.as_str()),
            sql_string_literal(stmt.table_name.as_str())
        ))
    }

    pub(super) fn is_explain_analyze(sql: &str) -> bool {
        fn matches_explain_analyze(statement: &str) -> bool {
            if let Ok(Some(rewritten)) = rewrite_explain_for_datafusion(statement) {
                return rewritten.analyze;
            }
            let mut words = statement.split_whitespace();
            matches!(
                (words.next(), words.next()),
                (Some(explain), Some(analyze))
                    if explain.eq_ignore_ascii_case("explain")
                        && (analyze.eq_ignore_ascii_case("analyze")
                            || analyze.eq_ignore_ascii_case("analyse"))
            )
        }

        if matches_explain_analyze(sql) {
            return true;
        }

        kalamdb_sql::execute_as::extract_inner_sql(sql)
            .is_some_and(|inner| matches_explain_analyze(&inner))
    }

    pub(super) fn postgres_explain_query_plan(
        format: PostgresExplainFormat,
        batches: Vec<RecordBatch>,
    ) -> Result<(Vec<RecordBatch>, SchemaRef), KalamDbError> {
        let query_plan = match format {
            PostgresExplainFormat::Json => postgres_explain_json_text(&batches),
            PostgresExplainFormat::Text => postgres_explain_text(&batches),
        };
        let schema = Arc::new(Schema::new(vec![Field::new("QUERY PLAN", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(StringArray::from(vec![query_plan]))],
        )
        .map_err(|error| KalamDbError::InvalidSql(error.to_string()))?;
        Ok((vec![batch], schema))
    }

    pub(super) fn is_postgres_client_set(sql: &str) -> bool {
        let trimmed = sql.trim().trim_end_matches(';').trim();
        if trimmed.len() < 4 || !trimmed[..3].eq_ignore_ascii_case("SET") {
            return false;
        }
        let rest = trimmed[3..].trim_start();
        !rest.to_ascii_lowercase().starts_with("datafusion.")
    }

    /// PostgreSQL `SHOW` GUC result for JDBC (`SHOW TRANSACTION ISOLATION LEVEL`).
    pub(super) fn postgres_show_result(sql: &str) -> Option<ExecutionResult> {
        let shown = crate::sql::functions::classify_postgres_show(sql)?;
        let schema =
            Arc::new(Schema::new(vec![Field::new(shown.name.as_str(), DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(StringArray::from(vec![shown.value]))],
        )
        .ok()?;
        Some(ExecutionResult::Rows {
            batches:   vec![batch],
            row_count: 1,
            schema:    Some(schema),
        })
    }
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn postgres_explain_text(batches: &[RecordBatch]) -> String {
    let mut lines = Vec::new();
    for (plan_type, plan) in explain_plan_rows(batches) {
        if !plan_type.is_empty() {
            lines.push(plan_type);
        }
        if !plan.is_empty() {
            lines.push(plan);
        }
    }
    lines.join("\n")
}

fn postgres_explain_json_text(batches: &[RecordBatch]) -> String {
    let mut last_json = None;
    for (_plan_type, plan) in explain_plan_rows(batches) {
        let trimmed = plan.trim();
        if trimmed.starts_with('[') || trimmed.starts_with('{') {
            last_json = Some(trimmed.to_string());
        }
    }
    match last_json {
        Some(json) if json.starts_with('[') => json,
        Some(json) => format!("[{json}]"),
        None => "[]".to_string(),
    }
}

fn explain_plan_rows(batches: &[RecordBatch]) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        let plan_type_idx = schema
            .fields()
            .iter()
            .position(|field| field.name().eq_ignore_ascii_case("plan_type"));
        let plan_idx = schema
            .fields()
            .iter()
            .position(|field| field.name().eq_ignore_ascii_case("plan"))
            .or_else(|| batch.num_columns().checked_sub(1));
        let Some(plan_idx) = plan_idx else {
            continue;
        };
        let plan_type_col =
            plan_type_idx.and_then(|idx| batch.column(idx).as_any().downcast_ref::<StringArray>());
        let Some(plan_col) = batch.column(plan_idx).as_any().downcast_ref::<StringArray>() else {
            continue;
        };
        for row_idx in 0..batch.num_rows() {
            let plan_type = plan_type_col
                .filter(|col| !col.is_null(row_idx))
                .map(|col| col.value(row_idx).to_string())
                .unwrap_or_default();
            let plan = if plan_col.is_null(row_idx) {
                String::new()
            } else {
                plan_col.value(row_idx).to_string()
            };
            rows.push((plan_type, plan));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_show_transaction_isolation_returns_read_committed() {
        let result = SqlExecutor::postgres_show_result("SHOW TRANSACTION ISOLATION LEVEL")
            .expect("JDBC SHOW TRANSACTION ISOLATION LEVEL");
        let ExecutionResult::Rows {
            batches,
            row_count,
            schema,
        } = result
        else {
            panic!("expected Rows, got {result:?}");
        };
        assert_eq!(row_count, 1);
        assert_eq!(schema.unwrap().field(0).name(), "transaction_isolation");
        let values = batches[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(values.value(0), "read committed");
    }

    #[test]
    fn parenthesized_analyze_is_detected() {
        assert!(SqlExecutor::is_explain_analyze(
            "EXPLAIN (FORMAT JSON, ANALYZE, BUFFERS) select * from system.users"
        ));
        assert!(!SqlExecutor::is_explain_analyze(
            "EXPLAIN (FORMAT JSON, BUFFERS) select * from system.users"
        ));
        assert!(SqlExecutor::is_explain_analyze("EXPLAIN ANALYZE SELECT 1"));
        assert!(SqlExecutor::is_explain_analyze(
            "EXPLAIN (ANALYZE, METRICS 'rows', LEVEL summary) SELECT 1"
        ));
    }

    #[test]
    fn postgres_explain_json_uses_query_plan_column() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("plan_type", DataType::Utf8, false),
            Field::new("plan", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(vec!["logical_plan", "physical_plan"])),
                Arc::new(StringArray::from(vec![
                    r#"[{ "Plan": { "Node Type": "Projection" } }]"#,
                    r#"[{ "Plan": { "Node Type": "Values" } }]"#,
                ])),
            ],
        )
        .unwrap();
        let (batches, out_schema) =
            SqlExecutor::postgres_explain_query_plan(PostgresExplainFormat::Json, vec![batch])
                .expect("reshape JSON EXPLAIN");
        assert_eq!(out_schema.field(0).name(), "QUERY PLAN");
        let values = batches[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(values.value(0), r#"[{ "Plan": { "Node Type": "Values" } }]"#);
    }
}
