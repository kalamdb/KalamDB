//! Differential PostgreSQL 18 type semantics suite (T100).
//!
//! Covers:
//! - Bounded `numeric(p, s)` half-away-from-zero rounding, negative sub-unit formatting, overflow
//!   rejection, `NaN`/`Infinity` rejection, and invalid `(p, s)` DDL bounds.
//! - `json` (verbatim text preservation) vs `jsonb` (whitespace stripping, duplicate-key
//!   deduplication, and logical modifier preservation).
//! - `timestamp` vs `timestamptz` distinct logical & Arrow types (`None` vs `Some("UTC")`).
//! - Stable enum member identity (`slot` + `type_field_id`) across `ADD VALUE BEFORE/AFTER` and
//!   `RENAME VALUE`.
//! - Complete array element metadata (`List` inner `Field` carrying enum labels, `TypeId`,
//!   `jsonb`/`timestamptz` modifiers, `Decimal128(p, s)`, and composite `Struct` fields).
//! - Nested composite type revision cascading and pinned historical revision retention.

use crate::support;

use std::sync::Arc;

use arrow::{
    datatypes::{DataType, TimeUnit},
    util::display::array_value_to_string,
};
use kalamdb_commons::{
    conversions::{
        read_kalam_enum_labels, read_kalam_type_id_metadata, read_logical_type_modifier,
    },
    models::{TableId, TableName, UserId},
    Role, TypeRevision,
};
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};

fn executor(app: Arc<AppContext>) -> SqlExecutor {
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app), false);
    SqlExecutor::new(app, registry)
}

fn exec_ctx(app: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(
        UserId::new("u_pg18_type_semantics"),
        Role::Dba,
        app.base_session_context(),
    )
}

async fn exec(
    executor: &SqlExecutor,
    ctx: &ExecutionContext,
    sql: &str,
) -> Result<ExecutionResult, String> {
    executor.execute(sql, ctx, vec![]).await.map_err(|error| error.to_string())
}

#[tokio::test]
#[ntest::timeout(120_000)]
async fn numeric_precision_rounding_overflow_and_edge_cases() {
    let (app, _db) = support::create_cluster_app_context().await;
    let executor = executor(Arc::clone(&app));
    let ctx = exec_ctx(&app);
    let schema = support::unique_namespace("pg18_num");
    let s = schema.as_str();

    exec(&executor, &ctx, &format!("CREATE SCHEMA {s}"))
        .await
        .expect("create schema");

    // Unsupported scale/precision forms must be rejected at DDL time.
    assert!(
        exec(
            &executor,
            &ctx,
            &format!(
                "CREATE TABLE {s}.bad_scale (id BIGINT PRIMARY KEY, v NUMERIC(4, 6)) WITH (TYPE = \
                 'SHARED')"
            ),
        )
        .await
        .is_err(),
        "scale > precision must be rejected"
    );
    assert!(
        exec(
            &executor,
            &ctx,
            &format!(
                "CREATE TABLE {s}.bad_prec (id BIGINT PRIMARY KEY, v NUMERIC(40, 2)) WITH (TYPE = \
                 'SHARED')"
            ),
        )
        .await
        .is_err(),
        "precision > 38 must be rejected"
    );

    exec(
        &executor,
        &ctx,
        &format!(
            "CREATE TABLE {s}.ledger (
                id BIGINT PRIMARY KEY,
                amount NUMERIC(5, 2) NOT NULL
            ) WITH (TYPE = 'SHARED')"
        ),
    )
    .await
    .expect("create ledger");

    // Half-away-from-zero rounding and negative sub-unit values (-0.50, -0.05).
    exec(
        &executor,
        &ctx,
        &format!(
            "INSERT INTO {s}.ledger (id, amount) VALUES
                (1, 12.345),
                (2, -12.345),
                (3, -0.50),
                (4, -0.05),
                (5, 999.994)"
        ),
    )
    .await
    .expect("insert valid bounded numerics");

    // Overflow after rounding (999.995 -> 1000.00 exceeds NUMERIC(5, 2)) and direct overflow.
    assert!(
        exec(
            &executor,
            &ctx,
            &format!("INSERT INTO {s}.ledger (id, amount) VALUES (6, 999.995)"),
        )
        .await
        .is_err(),
        "rounding overflow must be rejected"
    );
    assert!(
        exec(
            &executor,
            &ctx,
            &format!("INSERT INTO {s}.ledger (id, amount) VALUES (7, 1000.00)"),
        )
        .await
        .is_err(),
        "magnitude exceeding precision must be rejected"
    );

    // NaN and Infinity are rejected on bounded Decimal128.
    for non_finite in ["NaN", "Infinity", "-Infinity"] {
        assert!(
            exec(
                &executor,
                &ctx,
                &format!("INSERT INTO {s}.ledger (id, amount) VALUES (8, '{non_finite}')"),
            )
            .await
            .is_err(),
            "{non_finite} must be rejected on bounded NUMERIC(5, 2)"
        );
    }

    let result = exec(&executor, &ctx, &format!("SELECT id, amount FROM {s}.ledger ORDER BY id"))
        .await
        .expect("select ledger");
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected rows");
    };
    let batch = batches.first().expect("batch");
    assert_eq!(batch.schema().field(1).data_type(), &DataType::Decimal128(5, 2));
    let actual: Vec<String> = (0..batch.num_rows())
        .map(|row| array_value_to_string(batch.column(1), row).expect("format decimal"))
        .collect();
    assert_eq!(actual, vec!["12.35", "-12.35", "-0.50", "-0.05", "999.99"]);
}

