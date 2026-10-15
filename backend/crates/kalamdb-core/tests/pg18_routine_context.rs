//! Integration tests for routine security context (`SECURITY INVOKER` vs `SECURITY DEFINER`),
//! volatility enforcement (`IMMUTABLE`/`STABLE` reject writes), and max call depth (16).

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

fn dba_ctx(app: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(
        UserId::new("u_pg18_ctx_dba"),
        Role::Dba,
        app.base_session_context(),
    )
}

fn user_ctx(app: &Arc<AppContext>, id: &str) -> ExecutionContext {
    ExecutionContext::new(UserId::new(id), Role::User, app.base_session_context())
}

async fn exec_as(
    executor: &SqlExecutor,
    ctx: &ExecutionContext,
    sql: &str,
) -> ExecutionResult {
    executor.execute(sql, ctx, vec![]).await.expect(sql)
}

async fn exec_err_as(
    executor: &SqlExecutor,
    ctx: &ExecutionContext,
    sql: &str,
) -> String {
    executor
        .execute(sql, ctx, vec![])
        .await
        .expect_err(&format!("expected error for `{sql}`"))
        .to_string()
}

fn scalar_string(result: &ExecutionResult) -> String {
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected rows, got {result:?}");
    };
    let col = batches[0].column(0);
    if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
        return arr.value(0).to_string();
    }
    if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        return arr.value(0).to_string();
    }
    if let Some(arr) = col.as_any().downcast_ref::<Int32Array>() {
        return arr.value(0).to_string();
    }
    panic!("unexpected column type: {:?}", col.data_type());
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn security_invoker_vs_definer_switches_effective_principal() {
    let (app, executor, _test_db) = setup_app().await;
    let dba = dba_ctx(&app);
    let caller = user_ctx(&app, "u_pg18_ctx_caller");

    exec_as(&executor, &dba, "CREATE SCHEMA r_sec").await;
    exec_as(
        &executor,
        &dba,
        "CREATE FUNCTION r_sec.who_invoker() RETURNS TEXT LANGUAGE JAVASCRIPT SECURITY INVOKER AS $$ return ctx.user.id + ':' + ctx.session.user.id; $$",
    )
    .await;
    exec_as(
        &executor,
        &dba,
        "CREATE FUNCTION r_sec.who_definer() RETURNS TEXT LANGUAGE JAVASCRIPT SECURITY DEFINER AS $$ return ctx.user.id + ':' + ctx.session.user.id; $$",
    )
    .await;
    exec_as(
        &executor,
        &dba,
        "GRANT EXECUTE ON FUNCTION r_sec.who_invoker TO PUBLIC",
    )
    .await;
    exec_as(
        &executor,
        &dba,
        "GRANT EXECUTE ON FUNCTION r_sec.who_definer TO PUBLIC",
    )
    .await;

    let inv_res = exec_as(&executor, &caller, "SELECT r_sec.who_invoker()").await;
    assert_eq!(
        scalar_string(&inv_res),
        "u_pg18_ctx_caller:u_pg18_ctx_caller"
    );

    let def_res = exec_as(&executor, &caller, "SELECT r_sec.who_definer()").await;
    assert_eq!(
        scalar_string(&def_res),
        "u_pg18_ctx_dba:u_pg18_ctx_caller"
    );
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn immutable_and_stable_routines_reject_writes_at_runtime() {
    let (app, executor, _test_db) = setup_app().await;
    let dba = dba_ctx(&app);

    exec_as(&executor, &dba, "CREATE SCHEMA r_vol").await;
    exec_as(
        &executor,
        &dba,
        "CREATE TABLE r_vol.items (id INT PRIMARY KEY, v INT) WITH (TYPE = 'SHARED')",
    )
    .await;
    exec_as(
        &executor,
        &dba,
        r#"CREATE FUNCTION r_vol.bad_write(id INT) RETURNS INT LANGUAGE JAVASCRIPT STABLE AS $$
            await ctx.db.execute("INSERT INTO r_vol.items (id, v) VALUES ($1, 1)", [input.id]);
            return 1;
        $$"#,
    )
    .await;

    let err = exec_err_as(&executor, &dba, "SELECT r_vol.bad_write(1)").await;
    assert!(
        err.to_ascii_lowercase().contains("read-only")
            || err.to_ascii_lowercase().contains("volatility")
            || err.to_ascii_lowercase().contains("stable")
            || err.to_ascii_lowercase().contains("write"),
        "expected read-only / volatility rejection, got: {err}"
    );
}

#[tokio::test]
#[ntest::timeout(30000)]
async fn recursive_routine_call_stops_at_max_call_depth_16() {
    let (app, executor, _test_db) = setup_app().await;
    let dba = dba_ctx(&app);

    exec_as(&executor, &dba, "CREATE SCHEMA r_depth").await;
    exec_as(
        &executor,
        &dba,
        r#"CREATE FUNCTION r_depth.recurse(n INT) RETURNS INT LANGUAGE JAVASCRIPT AS $$
            return await ctx.functions.r_depth.recurse({ n: input.n + 1 });
        $$"#,
    )
    .await;

    let err = exec_err_as(&executor, &dba, "SELECT r_depth.recurse(1)").await;
    assert!(
        err.to_ascii_lowercase().contains("depth")
            || err.to_ascii_lowercase().contains("16")
            || err.to_ascii_lowercase().contains("recursion"),
        "expected max call depth error, got: {err}"
    );
}
