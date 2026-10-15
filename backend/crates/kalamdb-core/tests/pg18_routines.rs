//! End-to-end PostgreSQL 18 routine lifecycle integration tests (US11).

mod support;

use std::sync::Arc;

use datafusion::arrow::array::{Int32Array, Int64Array, StringArray, StringViewArray};
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
    ExecutionContext::new(UserId::new("u_pg18_routines_e2e"), Role::Dba, app.base_session_context())
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) -> ExecutionResult {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql)
}

fn scalar_string(result: &ExecutionResult) -> String {
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected rows, got {result:?}");
    };
    let col = batches[0].column(0);
    if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
        return arr.value(0).to_string();
    }
    if let Some(arr) = col.as_any().downcast_ref::<StringViewArray>() {
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
async fn end_to_end_routine_lifecycle_sql_plpgsql_and_javascript() {
    let (app, executor, _test_db) = setup_app().await;

    exec(&app, &executor, "CREATE SCHEMA app_r").await;
    exec(
        &app,
        &executor,
        "CREATE TABLE app_r.events (id INT PRIMARY KEY, label TEXT NOT NULL) WITH (TYPE = \
         'SHARED')",
    )
    .await;

    // 1. SQL routine with default expression
    exec(
        &app,
        &executor,
        "CREATE FUNCTION app_r.format_label(prefix TEXT DEFAULT 'ev', num INT DEFAULT 1) RETURNS \
         TEXT LANGUAGE SQL IMMUTABLE AS $$ SELECT prefix || '-' || CAST(num AS TEXT) $$",
    )
    .await;
    let formatted =
        exec(&app, &executor, "SELECT app_r.format_label(num => 42, prefix := 'task')").await;
    assert_eq!(scalar_string(&formatted), "task-42");
    let preview_defaults =
        exec(&app, &executor, r#"SELECT * FROM "app_r"."format_label"() LIMIT 501 OFFSET 0"#).await;
    assert_eq!(scalar_string(&preview_defaults), "ev-1");

    // 2. PL/pgSQL routine with IF / LOOP / RETURN
    exec(
        &app,
        &executor,
        r#"CREATE FUNCTION app_r.sum_to(limit_n INT) RETURNS INT LANGUAGE PLPGSQL IMMUTABLE AS $$
        DECLARE
            acc INT := 0;
            i INT := 1;
        BEGIN
            WHILE i <= limit_n LOOP
                acc := acc + i;
                i := i + 1;
            END LOOP;
            RETURN acc;
        END
        $$"#,
    )
    .await;
    let summed = exec(&app, &executor, "SELECT app_r.sum_to(5)").await;
    assert_eq!(scalar_string(&summed), "15");

    let preview =
        exec(&app, &executor, r#"SELECT * FROM "app_r"."sum_to"(5) LIMIT 501 OFFSET 0"#).await;
    assert_eq!(scalar_string(&preview), "15");
    let ExecutionResult::Rows { schema, .. } = &preview else {
        panic!("expected rows");
    };
    assert_eq!(schema.as_ref().expect("schema").field(0).name(), "sum_to");
    let skipped =
        exec(&app, &executor, r#"SELECT * FROM "app_r"."sum_to"(5) LIMIT 501 OFFSET 1"#).await;
    let ExecutionResult::Rows { row_count, .. } = &skipped else {
        panic!("expected rows");
    };
    assert_eq!(*row_count, 0);

    // 3. JavaScript routine invoking SQL routine and writing row
    exec(
        &app,
        &executor,
        r#"CREATE PROCEDURE app_r.emit_event(event_id INT, prefix TEXT DEFAULT 'evt') LANGUAGE JAVASCRIPT AS $$
            const label = await ctx.functions.app_r.format_label({ prefix: input.prefix, num: input.event_id });
            await ctx.db.execute("INSERT INTO app_r.events (id, label) VALUES ($1, $2)", [input.event_id, label]);
        $$"#,
    )
    .await;
    exec(&app, &executor, "CALL app_r.emit_event(7, prefix => 'job')").await;

    let row_label = exec(&app, &executor, "SELECT label FROM app_r.events WHERE id = 7").await;
    assert_eq!(scalar_string(&row_label), "job-7");

    // 4. COMMENT, ALTER, RENAME, DROP
    exec(
        &app,
        &executor,
        "COMMENT ON FUNCTION app_r.sum_to(INT) IS 'Triangular number helper'",
    )
    .await;
    exec(&app, &executor, "ALTER FUNCTION app_r.sum_to(INT) RENAME TO triangular_sum").await;
    let renamed_res = exec(&app, &executor, "SELECT app_r.triangular_sum(6)").await;
    assert_eq!(scalar_string(&renamed_res), "21");

    exec(&app, &executor, "DROP FUNCTION app_r.triangular_sum(INT)").await;
}