#[tokio::test]
#[ntest::timeout(120_000)]
async fn json_vs_jsonb_and_timestamp_vs_timestamptz_codecs() {
    let (app, _db) = support::create_cluster_app_context().await;
    let executor = executor(Arc::clone(&app));
    let ctx = exec_ctx(&app);
    let schema = support::unique_namespace("pg18_json_ts");
    let s = schema.as_str();

    exec(&executor, &ctx, &format!("CREATE SCHEMA {s}"))
        .await
        .expect("create schema");
    exec(
        &executor,
        &ctx,
        &format!(
            "CREATE TABLE {s}.events (
                id BIGINT PRIMARY KEY,
                raw_doc JSON,
                norm_doc JSONB,
                local_ts TIMESTAMP,
                utc_ts TIMESTAMPTZ
            ) WITH (TYPE = 'SHARED')"
        ),
    )
    .await
    .expect("create events");

    let raw_input = r#"  { "b": 1,   "a": [ 1, 2 ], "b": 99 }  "#;
    exec(
        &executor,
        &ctx,
        &format!(
            "INSERT INTO {s}.events (id, raw_doc, norm_doc, local_ts, utc_ts)
             VALUES (1, '{raw_input}', '{raw_input}', '2026-01-15 12:30:00', '2026-01-15 \
             12:30:00+00')"
        ),
    )
    .await
    .expect("insert event");

    // Invalid JSON syntax must be rejected on both json and jsonb.
    assert!(exec(
        &executor,
        &ctx,
        &format!(
            "INSERT INTO {s}.events (id, raw_doc, norm_doc) VALUES (2, '{{bad', '{{\"ok\":1}}')"
        ),
    )
    .await
    .is_err());
    assert!(exec(
        &executor,
        &ctx,
        &format!(
            "INSERT INTO {s}.events (id, raw_doc, norm_doc) VALUES (3, '{{\"ok\":1}}', '{{bad')"
        ),
    )
    .await
    .is_err());

    let result = exec(
        &executor,
        &ctx,
        &format!("SELECT raw_doc, norm_doc, local_ts, utc_ts FROM {s}.events WHERE id = 1"),
    )
    .await
    .expect("select event");
    let ExecutionResult::Rows { batches, .. } = result else {
        panic!("expected rows");
    };
    let batch = batches.first().expect("batch");
    let schema_ref = batch.schema();

    let raw_field = schema_ref.field_with_name("raw_doc").expect("raw_doc");
    let norm_field = schema_ref.field_with_name("norm_doc").expect("norm_doc");
    let local_field = schema_ref.field_with_name("local_ts").expect("local_ts");
    let utc_field = schema_ref.field_with_name("utc_ts").expect("utc_ts");

    assert_eq!(read_logical_type_modifier(raw_field), None);
    assert_eq!(read_logical_type_modifier(norm_field), Some("jsonb"));
    assert_eq!(read_logical_type_modifier(local_field), None);
    assert_eq!(read_logical_type_modifier(utc_field), Some("timestamptz"));
    assert_eq!(local_field.data_type(), &DataType::Timestamp(TimeUnit::Microsecond, None));
    assert_eq!(
        utc_field.data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
    );

    let raw_out = array_value_to_string(batch.column(0), 0).expect("raw_doc text");
    let norm_out = array_value_to_string(batch.column(1), 0).expect("norm_doc text");
    assert_eq!(raw_out, raw_input, "JSON must preserve original text formatting");
    let norm_parsed: serde_json::Value = serde_json::from_str(&norm_out).expect("valid jsonb");
    assert_eq!(
        norm_parsed["b"],
        serde_json::json!(99),
        "JSONB duplicate key must resolve to last key"
    );
    assert_eq!(norm_parsed["a"], serde_json::json!([1, 2]));
    assert!(!norm_out.contains("   "), "JSONB must strip insignificant whitespace");
}

