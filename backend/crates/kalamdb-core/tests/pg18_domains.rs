mod support;

use std::sync::Arc;

use datafusion::arrow::array::{Array, Int32Array};
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
    ExecutionContext::new(
        UserId::new("u_pg18_domains"),
        Role::Dba,
        app.base_session_context(),
    )
}

async fn exec(app: &Arc<AppContext>, executor: &SqlExecutor, sql: &str) {
    let ctx = root_ctx(app);
    executor.execute(sql, &ctx, vec![]).await.expect(sql);
}

async fn query_i32_col(
    app: &Arc<AppContext>,
    executor: &SqlExecutor,
    sql: &str,
) -> Vec<Option<i32>> {
    let ctx = root_ctx(app);
    match executor.execute(sql, &ctx, vec![]).await.expect(sql) {
        ExecutionResult::Rows { batches, .. } => {
            let mut out = Vec::new();
            for batch in batches {
                let col = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .expect("expected Int32Array");
                for i in 0..col.len() {
                    out.push((!col.is_null(i)).then(|| col.value(i)));
                }
            }
            out
        },
        other => panic!("expected rows for `{sql}`, got {other:?}"),
    }
}

#[tokio::test]
#[ntest::timeout(45000)]
async fn test_pg18_domains_three_valued_check_stacking_and_ddl() {
    let (app, executor, _dir) = setup_app().await;
    exec(&app, &executor, "CREATE SCHEMA app").await;

    // 1. Domain with three-valued CHECK (NULL passes CHECK when NOT NULL is absent)
    exec(
        &app,
        &executor,
        "CREATE DOMAIN app.even_int AS INT CHECK (VALUE % 2 = 0)",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE SHARED TABLE app.nums (id INT PRIMARY KEY, v app.even_int)",
    )
    .await;

    exec(&app, &executor, "INSERT INTO app.nums VALUES (1, 4)").await;
    exec(&app, &executor, "INSERT INTO app.nums VALUES (2, NULL)").await;

    let ctx = root_ctx(&app);
    let odd_err = executor
        .execute("INSERT INTO app.nums VALUES (3, 5)", &ctx, vec![])
        .await;
    assert!(odd_err.is_err(), "odd value must fail domain CHECK");

    let vals = query_i32_col(&app, &executor, "SELECT v FROM app.nums ORDER BY id").await;
    assert_eq!(vals, vec![Some(4), None]);

    // 2. Stacked domain over domain with DEFAULT and NOT NULL
    exec(
        &app,
        &executor,
        "CREATE DOMAIN app.pos_even_int AS app.even_int DEFAULT 2 NOT NULL CONSTRAINT pos_chk CHECK (VALUE > 0)",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE SHARED TABLE app.pos_nums (id INT PRIMARY KEY, v app.pos_even_int)",
    )
    .await;

    // Default materialization when column omitted
    exec(&app, &executor, "INSERT INTO app.pos_nums (id) VALUES (10)").await;

    let pos_vals = query_i32_col(&app, &executor, "SELECT v FROM app.pos_nums ORDER BY id").await;
    assert_eq!(pos_vals, vec![Some(2)]);

    // Stacked constraints enforce both parent (even) and child (> 0 and NOT NULL)
    assert!(
        executor
            .execute("INSERT INTO app.pos_nums VALUES (11, -2)", &ctx, vec![])
            .await
            .is_err(),
        "negative even number must fail child domain pos_chk"
    );
    assert!(
        executor
            .execute("INSERT INTO app.pos_nums VALUES (12, 3)", &ctx, vec![])
            .await
            .is_err(),
        "positive odd number must fail parent domain even_int check"
    );
    assert!(
        executor
            .execute("INSERT INTO app.pos_nums VALUES (13, NULL)", &ctx, vec![])
            .await
            .is_err(),
        "NULL must fail domain NOT NULL"
    );

    // 3. ALTER DOMAIN SET NOT NULL fails while NULL exists in app.nums, succeeds after cleanup
    assert!(
        executor
            .execute("ALTER DOMAIN app.even_int SET NOT NULL", &ctx, vec![])
            .await
            .is_err(),
        "SET NOT NULL must fail when existing table rows contain NULL"
    );
    exec(&app, &executor, "DELETE FROM app.nums WHERE id = 2").await;
    exec(&app, &executor, "ALTER DOMAIN app.even_int SET NOT NULL").await;

    // 4. ALTER DOMAIN ADD CONSTRAINT ... NOT VALID and VALIDATE CONSTRAINT
    exec(&app, &executor, "INSERT INTO app.nums VALUES (4, 100)").await;
    assert!(
        executor
            .execute(
                "ALTER DOMAIN app.even_int ADD CONSTRAINT small_chk CHECK (VALUE < 50)",
                &ctx,
                vec![],
            )
            .await
            .is_err(),
        "validated ADD CONSTRAINT must reject existing row v=100"
    );
    exec(
        &app,
        &executor,
        "ALTER DOMAIN app.even_int ADD CONSTRAINT small_chk CHECK (VALUE < 50) NOT VALID",
    )
    .await;
    // New writes are checked even for NOT VALID constraint
    assert!(
        executor
            .execute("INSERT INTO app.nums VALUES (5, 60)", &ctx, vec![])
            .await
            .is_err(),
        "NOT VALID domain constraint must still validate new writes"
    );
    // VALIDATE CONSTRAINT fails until row 4 is updated/deleted
    assert!(
        executor
            .execute(
                "ALTER DOMAIN app.even_int VALIDATE CONSTRAINT small_chk",
                &ctx,
                vec![],
            )
            .await
            .is_err(),
        "VALIDATE CONSTRAINT must fail while row v=100 exists"
    );
    exec(&app, &executor, "DELETE FROM app.nums WHERE id = 4").await;
    exec(
        &app,
        &executor,
        "ALTER DOMAIN app.even_int VALIDATE CONSTRAINT small_chk",
    )
    .await;

    // 5. RENAME CONSTRAINT, DROP CONSTRAINT, COMMENT ON DOMAIN
    exec(
        &app,
        &executor,
        "ALTER DOMAIN app.even_int RENAME CONSTRAINT small_chk TO under_fifty",
    )
    .await;
    exec(
        &app,
        &executor,
        "ALTER DOMAIN app.even_int DROP CONSTRAINT under_fifty",
    )
    .await;
    exec(&app, &executor, "INSERT INTO app.nums VALUES (6, 80)").await;
    exec(
        &app,
        &executor,
        "COMMENT ON DOMAIN app.even_int IS 'even numbers domain'",
    )
    .await;
}

