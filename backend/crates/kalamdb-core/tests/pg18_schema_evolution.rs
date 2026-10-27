//! Phase 5 (`T086` / `T086A` / `T091`–`T098` / `T101`–`T102`) PostgreSQL 18 schema evolution
//! and row storage v2 integration tests.

use crate::support;

use std::sync::Arc;

use datafusion_common::ScalarValue;
use kalamdb_commons::{models::UserId, Role};
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};
use kalamdb_handlers::register_all_handlers;
use kalamdb_store::test_utils::TestDb;

async fn make_executor() -> (Arc<AppContext>, SqlExecutor, ExecutionContext, TestDb) {
    let (app_ctx, test_db) = support::create_cluster_app_context().await;
    let registry = Arc::new(HandlerRegistry::new());
    register_all_handlers(&registry, app_ctx.clone(), false);
    let executor = SqlExecutor::new(app_ctx.clone(), registry);
    let exec_ctx =
        ExecutionContext::new(UserId::new("u_evol"), Role::Dba, app_ctx.base_session_context());
    (app_ctx, executor, exec_ctx, test_db)
}

async fn exec_ok(executor: &SqlExecutor, ctx: &ExecutionContext, sql: &str) -> ExecutionResult {
    executor
        .execute(sql, ctx, Vec::new())
        .await
        .unwrap_or_else(|err| panic!("SQL failed: {sql}\nerror: {err}"))
}

async fn exec_err(executor: &SqlExecutor, ctx: &ExecutionContext, sql: &str) -> String {
    match executor.execute(sql, ctx, Vec::new()).await {
        Ok(res) => panic!("Expected SQL error for:\n{sql}\ngot: {res:?}"),
        Err(err) => err.to_string(),
    }
}

