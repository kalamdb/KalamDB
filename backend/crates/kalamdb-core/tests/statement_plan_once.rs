use std::sync::Arc;

use arrow::{
    array::Int32Array,
    datatypes::{DataType, Field, Schema},
    record_batch::RecordBatch,
};
use datafusion::{
    datasource::MemTable, prelude::SessionContext, sql::parser::Statement as DfStatement,
};
use kalamdb_sql::parser::KalamParser;

#[tokio::test]
async fn select_and_parameterized_dml_replan_the_same_statement() {
    let context = SessionContext::new();
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]));
    let batch =
        RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(Int32Array::from(vec![1]))])
            .expect("batch");
    let table = MemTable::try_new(schema, vec![vec![batch]]).expect("mem table");
    context.register_table("items", Arc::new(table)).expect("register");

    let select = KalamParser::parse_one("SELECT 1").expect("select");
    let first = plan(&context, &select).await;
    let retry = plan(&context, &select).await;
    assert_eq!(first.display_indent().to_string(), retry.display_indent().to_string());

    let insert = KalamParser::parse_one("INSERT INTO items (id) VALUES ($1)").expect("insert");
    let inserted = plan(&context, &insert).await;
    let inserted_retry = plan(&context, &insert).await;
    assert_eq!(
        inserted.display_indent().to_string(),
        inserted_retry.display_indent().to_string()
    );

    let executor = include_str!("../src/sql/executor/sql_executor/mod.rs");
    assert!(
        !executor.contains("session.sql("),
        "sql executor must plan the parsed statement"
    );
}

async fn plan(
    context: &SessionContext,
    statement: &sqlparser::ast::Statement,
) -> datafusion::logical_expr::LogicalPlan {
    context
        .state()
        .statement_to_plan(DfStatement::Statement(Box::new(statement.clone())))
        .await
        .expect("statement_to_plan")
}