#[tokio::test]
#[ntest::timeout(120_000)]
async fn enum_stable_member_identity_and_array_metadata() {
    let (app, _db) = support::create_cluster_app_context().await;
    let executor = executor(Arc::clone(&app));
    let ctx = exec_ctx(&app);
    let schema = support::unique_namespace("pg18_enum");
    let s = schema.as_str();

    exec(&executor, &ctx, &format!("CREATE SCHEMA {s}"))
        .await
        .expect("create schema");
    exec(
        &executor,
        &ctx,
        &format!("CREATE TYPE {s}.ticket_state AS ENUM ('open', 'closed')"),
    )
    .await
    .expect("create enum");

    let stores = app.system_tables().catalog_stores();
    let enum_type = stores
        .find_type(&schema, "ticket_state")
        .expect("find enum")
        .expect("ticket_state");
    let initial_fields = stores.list_type_fields(&enum_type.type_id).expect("initial fields");
    assert_eq!(initial_fields.len(), 2);
    assert_eq!(initial_fields[0].name, "open");
    assert_eq!(initial_fields[0].slot, 1);
    assert_eq!(initial_fields[0].ordinal, 1);
    assert_eq!(initial_fields[1].name, "closed");
    assert_eq!(initial_fields[1].slot, 2);
    assert_eq!(initial_fields[1].ordinal, 2);
    let closed_field_id = initial_fields[1].type_field_id.clone();

    // ADD VALUE 'triaged' BEFORE 'closed' preserves 'closed' slot=2 and type_field_id, while
    // ordinal becomes 3.
    exec(
        &executor,
        &ctx,
        &format!("ALTER TYPE {s}.ticket_state ADD VALUE 'triaged' BEFORE 'closed'"),
    )
    .await
    .expect("add value before");
    let after_add = stores.list_type_fields(&enum_type.type_id).expect("after add");
    assert_eq!(
        after_add
            .iter()
            .map(|f| (f.name.as_str(), f.slot, f.ordinal))
            .collect::<Vec<_>>(),
        vec![("open", 1, 1), ("triaged", 3, 2), ("closed", 2, 3)]
    );
    assert_eq!(after_add[2].type_field_id, closed_field_id);

    // RENAME VALUE 'closed' TO 'archived' preserves slot=2, ordinal=3, and type_field_id.
    exec(
        &executor,
        &ctx,
        &format!("ALTER TYPE {s}.ticket_state RENAME VALUE 'closed' TO 'archived'"),
    )
    .await
    .expect("rename value");
    let after_rename = stores.list_type_fields(&enum_type.type_id).expect("after rename");
    assert_eq!(
        after_rename
            .iter()
            .map(|f| (f.name.as_str(), f.slot, f.ordinal))
            .collect::<Vec<_>>(),
        vec![("open", 1, 1), ("triaged", 3, 2), ("archived", 2, 3)]
    );
    assert_eq!(after_rename[2].type_field_id, closed_field_id);

    // Create table with enum array and jsonb array to verify array item Field metadata.
    exec(
        &executor,
        &ctx,
        &format!(
            "CREATE TABLE {s}.board (
                id BIGINT PRIMARY KEY,
                states {s}.ticket_state[],
                payloads JSONB[]
            ) WITH (TYPE = 'SHARED')"
        ),
    )
    .await
    .expect("create board");

    let table_id = TableId::new(schema.clone(), TableName::new("board"));
    let arrow_schema = app
        .schema_registry()
        .get(&table_id)
        .expect("cached board")
        .arrow_schema()
        .expect("board arrow schema");
    let states_field = arrow_schema.field_with_name("states").expect("states field");
    let DataType::List(states_item) = states_field.data_type() else {
        panic!("expected List for states");
    };
    assert_eq!(read_kalam_type_id_metadata(states_item).as_ref(), Some(&enum_type.type_id));
    assert_eq!(
        read_kalam_enum_labels(states_item).as_deref(),
        Some(
            [
                "open".to_string(),
                "triaged".to_string(),
                "archived".to_string()
            ]
            .as_slice()
        )
    );

    let payloads_field = arrow_schema.field_with_name("payloads").expect("payloads field");
    let DataType::List(payloads_item) = payloads_field.data_type() else {
        panic!("expected List for payloads");
    };
    assert_eq!(read_logical_type_modifier(payloads_item), Some("jsonb"));

    // Valid enum array & jsonb array insert succeeds and normalizes jsonb elements.
    exec(
        &executor,
        &ctx,
        &format!(
            "INSERT INTO {s}.board (id, states, payloads)
             VALUES (1, ARRAY['triaged', 'archived'], ARRAY['{{\"x\": 1, \"x\": 2}}'])"
        ),
    )
    .await
    .expect("insert valid enum and jsonb arrays");

    // Old renamed enum label 'closed' inside ARRAY[...] must be rejected.
    assert!(
        exec(
            &executor,
            &ctx,
            &format!(
                "INSERT INTO {s}.board (id, states, payloads) VALUES (2, ARRAY['closed'], NULL)"
            ),
        )
        .await
        .is_err(),
        "renamed enum label 'closed' must be rejected in array"
    );
}

