use crate::support;

use std::sync::Arc;

use datafusion::arrow::array::{Array, StringArray, StructArray};
use kalamdb_commons::{models::UserId, Role};
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};
use kalamdb_store::test_utils::TestDb;

async fn setup_app() -> (Arc<AppContext>, SqlExecutor, TestDb) {
    let (app, test_db) = support::create_cluster_app_context().await;
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app), false);
    let executor = SqlExecutor::new(Arc::clone(&app), registry);
    (app, executor, test_db)
}

fn root_ctx(app: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(UserId::new("u_pg18_type_decls"), Role::Dba, app.base_session_context())
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql);
}

#[tokio::test]
#[ntest::timeout(45000)]
async fn test_pg18_zero_attribute_composite_and_implicit_table_row_types() {
    let (app, executor, _dir) = setup_app().await;
    exec(&app, &executor, "CREATE SCHEMA app").await;

    // 1. Zero-attribute composite type: CREATE TYPE empty AS ()
    exec(&app, &executor, "CREATE TYPE app.empty_type AS ()").await;
    exec(
        &app,
        &executor,
        "CREATE SHARED TABLE app.empty_holders (id INT PRIMARY KEY, e app.empty_type)",
    )
    .await;
    exec(&app, &executor, "INSERT INTO app.empty_holders VALUES (1, NULL)").await;

    let ctx = root_ctx(&app);
    match executor
        .execute("SELECT id, e FROM app.empty_holders", &ctx, vec![])
        .await
        .expect("select empty_holders")
    {
        ExecutionResult::Rows { batches, .. } => {
            assert_eq!(batches[0].num_rows(), 1);
            let e_col = batches[0]
                .column(1)
                .as_any()
                .downcast_ref::<StructArray>()
                .expect("expected 0-field StructArray");
            assert_eq!(e_col.fields().len(), 0);
            assert!(e_col.is_null(0));
        },
        other => panic!("unexpected result: {other:?}"),
    }

    // 2. Implicit table row composite type without CREATE TYPE ... FROM TABLE
    exec(
        &app,
        &executor,
        "CREATE SHARED TABLE app.customers (id INT PRIMARY KEY, name TEXT NOT NULL)",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE SHARED TABLE app.orders (id INT PRIMARY KEY, customer_snapshot app.customers, \
         history app.customers[])",
    )
    .await;

    exec(
        &app,
        &executor,
        "INSERT INTO app.orders VALUES (10, ROW(1, 'Alice'), ARRAY[ROW(1, 'Alice')])",
    )
    .await;

    // Dropping app.customers while app.orders references its implicit row type must fail
    assert!(
        executor.execute("DROP TABLE app.customers", &ctx, vec![]).await.is_err(),
        "cannot drop table whose implicit row type is referenced by another table"
    );

    // ALTER TABLE on app.customers syncs the implicit row type
    exec(&app, &executor, "ALTER TABLE app.customers ADD COLUMN tier TEXT").await;
    exec(
        &app,
        &executor,
        "INSERT INTO app.orders VALUES (11, ROW(2, 'Bob', 'gold'), ARRAY[ROW(2, 'Bob', 'gold')])",
    )
    .await;

    // After dropping dependent table app.orders, app.customers can be dropped and cleans up its
    // implicit row type
    exec(&app, &executor, "DROP TABLE app.orders").await;
    exec(&app, &executor, "DROP TABLE app.customers").await;
}

#[tokio::test]
#[ntest::timeout(45000)]
async fn test_pg18_quoted_type_case_collisions_and_enum_rename_value_on_rows() {
    let (app, executor, _dir) = setup_app().await;
    exec(&app, &executor, "CREATE SCHEMA app").await;

    // 1. Quoted identifier case preservation and collision checks
    exec(&app, &executor, r#"CREATE TYPE app."MixedType" AS (label TEXT)"#).await;
    exec(&app, &executor, r#"CREATE TYPE app."mixedtype" AS (label TEXT)"#).await;

    let ctx = root_ctx(&app);
    // Unquoted MixedType folds to mixedtype, which already exists
    assert!(
        executor
            .execute("CREATE TYPE app.MixedType AS (label TEXT)", &ctx, vec![])
            .await
            .is_err(),
        "unquoted MixedType folds to mixedtype and must collide"
    );

    // Table and type/domain share the same per-schema type namespace
    assert!(
        executor
            .execute(r#"CREATE SHARED TABLE app."MixedType" (id INT PRIMARY KEY)"#, &ctx, vec![],)
            .await
            .is_err(),
        "table with same name as existing type in schema must be rejected"
    );

    // 2. ALTER TYPE ... RENAME VALUE updates live stored rows
    exec(
        &app,
        &executor,
        "CREATE TYPE app.ticket_status AS ENUM ('open', 'in_progress', 'closed')",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE SHARED TABLE app.tickets (id INT PRIMARY KEY, status app.ticket_status, history \
         app.ticket_status[])",
    )
    .await;
    exec(
        &app,
        &executor,
        "INSERT INTO app.tickets VALUES (1, 'in_progress', ARRAY['open', 'in_progress'])",
    )
    .await;

    exec(
        &app,
        &executor,
        "ALTER TYPE app.ticket_status RENAME VALUE 'in_progress' TO 'active'",
    )
    .await;

    match executor
        .execute("SELECT status FROM app.tickets WHERE id = 1", &ctx, vec![])
        .await
        .expect("select ticket status after RENAME VALUE")
    {
        ExecutionResult::Rows { batches, .. } => {
            let col = batches[0]
                .column(0)
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("expected StringArray");
            assert_eq!(col.value(0), "active");
        },
        other => panic!("unexpected result: {other:?}"),
    }
}
