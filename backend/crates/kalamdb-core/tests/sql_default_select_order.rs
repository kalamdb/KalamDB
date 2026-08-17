mod support;

use kalamdb_core::sql::{ExecutionContext, ExecutionResult, SqlExecutor};
use support::{
    execute_ok, insert_sql, observer_exec_ctx, result_rows, setup_shared_table, setup_user_table,
};

fn row_ids(result: ExecutionResult) -> Vec<String> {
    result_rows(result)
        .into_iter()
        .map(|row| {
            let value = row.get("id").unwrap_or_else(|| panic!("missing id column"));
            value
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| value.as_i64().map(|id| id.to_string()))
                .unwrap_or_else(|| value.to_string())
        })
        .collect()
}

async fn select_ids(executor: &SqlExecutor, exec_ctx: &ExecutionContext, sql: &str) -> Vec<String> {
    row_ids(execute_ok(executor, exec_ctx, sql).await)
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn select_without_order_by_returns_stable_primary_key_order() {
    let (app_ctx, executor, table_id, _test_db) =
        setup_shared_table("sql_default_order", "blogs").await;
    let observer_ctx = observer_exec_ctx(&app_ctx);

    // Insert out of primary-key order so scan/hash-map iteration cannot
    // accidentally match the required default (PK ASC) order.
    for (id, name) in [
        (7, "g"),
        (2, "b"),
        (9, "i"),
        (1, "a"),
        (5, "e"),
        (10, "j"),
        (3, "c"),
        (8, "h"),
        (4, "d"),
        (6, "f"),
    ] {
        execute_ok(&executor, &observer_ctx, &insert_sql(&table_id, id, name)).await;
    }

    let limited_sql =
        format!("SELECT * FROM {}.{} LIMIT 5", table_id.namespace_id(), table_id.table_name());
    let expected_limited = vec!["1", "2", "3", "4", "5"];
    let first = select_ids(&executor, &observer_ctx, &limited_sql).await;
    assert_eq!(
        first, expected_limited,
        "SELECT without ORDER BY must return the first rows by primary key"
    );

    for _ in 0..4 {
        assert_eq!(
            select_ids(&executor, &observer_ctx, &limited_sql).await,
            expected_limited,
            "repeated SELECT LIMIT without ORDER BY must be stable"
        );
    }

    let all_sql = format!("SELECT id FROM {}.{}", table_id.namespace_id(), table_id.table_name());
    assert_eq!(
        select_ids(&executor, &observer_ctx, &all_sql).await,
        vec!["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"],
        "unbounded SELECT without ORDER BY must still use default primary-key order"
    );

    let desc_sql = format!(
        "SELECT id FROM {}.{} ORDER BY id DESC LIMIT 3",
        table_id.namespace_id(),
        table_id.table_name()
    );
    assert_eq!(
        select_ids(&executor, &observer_ctx, &desc_sql).await,
        vec!["10", "9", "8"],
        "explicit ORDER BY must not be replaced by the default"
    );
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn user_table_select_without_order_by_is_stable_across_repeats() {
    let (app_ctx, executor, table_id, _test_db) =
        setup_user_table("sql_default_order_user", "blogs").await;
    let observer_ctx = observer_exec_ctx(&app_ctx);

    for id in [4, 1, 3, 2] {
        execute_ok(&executor, &observer_ctx, &insert_sql(&table_id, id, &format!("n{id}"))).await;
    }

    let sql =
        format!("SELECT id FROM {}.{} LIMIT 3", table_id.namespace_id(), table_id.table_name());
    let expected = vec!["1", "2", "3"];
    assert_eq!(select_ids(&executor, &observer_ctx, &sql).await, expected);
    assert_eq!(select_ids(&executor, &observer_ctx, &sql).await, expected);
}
