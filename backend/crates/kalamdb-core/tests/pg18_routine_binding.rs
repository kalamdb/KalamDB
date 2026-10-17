//! Integration tests for PostgreSQL 18 routine argument binding, defaults, OUT/INOUT, and strict
//! NULL.

mod support;

use std::sync::Arc;

use datafusion::arrow::array::{Array, Int32Array, Int64Array, StringArray};
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
    ExecutionContext::new(
        UserId::new("u_pg18_routine_binding"),
        Role::Dba,
        app.base_session_context(),
    )
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> ExecutionResult {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql)
}

async fn exec_err(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> String {
    let ctx = root_ctx(app);
    executor
        .execute(sql, &ctx, vec![])
        .await
        .expect_err(&format!("expected error for `{sql}`"))
        .to_string()
}

fn first_i64_or_i32(result: &ExecutionResult) -> Option<i64> {
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected rows, got {result:?}");
    };
    let col = batches[0].column(0);
    if col.is_null(0) {
        return None;
    }
    if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        return Some(arr.value(0));
    }
    if let Some(arr) = col.as_any().downcast_ref::<Int32Array>() {
        return Some(i64::from(arr.value(0)));
    }
    if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
        return arr.value(0).parse().ok();
    }
    panic!("unexpected column type: {:?}", col.data_type());
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn positional_named_and_default_argument_binding() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA r_bind").await;
    exec(
        &app,
        &executor,
        "CREATE FUNCTION r_bind.weighted_sum(a INT, b INT DEFAULT 10, c INT DEFAULT 100) RETURNS \
         INT LANGUAGE SQL AS $$ SELECT a + b * 2 + c * 3 $$",
    )
    .await;

    // Positional with omitted defaults: a=1, b=10, c=100 => 1 + 20 + 300 = 321
    let r1 = exec(&app, &executor, "SELECT r_bind.weighted_sum(1)").await;
    assert_eq!(first_i64_or_i32(&r1), Some(321));

    // Named with => and := in arbitrary order: c=2, a=5 (b defaults to 10) => 5 + 20 + 6 = 31
    let r2 = exec(&app, &executor, "SELECT r_bind.weighted_sum(c => 2, a := 5)").await;
    assert_eq!(first_i64_or_i32(&r2), Some(31));

    // Mixed positional then named: a=3, c=4 (b defaults to 10) => 3 + 20 + 12 = 35
    let r3 = exec(&app, &executor, "SELECT r_bind.weighted_sum(3, c => 4)").await;
    assert_eq!(first_i64_or_i32(&r3), Some(35));

    // CALL on a FUNCTION is rejected
    let call_fn = exec_err(&app, &executor, "CALL r_bind.weighted_sum(1)").await;
    assert!(
        call_fn.to_ascii_lowercase().contains("is a function, not a procedure"),
        "{call_fn}"
    );

    // Positional after named is rejected at parse/bind time
    let pos_after_named = exec_err(&app, &executor, "SELECT r_bind.weighted_sum(a => 1, 2)").await;
    assert!(
        pos_after_named.to_ascii_lowercase().contains("positional")
            || pos_after_named.to_ascii_lowercase().contains("named"),
        "{pos_after_named}"
    );
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn strict_null_short_circuits_and_called_on_null_executes() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA r_null").await;
    exec(
        &app,
        &executor,
        "CREATE FUNCTION r_null.strict_double(v INT) RETURNS INT LANGUAGE SQL STRICT AS $$ SELECT \
         COALESCE(v, 99) * 2 $$",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE FUNCTION r_null.coalesce_double(v INT) RETURNS INT LANGUAGE SQL CALLED ON NULL \
         INPUT AS $$ SELECT COALESCE(v, 99) * 2 $$",
    )
    .await;

    let strict_res = exec(&app, &executor, "SELECT r_null.strict_double(NULL)").await;
    assert_eq!(first_i64_or_i32(&strict_res), None);

    let called_res = exec(&app, &executor, "SELECT r_null.coalesce_double(NULL)").await;
    assert_eq!(first_i64_or_i32(&called_res), Some(198));
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn out_and_inout_parameters_materialize_return_columns() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA r_out").await;
    exec(
        &app,
        &executor,
        "CREATE PROCEDURE r_out.bump(INOUT x INT, OUT doubled INT) LANGUAGE PLPGSQL AS $$ BEGIN \
         doubled := x * 2; x := x + 1; END $$",
    )
    .await;

    let res = exec(&app, &executor, "CALL r_out.bump(7)").await;
    let ExecutionResult::Rows { batches, .. } = res else {
        panic!("expected rows from procedure with INOUT/OUT params");
    };
    assert_eq!(batches[0].num_columns(), 2);
    assert_eq!(batches[0].schema().field(0).name(), "x");
    assert_eq!(batches[0].schema().field(1).name(), "doubled");
}
