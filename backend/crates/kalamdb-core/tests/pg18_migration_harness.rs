use std::sync::Arc;

use arrow::{
    array::{Array, Int32Array, Int64Array, StringArray},
    util::display::ArrayFormatter,
};
use datafusion::catalog::SchemaProvider;
use kalamdb_core::sql::context::ExecutionResult;
use kalamdb_sql::split_statements;
use kalamdb_views::pg_catalog::PgCatalogSchemaProvider;
use ntest::timeout;
use crate::support::{create_cluster_app_context, create_executor, execute_ok, request_exec_ctx};

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrderRowSnapshot {
    id: i64,
    customer: String,
    qty: i32,
    amount: String,
    state: String,
    metadata: String,
}

fn array_value_to_string(array: &dyn Array, row: usize) -> String {
    let formatter = ArrayFormatter::try_new(array, &Default::default()).expect("array formatter");
    formatter.value(row).to_string()
}

async fn snapshot_orders(
    executor: &kalamdb_core::sql::executor::SqlExecutor,
    exec_ctx: &kalamdb_core::sql::ExecutionContext,
) -> Vec<OrderRowSnapshot> {
    let res = execute_ok(
        executor,
        exec_ctx,
        "SELECT id, customer, qty, CAST(amount AS TEXT) AS amount_txt, CAST(state AS TEXT) AS \
         state_txt, CAST(metadata AS TEXT) AS meta_txt FROM mig_app.orders ORDER BY id",
    )
    .await;
    let ExecutionResult::Rows { batches, .. } = res else {
        panic!("expected rows from mig_app.orders");
    };
    let mut out = Vec::new();
    for batch in batches {
        let ids = batch.column(0).as_any().downcast_ref::<Int64Array>().unwrap();
        let qtys = batch.column(2).as_any().downcast_ref::<Int32Array>().unwrap();
        for row in 0..batch.num_rows() {
            out.push(OrderRowSnapshot {
                id: ids.value(row),
                customer: array_value_to_string(batch.column(1).as_ref(), row),
                qty: qtys.value(row),
                amount: array_value_to_string(batch.column(3).as_ref(), row),
                state: array_value_to_string(batch.column(4).as_ref(), row),
                metadata: array_value_to_string(batch.column(5).as_ref(), row),
            });
        }
    }
    out
}

#[tokio::test]
#[timeout(30000)]
async fn migration_fixture_preserves_typed_contents_catalog_metadata_and_catches_equal_count_corruption(
) {
    let (app_ctx, _test_db) = create_cluster_app_context().await;
    let pg_catalog_schema =
        Arc::new(PgCatalogSchemaProvider::new(app_ctx.system_tables(), Arc::new(Vec::new)));
    app_ctx
        .base_session_context()
        .catalog("kalam")
        .expect("kalam catalog")
        .register_schema("pg_catalog", pg_catalog_schema as Arc<dyn SchemaProvider>)
        .expect("register pg_catalog");
    let executor = create_executor(Arc::clone(&app_ctx));
    let exec_ctx = request_exec_ctx(&app_ctx, "pg18-migration-harness");

    let fixture_sql = include_str!("../../../../cli/tests/pg18_migration_fixture.sql");
    for stmt in split_statements(fixture_sql).expect("split fixture SQL") {
        let trimmed = stmt.trim();
        if !trimmed.is_empty() {
            execute_ok(&executor, &exec_ctx, trimmed).await;
        }
    }

    // 1. Verify ORM/GUI catalog introspection queries against pg_catalog and information_schema
    //    (T068/T070)
    let tables_res = execute_ok(
        &executor,
        &exec_ctx,
        "SELECT table_schema, table_name FROM information_schema.tables WHERE table_schema = \
         'mig_app'",
    )
    .await;
    if let ExecutionResult::Rows { row_count, .. } = tables_res {
        assert_eq!(row_count, 1);
    } else {
        panic!("expected information_schema.tables row");
    }

    let enums_res = execute_ok(
        &executor,
        &exec_ctx,
        "SELECT e.enumlabel FROM pg_catalog.pg_enum e JOIN pg_catalog.pg_type t ON e.enumtypid = \
         t.oid WHERE t.typname = 'order_state' ORDER BY e.enumsortorder",
    )
    .await;
    if let ExecutionResult::Rows {
        batches, row_count, ..
    } = enums_res
    {
        assert_eq!(row_count, 3);
        let labels = batches[0].column(0).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(labels.value(0), "pending");
        assert_eq!(labels.value(1), "paid");
        assert_eq!(labels.value(2), "shipped");
    } else {
        panic!("expected pg_enum rows");
    }

    let attrdef_res =
        execute_ok(&executor, &exec_ctx, "SELECT adbin FROM pg_catalog.pg_attrdef").await;
    if let ExecutionResult::Rows { row_count, .. } = attrdef_res {
        assert!(row_count >= 1, "expected pg_attrdef rows for default expressions");
    } else {
        panic!("expected pg_attrdef rows");
    }

    // 2. Compare typed data contents and domain/column defaults (T071/T109)
    let expected = vec![
        OrderRowSnapshot {
            id: 1,
            customer: "alice".to_string(),
            qty: 2,
            amount: "49.95".to_string(),
            state: "paid".to_string(),
            metadata: "{\"Priority\":1,\"tier\":\"gold\"}".to_string(),
        },
        OrderRowSnapshot {
            id: 2,
            customer: "bob".to_string(),
            qty: 1, // materialized from DOMAIN positive_qty DEFAULT 1
            amount: "19.50".to_string(),
            state: "pending".to_string(), // materialized from column DEFAULT 'pending'
            metadata: "{\"tier\":\"silver\"}".to_string(),
        },
    ];
    let actual = snapshot_orders(&executor, &exec_ctx).await;
    assert_eq!(
        actual, expected,
        "typed data contents must match expected PostgreSQL 18 snapshot"
    );

    // 3. Intentionally corrupt equal-count data to prove the comparator catches it (T109)
    execute_ok(&executor, &exec_ctx, "UPDATE mig_app.orders SET amount = 19.51 WHERE id = 2").await;
    let corrupted = snapshot_orders(&executor, &exec_ctx).await;
    assert_eq!(corrupted.len(), expected.len(), "row count remains equal");
    assert_ne!(
        corrupted, expected,
        "typed content comparator must detect equal-count data corruption"
    );
}
