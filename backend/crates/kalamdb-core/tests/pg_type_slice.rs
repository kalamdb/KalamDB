//! Vertical slice for PostgreSQL type identity.
//!
//! `numeric(12,2)`, `jsonb`, `timestamptz`, a named enum, and a composite array
//! must keep their meaning through planning, hot storage, and text and binary
//! results. This is the T042 gate: it fails while those forms still collapse.

mod support;

use std::sync::Arc;

use arrow::util::display::array_value_to_string;
use kalamdb_commons::{
    conversions::read_logical_type_modifier,
    models::{
        datatypes::WireFormat, CatalogTypeKind, KalamDataType, LogicalTypeRef, TableId, TableName,
    },
    Role, UserId,
};
use kalamdb_core::{
    app_context::AppContext,
    sql::{
        context::{ExecutionContext, ExecutionResult},
        executor::{handler_registry::HandlerRegistry, SqlExecutor},
    },
};
use kalamdb_system::CatalogTypeField;

fn executor(app: Arc<AppContext>) -> SqlExecutor {
    let registry = Arc::new(HandlerRegistry::new());
    kalamdb_handlers::register_all_handlers(&registry, Arc::clone(&app), false);
    SqlExecutor::new(app, registry)
}

fn exec_ctx(app: &Arc<AppContext>) -> ExecutionContext {
    ExecutionContext::new(UserId::from("pg-type-slice"), Role::Dba, app.base_session_context())
}

async fn exec(
    executor: &SqlExecutor,
    ctx: &ExecutionContext,
    sql: &str,
) -> Result<ExecutionResult, String> {
    executor.execute(sql, ctx, vec![]).await.map_err(|error| error.to_string())
}

fn column<'a>(
    table: &'a kalamdb_commons::models::schemas::TableDefinition,
    name: &str,
) -> &'a kalamdb_commons::models::schemas::ColumnDefinition {
    table
        .columns
        .iter()
        .find(|column| column.column_name == name)
        .unwrap_or_else(|| panic!("column {name} missing"))
}

fn wire_bytes(data_type: KalamDataType) -> Vec<u8> {
    let mut bytes = Vec::new();
    data_type.encode(&mut bytes).expect("type wire bytes");
    bytes
}

fn note(gaps: &mut Vec<String>, condition: bool, message: impl Into<String>) {
    if condition {
        gaps.push(message.into());
    }
}

#[tokio::test]
#[ntest::timeout(120_000)]
async fn pg_type_slice_plan_storage_text_and_binary() {
    let (app, _db) = support::create_cluster_app_context().await;
    let executor = executor(Arc::clone(&app));
    let ctx = exec_ctx(&app);
    let schema = support::unique_namespace("pgslice");
    let schema_name = schema.as_str();

    exec(&executor, &ctx, &format!("CREATE SCHEMA {schema_name}"))
        .await
        .expect("create schema");
    exec(
        &executor,
        &ctx,
        &format!("CREATE TYPE {schema_name}.charge_status AS ENUM ('open', 'paid', 'void')"),
    )
    .await
    .expect("create enum");
    exec(
        &executor,
        &ctx,
        &format!("CREATE TYPE {schema_name}.payment AS (amount numeric(12,2), note text)"),
    )
    .await
    .expect("create composite");
    exec(
        &executor,
        &ctx,
        &format!(
            "CREATE TABLE {schema_name}.charges (
                id BIGINT PRIMARY KEY,
                amount numeric(12,2),
                payload jsonb,
                charged_at timestamptz,
                status charge_status,
                lines payment[]
            ) WITH (TYPE = 'SHARED')"
        ),
    )
    .await
    .expect("create table");

    let table_id = TableId::new(schema.clone(), TableName::new("charges"));
    let table = app
        .schema_registry()
        .get_table_if_exists(&table_id)
        .expect("load table")
        .expect("charges table");
    let stores = app.system_tables().catalog_stores();
    let enum_type = stores
        .find_type(&schema, "charge_status")
        .expect("enum lookup")
        .expect("charge_status");
    let composite = stores
        .find_type(&schema, "payment")
        .expect("composite lookup")
        .expect("payment");
    let mut enum_fields = stores.list_type_fields(&enum_type.type_id).expect("enum fields");
    enum_fields.sort_by_key(|field| field.ordinal);
    let mut composite_fields =
        stores.list_type_fields(&composite.type_id).expect("composite fields");
    composite_fields.sort_by_key(|field| field.ordinal);

    let mut gaps = Vec::new();
    plan_gaps(
        &mut gaps,
        &table,
        &enum_type.kind,
        &enum_type.type_id,
        &enum_fields,
        &composite.kind,
        &composite.type_id,
        &composite_fields,
    );

    let inserted = exec(
        &executor,
        &ctx,
        &format!(
            "INSERT INTO {schema_name}.charges (id, amount, payload, charged_at, status, lines) \
             VALUES (1, 1234.50, '{{\"a\":1,\"b\":2}}', '2020-01-15 00:00:00+00', 'paid', NULL)"
        ),
    )
    .await;
    let stored = match inserted {
        Ok(result) => result,
        Err(error) => {
            gaps.push(format!("hot storage insert failed: {error}"));
            assert_gaps(&gaps);
            return;
        },
    };
    note(
        &mut gaps,
        !matches!(stored, ExecutionResult::Inserted { rows_affected: 1 }),
        format!("insert did not report one row: {stored:?}"),
    );

    let selected = exec(
        &executor,
        &ctx,
        &format!(
            "SELECT id, amount, payload, charged_at, status, lines FROM {schema_name}.charges \
             WHERE id = 1"
        ),
    )
    .await;
    match selected {
        Ok(result) => result_gaps(&mut gaps, result),
        Err(error) => gaps.push(format!("hot storage select failed: {error}")),
    }

    let array_insert = exec(
        &executor,
        &ctx,
        &format!(
            "INSERT INTO {schema_name}.charges (id, amount, payload, charged_at, status, lines) \
             VALUES (2, 10.25, 'null', '2020-01-15 00:00:00+00', 'open', ARRAY[ROW(10.25, \
             'wire')::{schema_name}.payment])"
        ),
    )
    .await;
    if let Err(error) = array_insert {
        gaps.push(format!("composite array value was not stored: {error}"));
    }

    assert_gaps(&gaps);
}