#[tokio::test]
#[ntest::timeout(45000)]
async fn test_pg18_domain_arrays_and_composites() {
    let (app, executor, _dir) = setup_app().await;
    exec(&app, &executor, "CREATE SCHEMA app").await;

    exec(
        &app,
        &executor,
        "CREATE DOMAIN app.nonempty_code AS TEXT NOT NULL CHECK (VALUE <> '')",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE TYPE app.tagged_item AS (code app.nonempty_code, tags app.nonempty_code[])",
    )
    .await;
    exec(
        &app,
        &executor,
        "CREATE SHARED TABLE app.catalog_items (id INT PRIMARY KEY, codes app.nonempty_code[], item app.tagged_item)",
    )
    .await;

    exec(
        &app,
        &executor,
        "INSERT INTO app.catalog_items VALUES (1, ARRAY['A1', 'B2'], ROW('X9', ARRAY['T1']))",
    )
    .await;

    let ctx = root_ctx(&app);
    assert!(
        executor
            .execute(
                "INSERT INTO app.catalog_items VALUES (2, ARRAY['A1', ''], ROW('X9', ARRAY['T1']))",
                &ctx,
                vec![],
            )
            .await
            .is_err(),
        "empty string in domain[] must fail domain CHECK"
    );
    assert!(
        executor
            .execute(
                "INSERT INTO app.catalog_items VALUES (3, ARRAY['A1'], ROW('', ARRAY['T1']))",
                &ctx,
                vec![],
            )
            .await
            .is_err(),
        "empty string in composite field using domain must fail domain CHECK"
    );

    let rows = query_i32_col(
        &app,
        &executor,
        "SELECT id FROM app.catalog_items WHERE id = 1",
    )
    .await;
    assert_eq!(rows, vec![Some(1)]);
}
