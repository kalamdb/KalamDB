//! Integration tests for JS <-> SQL/PLPGSQL routine calls under one shared root transaction.

mod support;

use std::sync::Arc;

use datafusion::arrow::array::{Int32Array, Int64Array, StringArray};
use kalamdb_commons::{models::UserId, Role};
use kalamdb_configs::ServerConfig;
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};
use kalamdb_store::test_utils::TestDb;

async fn setup_app() -> (Arc<AppContext>, Arc<SqlExecutor>, TestDb) {
    let mut config = ServerConfig::default();
    config.postgres_wire.enabled = true;
    let (app, test_db) = support::create_cluster_app_context_with_config(config).await;
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app), false);
    let executor = Arc::new(SqlExecutor::new(Arc::clone(&app), registry));
    app.set_sql_executor(Arc::clone(&executor));
    (app, executor, test_db)
}

fn root_ctx(app: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(UserId::new("u_pg18_js_sql_calls"), Role::Dba, app.base_session_context())
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> ExecutionResult {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql)
}

fn scalar_i64(result: &ExecutionResult) -> i64 {
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected rows, got {result:?}");
    };
    let col = batches[0].column(0);
    if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        return arr.value(0);
    }
    if let Some(arr) = col.as_any().downcast_ref::<Int32Array>() {
        return i64::from(arr.value(0));
    }
    if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
        return arr.value(0).parse().expect("parse int");
    }
    panic!("unexpected column type: {:?}", col.data_type());
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn js_routine_calls_sql_and_plpgsql_routines_and_rolls_back_on_error() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA js_sql").await;
    exec(
        &app,
        &executor,
        "CREATE TABLE js_sql.ledger (id INT PRIMARY KEY, delta INT NOT NULL) WITH (TYPE = \
         'SHARED')",
    )
    .await;

    exec(
        &app,
        &executor,
        "CREATE FUNCTION js_sql.record_entry(entry_id INT, amount INT) RETURNS INT LANGUAGE SQL \
         AS $$ INSERT INTO js_sql.ledger (id, delta) VALUES (entry_id, amount); SELECT amount * 2 \
         $$",
    )
    .await;

    exec(
        &app,
        &executor,
        "CREATE FUNCTION js_sql.pl_bonus(base INT) RETURNS INT LANGUAGE PLPGSQL AS $$ BEGIN \
         RETURN base + 5; END $$",
    )
    .await;

    exec(
        &app,
        &executor,
        r#"CREATE FUNCTION js_sql.orchestrate(entry_id INT, amount INT) RETURNS INT LANGUAGE JAVASCRIPT AS $$
            const doubled = await ctx.functions.js_sql.record_entry({ entry_id: input.entry_id, amount: input.amount });
            const total = await ctx.functions.call("js_sql.pl_bonus", { base: doubled });
            return total;
        $$"#,
    )
    .await;

    let res = exec(&app, &executor, "SELECT js_sql.orchestrate(1, 20)").await;
    assert_eq!(scalar_i64(&res), 45);

    let count_after_ok = exec(&app, &executor, "SELECT COUNT(*) FROM js_sql.ledger").await;
    assert_eq!(scalar_i64(&count_after_ok), 1);

    exec(
        &app,
        &executor,
        r#"CREATE FUNCTION js_sql.orchestrate_fail(entry_id INT, amount INT) RETURNS INT LANGUAGE JAVASCRIPT AS $$
            await ctx.functions.js_sql.record_entry({ entry_id: input.entry_id, amount: input.amount });
            throw new Error("abort after nested SQL write");
        $$"#,
    )
    .await;

    let ctx = root_ctx(&app);
    let err = executor
        .execute("SELECT js_sql.orchestrate_fail(2, 50)", &ctx, vec![])
        .await
        .expect_err("expected failure from orchestrate_fail");
    assert!(
        err.to_string().contains("abort after nested SQL write"),
        "unexpected error: {err}"
    );

    // Owned root transaction rolled back the nested SQL insert
    let count_after_fail = exec(&app, &executor, "SELECT COUNT(*) FROM js_sql.ledger").await;
    assert_eq!(scalar_i64(&count_after_fail), 1);
}