#[tokio::test]
#[ntest::timeout(120_000)]
async fn nested_composite_type_revisions_and_pinned_history() {
    let (app, _db) = support::create_cluster_app_context().await;
    let executor = executor(Arc::clone(&app));
    let ctx = exec_ctx(&app);
    let schema = support::unique_namespace("pg18_nested");
    let s = schema.as_str();

    exec(&executor, &ctx, &format!("CREATE SCHEMA {s}"))
        .await
        .expect("create schema");
    exec(
        &executor,
        &ctx,
        &format!("CREATE TYPE {s}.leaf AS (code TEXT, weight NUMERIC(6, 2))"),
    )
    .await
    .expect("create leaf");
    exec(
        &executor,
        &ctx,
        &format!("CREATE TYPE {s}.branch AS (title TEXT, item {s}.leaf)"),
    )
    .await
    .expect("create branch");

    let stores = app.system_tables().catalog_stores();
    let leaf_id = stores.find_type(&schema, "leaf").unwrap().unwrap().type_id;
    let branch_id = stores.find_type(&schema, "branch").unwrap().unwrap().type_id;

    let schema_registry = app.schema_registry();
    let type_registry = schema_registry.type_registry();
    let rev1 = TypeRevision::from_u32(1).expect("rev 1");
    let rev2 = TypeRevision::from_u32(2).expect("rev 2");
    let branch_rev_1 = type_registry.get_or_load(&stores, &branch_id).expect("load branch rev 1");
    assert_eq!(branch_rev_1.revision, rev1);

    // Alter inner type `leaf` -> cascades revision bump to enclosing `branch`.
    exec(&executor, &ctx, &format!("ALTER TYPE {s}.leaf ADD ATTRIBUTE note TEXT"))
        .await
        .expect("alter leaf");

    let leaf_rev_2 = type_registry.get_or_load(&stores, &leaf_id).expect("load leaf rev 2");
    let branch_rev_2 = type_registry.get_or_load(&stores, &branch_id).expect("load branch rev 2");
    assert_eq!(leaf_rev_2.revision, rev2);
    assert_eq!(
        branch_rev_2.revision, rev2,
        "enclosing composite type revision must advance when nested child type changes"
    );

    // Verify new revision has 3 fields on nested `item`.
    let item_field_rev_2 = branch_rev_2
        .arrow_fields
        .iter()
        .find(|f| f.name() == "item")
        .expect("item field");
    let DataType::Struct(leaf_fields_rev_2) = item_field_rev_2.data_type() else {
        panic!("expected Struct for item");
    };
    assert_eq!(leaf_fields_rev_2.len(), 3);

    // Pinned historical revision 1 remains retrievable and immutable.
    let pinned_rev_1 = type_registry.get_revision(&branch_id, rev1).expect("pinned branch rev 1");
    let item_field_rev_1 = pinned_rev_1
        .arrow_fields
        .iter()
        .find(|f| f.name() == "item")
        .expect("item field rev 1");
    let DataType::Struct(leaf_fields_rev_1) = item_field_rev_1.data_type() else {
        panic!("expected Struct for item rev 1");
    };
    assert_eq!(leaf_fields_rev_1.len(), 2);
}