fn plan_gaps(
    gaps: &mut Vec<String>,
    table: &kalamdb_commons::models::schemas::TableDefinition,
    enum_kind: &CatalogTypeKind,
    enum_id: &kalamdb_commons::models::TypeId,
    enum_fields: &[CatalogTypeField],
    composite_kind: &CatalogTypeKind,
    composite_id: &kalamdb_commons::models::TypeId,
    composite_fields: &[CatalogTypeField],
) {
    note(
        gaps,
        *enum_kind != CatalogTypeKind::Enum,
        format!("charge_status kind is {enum_kind}, expected enum"),
    );
    let labels: Vec<&str> = enum_fields.iter().map(|field| field.name.as_str()).collect();
    note(
        gaps,
        labels != ["open", "paid", "void"],
        format!("enum label order is {labels:?}, expected open, paid, void"),
    );
    note(
        gaps,
        *composite_kind != CatalogTypeKind::Composite,
        format!("payment kind is {composite_kind}, expected composite"),
    );
    let amount_field = composite_fields.iter().find(|field| field.name == "amount");
    note(
        gaps,
        amount_field.and_then(|field| field.data_type)
            != Some(KalamDataType::Decimal {
                precision: 12,
                scale:     2,
            }),
        "composite attribute numeric(12,2) did not keep precision 12 scale 2",
    );

    let amount = column(table, "amount");
    let payload = column(table, "payload");
    let charged_at = column(table, "charged_at");
    let status = column(table, "status");
    let lines = column(table, "lines");

    note(
        gaps,
        amount.data_type()
            != KalamDataType::Decimal {
                precision: 12,
                scale:     2,
            },
        format!("plan numeric(12,2) became {}", amount.data_type().sql_name()),
    );
    note(
        gaps,
        !payload.logical_type.is_jsonb() || payload.logical_type.sql_name() != "JSONB",
        format!(
            "plan jsonb collapsed to {} (wire {:02x?})",
            payload.logical_type.sql_name(),
            wire_bytes(payload.data_type())
        ),
    );
    note(
        gaps,
        !charged_at.logical_type.is_timestamptz()
            || charged_at.logical_type.sql_name() != "TIMESTAMPTZ",
        format!(
            "plan timestamptz collapsed to {} (wire {:02x?})",
            charged_at.logical_type.sql_name(),
            wire_bytes(charged_at.data_type())
        ),
    );
    note(
        gaps,
        status.named_type_id() != Some(enum_id),
        "plan enum column is not the catalog charge_status identity",
    );
    let lines_are_composite_array = matches!(
        &lines.logical_type,
        LogicalTypeRef::List { element, .. }
            if matches!(element.as_ref(), LogicalTypeRef::Named { type_id } if type_id == composite_id)
    );
    note(
        gaps,
        !lines_are_composite_array,
        format!("plan composite array became {}", lines.logical_type),
    );
}

fn result_gaps(gaps: &mut Vec<String>, result: ExecutionResult) {
    let ExecutionResult::Rows {
        batches, row_count, ..
    } = result
    else {
        gaps.push(format!("select did not return rows: {result:?}"));
        return;
    };
    note(gaps, row_count != 1, format!("select returned {row_count} rows"));
    let Some(batch) = batches.first() else {
        gaps.push("select returned no batch".to_string());
        return;
    };
    if batch.num_rows() == 0 {
        gaps.push("select batch is empty".to_string());
        return;
    }

    for (index, field) in batch.schema().fields().iter().enumerate() {
        let column = batch.column(index);
        let text = array_value_to_string(column, 0).unwrap_or_else(|error| error.to_string());
        match field.name().as_str() {
            "amount" => {
                note(
                    gaps,
                    text != "1234.50",
                    format!("text numeric(12,2) is {text:?}, expected 1234.50"),
                );
                note(
                    gaps,
                    !matches!(field.data_type(), arrow::datatypes::DataType::Decimal128(12, 2)),
                    format!("binary numeric(12,2) arrow type is {:?}", field.data_type()),
                );
            },
            "payload" => {
                note(
                    gaps,
                    read_logical_type_modifier(field) != Some("jsonb"),
                    format!(
                        "jsonb result field lost jsonb modifier ({:?})",
                        read_logical_type_modifier(field)
                    ),
                );
                note(
                    gaps,
                    text != "{\"a\":1,\"b\":2}",
                    format!("jsonb result text is {text:?}"),
                );
            },
            "charged_at" => {
                note(
                    gaps,
                    !matches!(field.data_type(), arrow::datatypes::DataType::Timestamp(_, Some(_)))
                        || read_logical_type_modifier(field) != Some("timestamptz"),
                    format!("binary timestamptz arrow type is {:?}", field.data_type()),
                );
            },
            "status" => {
                note(gaps, text != "paid", format!("text enum is {text:?}, expected paid"));
            },
            "lines" => {
                let null_text = text.is_empty() || text.eq_ignore_ascii_case("null");
                note(gaps, !null_text, format!("null composite array text is {text:?}"));
            },
            _ => {},
        }
    }
}

fn assert_gaps(gaps: &[String]) {
    assert!(gaps.is_empty(), "pg type slice gaps:\n{}", gaps.join("\n"));
}
