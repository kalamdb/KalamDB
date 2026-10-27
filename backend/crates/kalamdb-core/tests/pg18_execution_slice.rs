use std::sync::Arc;

use arrow::array::{Int64Array, StringArray};
use kalamdb_commons::{
    models::{SchemaId, UserId},
    Role,
};
use kalamdb_core::sql::{context::ExecutionResult, ExecutionContext, ScalarValue};
use ntest::timeout;
use crate::support::{create_cluster_app_context, create_executor, execute_ok, request_exec_ctx};

#[tokio::test]
#[timeout(30000)]
async fn returning_clauses_for_insert_update_delete_and_ctes() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let exec_ctx = request_exec_ctx(&app_ctx, "pg18-exec-returning");

    execute_ok(&executor, &exec_ctx, "CREATE SCHEMA exec_slice").await;
    execute_ok(
        &executor,
        &exec_ctx,
        "CREATE TABLE exec_slice.tasks (id BIGINT PRIMARY KEY, title TEXT NOT NULL, points INT \
         NOT NULL DEFAULT 10) WITH (TYPE = 'SHARED')",
    )
    .await;

    // 1. INSERT ... RETURNING
    let ins_res = execute_ok(
        &executor,
        &exec_ctx,
        "INSERT INTO exec_slice.tasks (id, title, points) VALUES (1, 'alpha', 5), (2, 'beta', 15) \
         RETURNING id, title, points",
    )
    .await;
    match ins_res {
        ExecutionResult::Rows {
            batches, row_count, ..
        } => {
            assert_eq!(row_count, 2);
            let ids = batches[0].column(0).as_any().downcast_ref::<Int64Array>().unwrap();
            assert_eq!(ids.value(0), 1);
            assert_eq!(ids.value(1), 2);
        },
        other => panic!("expected RETURNING rows from INSERT, got {other:?}"),
    }

    // 2. UPDATE ... RETURNING
    let upd_res = execute_ok(
        &executor,
        &exec_ctx,
        "UPDATE exec_slice.tasks SET title = 'alpha-v2', points = points + 100 WHERE id = 1 \
         RETURNING id, title, points",
    )
    .await;
    match upd_res {
        ExecutionResult::Rows {
            batches, row_count, ..
        } => {
            assert_eq!(row_count, 1);
            let titles = batches[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
            assert_eq!(titles.value(0), "alpha-v2");
        },
        other => panic!("expected RETURNING rows from UPDATE, got {other:?}"),
    }

    // 3. DELETE ... RETURNING
    let del_res = execute_ok(
        &executor,
        &exec_ctx,
        "DELETE FROM exec_slice.tasks WHERE id = 2 RETURNING id, title",
    )
    .await;
    match del_res {
        ExecutionResult::Rows {
            batches, row_count, ..
        } => {
            assert_eq!(row_count, 1);
            let ids = batches[0].column(0).as_any().downcast_ref::<Int64Array>().unwrap();
            let titles = batches[0].column(1).as_any().downcast_ref::<StringArray>().unwrap();
            assert_eq!(ids.value(0), 2);
            assert_eq!(titles.value(0), "beta");
        },
        other => panic!("expected RETURNING rows from DELETE, got {other:?}"),
    }

    // 4. Non-recursive and recursive CTEs + window functions + CASE/COALESCE/NULLIF
    let cte_res = execute_ok(
        &executor,
        &exec_ctx,
        "WITH RECURSIVE seq(n) AS (
            SELECT 1
            UNION ALL
            SELECT n + 1 FROM seq WHERE n < 4
        )
        SELECT
            n,
            SUM(n) OVER (ORDER BY n) AS running_sum,
            COALESCE(NULLIF(n, 2), 99) AS coalesced,
            CASE WHEN n % 2 = 0 THEN 'even' ELSE 'odd' END AS parity
        FROM seq
        ORDER BY n",
    )
    .await;
    match cte_res {
        ExecutionResult::Rows {
            batches, row_count, ..
        } => {
            assert_eq!(row_count, 4);
            let running = batches[0].column(1).as_any().downcast_ref::<Int64Array>().unwrap();
            assert_eq!(running.value(3), 10);
        },
        other => panic!("expected rows from recursive CTE + window query, got {other:?}"),
    }
}

#[tokio::test]
#[timeout(30000)]
async fn set_search_path_and_prepared_dml_cache_eviction_on_error() {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let executor = create_executor(Arc::clone(&app_ctx));
    let exec_ctx = ExecutionContext::with_namespace(
        UserId::new("root"),
        Role::System,
        SchemaId::new("default"),
        app_ctx.base_session_context(),
    )
    .with_request_id("pg18-search-path-req");

    execute_ok(&executor, &exec_ctx, "CREATE SCHEMA custom_sp").await;
    execute_ok(
        &executor,
        &exec_ctx,
        "CREATE TABLE custom_sp.items (id BIGINT PRIMARY KEY, label TEXT NOT NULL) WITH (TYPE = \
         'SHARED')",
    )
    .await;

    execute_ok(&executor, &exec_ctx, "SET search_path TO custom_sp").await;
    assert_eq!(exec_ctx.default_namespace().as_str(), "custom_sp");

    execute_ok(&executor, &exec_ctx, "INSERT INTO items (id, label) VALUES (10, 'from_sp')").await;
    let sel = execute_ok(&executor, &exec_ctx, "SELECT label FROM items WHERE id = 10").await;
    match sel {
        ExecutionResult::Rows {
            batches, row_count, ..
        } => {
            assert_eq!(row_count, 1);
            let labels = batches[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
            assert_eq!(labels.value(0), "from_sp");
        },
        other => {
            panic!("expected 1 row from unqualified SELECT after SET search_path, got {other:?}")
        },
    }

    // Parameterized DML cache eviction check:
    let param_sql = "UPDATE custom_sp.items SET label = $1 WHERE id = $2";
    let meta = executor
        .prepare_statement_metadata(param_sql, &exec_ctx)
        .expect("prepare parameterized UPDATE");
    executor
        .execute_with_metadata(
            &meta,
            &exec_ctx,
            vec![
                ScalarValue::Utf8(Some("updated-1".to_string())),
                ScalarValue::Int64(Some(10)),
            ],
        )
        .await
        .expect("first parameterized UPDATE succeeds and populates plan cache");
    assert!(executor.plan_cache_len() >= 1);

    // Trigger an execution error (NULL into NOT NULL column) on cached plan:
    let err = executor
        .execute_with_metadata(
            &meta,
            &exec_ctx,
            vec![ScalarValue::Utf8(None), ScalarValue::Int64(Some(10))],
        )
        .await
        .expect_err("NULL into NOT NULL column must fail without retrying DML");
    assert_eq!(err.sqlstate(), "23502");
}