async fn query_scalars(
    executor: &SqlExecutor,
    ctx: &ExecutionContext,
    sql: &str,
) -> Vec<Vec<ScalarValue>> {
    let res = exec_ok(executor, ctx, sql).await;
    let ExecutionResult::Rows { batches, .. } = res else {
        panic!("Expected Rows for {sql}, got {res:?}");
    };
    let mut rows = Vec::new();
    for batch in &batches {
        for r in 0..batch.num_rows() {
            let mut row = Vec::with_capacity(batch.num_columns());
            for c in 0..batch.num_columns() {
                row.push(ScalarValue::try_from_array(batch.column(c), r).unwrap());
            }
            rows.push(row);
        }
    }
    rows
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn add_drop_rename_and_missing_default_coexist_across_schema_versions() {
    let (_app, executor, ctx, _test_db) = make_executor().await;
    exec_ok(&executor, &ctx, "CREATE SCHEMA IF NOT EXISTS evol").await;
    exec_ok(
        &executor,
        &ctx,
        "CREATE TABLE evol.items (
            id BIGINT PRIMARY KEY,
            qty INT NOT NULL,
            legacy_note TEXT
        ) WITH (TYPE = 'SHARED')",
    )
    .await;

    // v1 row
    exec_ok(
        &executor,
        &ctx,
        "INSERT INTO evol.items (id, qty, legacy_note) VALUES (1, 10, 'from_v1')",
    )
    .await;

    // v2: ADD COLUMN with non-volatile DEFAULT (metadata-only missing_default)
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol.items ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'",
    )
    .await;
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol.items ADD COLUMN opt_tag TEXT DEFAULT 'auto_tag'",
    )
    .await;

    // Insert v2 rows: one omitting opt_tag, one explicitly setting NULL
    exec_ok(
        &executor,
        &ctx,
        "INSERT INTO evol.items (id, qty, legacy_note) VALUES (2, 20, 'from_v2_default')",
    )
    .await;
    exec_ok(
        &executor,
        &ctx,
        "INSERT INTO evol.items (id, qty, legacy_note, status, opt_tag) VALUES (3, 30, \
         'from_v2_null', 'active', NULL)",
    )
    .await;

    // v3: DROP COLUMN legacy_note (tombstoned slot) and RENAME COLUMN qty TO quantity
    exec_ok(&executor, &ctx, "ALTER TABLE evol.items DROP COLUMN legacy_note").await;
    exec_ok(&executor, &ctx, "ALTER TABLE evol.items RENAME COLUMN qty TO quantity").await;

    // v4: ADD COLUMN after a dropped slot to prove column_id slot stability
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol.items ADD COLUMN bonus INT NOT NULL DEFAULT 5",
    )
    .await;
    exec_ok(
        &executor,
        &ctx,
        "INSERT INTO evol.items (id, quantity, status, opt_tag, bonus) VALUES (4, 40, 'shipped', \
         'manual', 99)",
    )
    .await;

    let rows = query_scalars(
        &executor,
        &ctx,
        "SELECT id, quantity, status, opt_tag, bonus FROM evol.items ORDER BY id",
    )
    .await;
    assert_eq!(rows.len(), 4);

    // Row 1 (written at v1 before status/opt_tag/bonus existed): receives missing_default!
    assert_eq!(rows[0][0], ScalarValue::Int64(Some(1)));
    assert_eq!(rows[0][1], ScalarValue::Int32(Some(10)));
    assert_eq!(rows[0][2], ScalarValue::Utf8(Some("pending".to_string())));
    assert_eq!(rows[0][3], ScalarValue::Utf8(Some("auto_tag".to_string())));
    assert_eq!(rows[0][4], ScalarValue::Int32(Some(5)));

    // Row 2 (written at v2 with defaults, before bonus existed)
    assert_eq!(rows[1][0], ScalarValue::Int64(Some(2)));
    assert_eq!(rows[1][1], ScalarValue::Int32(Some(20)));
    assert_eq!(rows[1][2], ScalarValue::Utf8(Some("pending".to_string())));
    assert_eq!(rows[1][3], ScalarValue::Utf8(Some("auto_tag".to_string())));
    assert_eq!(rows[1][4], ScalarValue::Int32(Some(5)));

    // Row 3 (written at v2 with explicit NULL in opt_tag): keeps NULL in opt_tag, gets 5 for bonus!
    assert_eq!(rows[2][0], ScalarValue::Int64(Some(3)));
    assert_eq!(rows[2][1], ScalarValue::Int32(Some(30)));
    assert_eq!(rows[2][2], ScalarValue::Utf8(Some("active".to_string())));
    assert!(
        rows[2][3].is_null(),
        "explicitly written NULL must stay NULL, got {:?}",
        rows[2][3]
    );
    assert_eq!(rows[2][4], ScalarValue::Int32(Some(5)));

    // Row 4 (written at v4)
    assert_eq!(rows[3][0], ScalarValue::Int64(Some(4)));
    assert_eq!(rows[3][1], ScalarValue::Int32(Some(40)));
    assert_eq!(rows[3][2], ScalarValue::Utf8(Some("shipped".to_string())));
    assert_eq!(rows[3][3], ScalarValue::Utf8(Some("manual".to_string())));
    assert_eq!(rows[3][4], ScalarValue::Int32(Some(99)));
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn alter_column_type_widening_and_using_rewrite_with_domain_and_enum_validation() {
    let (_app, executor, ctx, _test_db) = make_executor().await;
    exec_ok(&executor, &ctx, "CREATE SCHEMA IF NOT EXISTS evol_type").await;
    exec_ok(
        &executor,
        &ctx,
        "CREATE DOMAIN evol_type.positive_bigint AS BIGINT CHECK (VALUE > 0)",
    )
    .await;
    exec_ok(
        &executor,
        &ctx,
        "CREATE TYPE evol_type.tier AS ENUM ('free', 'pro', 'enterprise')",
    )
    .await;

    exec_ok(
        &executor,
        &ctx,
        "CREATE TABLE evol_type.metrics (
            id BIGINT PRIMARY KEY,
            small_cnt SMALLINT NOT NULL,
            price DECIMAL(6, 2) NOT NULL,
            raw_score TEXT NOT NULL,
            plan_name TEXT NOT NULL
        ) WITH (TYPE = 'SHARED')",
    )
    .await;

    exec_ok(
        &executor,
        &ctx,
        "INSERT INTO evol_type.metrics (id, price, small_cnt, raw_score, plan_name)
         VALUES (1, 12.50, 7, '100', 'pro'), (2, 99.95, 42, '250', 'enterprise')",
    )
    .await;

    // 1. Metadata-only widening: SMALLINT -> INT -> BIGINT, DECIMAL(6,2) -> DECIMAL(12,2)
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol_type.metrics ALTER COLUMN small_cnt SET DATA TYPE INT",
    )
    .await;
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol_type.metrics ALTER COLUMN small_cnt TYPE BIGINT",
    )
    .await;
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol_type.metrics ALTER COLUMN price SET DATA TYPE DECIMAL(12, 2)",
    )
    .await;

    // 2. Rewrite with USING expression: raw_score TEXT -> evol_type.positive_bigint USING
    //    (raw_score::bigint + 10)
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol_type.metrics ALTER COLUMN raw_score TYPE evol_type.positive_bigint \
         USING (raw_score::bigint + 10)",
    )
    .await;

    // 3. Rewrite TEXT -> ENUM evol_type.tier
    exec_ok(
        &executor,
        &ctx,
        "ALTER TABLE evol_type.metrics ALTER COLUMN plan_name SET DATA TYPE evol_type.tier",
    )
    .await;

    let rows = query_scalars(
        &executor,
        &ctx,
        "SELECT id, small_cnt, price, raw_score, plan_name FROM evol_type.metrics ORDER BY id",
    )
    .await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1], ScalarValue::Int64(Some(7)));
    assert_eq!(rows[0][2], ScalarValue::Decimal128(Some(1250), 12, 2));
    assert_eq!(rows[0][3], ScalarValue::Int64(Some(110)));
    assert_eq!(rows[0][4], ScalarValue::Utf8(Some("pro".to_string())));

    assert_eq!(rows[1][1], ScalarValue::Int64(Some(42)));
    assert_eq!(rows[1][2], ScalarValue::Decimal128(Some(9995), 12, 2));
    assert_eq!(rows[1][3], ScalarValue::Int64(Some(260)));
    assert_eq!(rows[1][4], ScalarValue::Utf8(Some("enterprise".to_string())));

    // 4. Reject invalid narrowing or domain check failure without corrupting table
    let err = exec_err(
        &executor,
        &ctx,
        "ALTER TABLE evol_type.metrics ALTER COLUMN raw_score TYPE evol_type.positive_bigint \
         USING (raw_score - 500)",
    )
    .await;
    assert!(
        err.to_lowercase().contains("domain") || err.to_lowercase().contains("check"),
        "unexpected error: {err}"
    );

    // Table still reads clean values after rejected ALTER COLUMN TYPE
    let after_err =
        query_scalars(&executor, &ctx, "SELECT id, raw_score FROM evol_type.metrics ORDER BY id")
            .await;
    assert_eq!(after_err[0][1], ScalarValue::Int64(Some(110)));
    assert_eq!(after_err[1][1], ScalarValue::Int64(Some(260)));
}

#[tokio::test]
#[ntest::timeout(20000)]
async fn add_not_null_column_without_default_rejected_on_populated_table() {
    let (_app, executor, ctx, _test_db) = make_executor().await;
    exec_ok(&executor, &ctx, "CREATE SCHEMA IF NOT EXISTS evol_nn").await;
    exec_ok(
        &executor,
        &ctx,
        "CREATE TABLE evol_nn.t (id BIGINT PRIMARY KEY, name TEXT) WITH (TYPE = 'SHARED')",
    )
    .await;
    exec_ok(&executor, &ctx, "INSERT INTO evol_nn.t (id, name) VALUES (1, 'alice')").await;

    let err =
        exec_err(&executor, &ctx, "ALTER TABLE evol_nn.t ADD COLUMN required_col INT NOT NULL")
            .await;
    assert!(err.to_lowercase().contains("null"), "expected null violation error, got: {err}");
}
