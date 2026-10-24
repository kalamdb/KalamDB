use kalam_schema_diff::{diff_schema_sql, diff_schema_sql_with_options, DiffOptions};

#[test]
fn pg18_table_schema_diff_supports_timestamptz_arrays_custom_types_and_ordering() {
    let before = r#"
        CREATE SCHEMA commerce;
        CREATE TYPE commerce.order_status AS ENUM ('pending', 'paid');
        CREATE DOMAIN commerce.positive_cents AS BIGINT NOT NULL CHECK (VALUE > 0);
        CREATE TABLE commerce.orders (
            id BIGINT PRIMARY KEY,
            status commerce.order_status NOT NULL,
            total_cents commerce.positive_cents NOT NULL,
            tags TEXT[],
            placed_at TIMESTAMPTZ NOT NULL
        ) WITH (TYPE = 'USER');
    "#;

    let after = r#"
        CREATE SCHEMA commerce;
        CREATE TYPE commerce.order_status AS ENUM ('pending', 'paid', 'shipped');
        CREATE DOMAIN commerce.positive_cents AS BIGINT NOT NULL CHECK (VALUE > 0);
        CREATE TABLE commerce.orders (
            id BIGINT PRIMARY KEY,
            status commerce.order_status NOT NULL,
            total_cents commerce.positive_cents NOT NULL,
            tags TEXT[],
            placed_at TIMESTAMP WITH TIME ZONE NOT NULL,
            updated_at TIMESTAMPTZ
        ) WITH (TYPE = 'USER');
    "#;

    let diff = diff_schema_sql(before, after).expect("diff pg18 table schema");
    assert!(
        diff.up
            .contains("ALTER TYPE commerce.order_status ADD VALUE 'shipped' AFTER 'paid';"),
        "expected enum value addition in:\n{}",
        diff.up
    );
    assert!(
        diff.up
            .contains("ALTER TABLE commerce.orders ADD COLUMN updated_at TIMESTAMPTZ;"),
        "expected column addition in:\n{}",
        diff.up
    );
    // TIMESTAMPTZ and TIMESTAMP WITH TIME ZONE canonicalize to the same key, so placed_at is
    // unchanged
    assert!(
        !diff.up.contains("MODIFY COLUMN placed_at"),
        "TIMESTAMPTZ alias should not trigger column modification:\n{}",
        diff.up
    );
}

#[test]
fn pg18_table_schema_diff_orders_schemas_types_tables_routines_and_drops() {
    let after = r#"
        CREATE SCHEMA commerce;
        CREATE TYPE commerce.order_status AS ENUM ('pending', 'paid');
        CREATE DOMAIN commerce.positive_cents AS BIGINT NOT NULL CHECK (VALUE > 0);
        CREATE TABLE commerce.orders (
            id BIGINT PRIMARY KEY,
            status commerce.order_status NOT NULL,
            total_cents commerce.positive_cents NOT NULL
        ) WITH (TYPE = 'SHARED');
        CREATE FUNCTION commerce.normalize_sku(raw_sku TEXT)
        RETURNS TEXT
        LANGUAGE JAVASCRIPT
        IMMUTABLE
        AS 'export default function normalize_sku(raw_sku) { return raw_sku; }';
    "#;

    let diff = diff_schema_sql("", after).expect("diff initial schema");
    let ns_pos = diff
        .up
        .find("CREATE NAMESPACE IF NOT EXISTS commerce;")
        .expect("namespace emitted");
    let type_pos = diff.up.find("CREATE TYPE commerce.order_status").expect("enum emitted");
    let domain_pos = diff.up.find("CREATE DOMAIN commerce.positive_cents").expect("domain emitted");
    let table_pos = diff.up.find("CREATE SHARED TABLE commerce.orders").expect("table emitted");
    let fn_pos = diff
        .up
        .find("CREATE OR REPLACE FUNCTION commerce.normalize_sku")
        .expect("function emitted");

    assert!(ns_pos < type_pos, "namespace before type:\n{}", diff.up);
    assert!(type_pos < table_pos, "type before table:\n{}", diff.up);
    assert!(domain_pos < table_pos, "domain before table:\n{}", diff.up);
    assert!(table_pos < fn_pos, "table before routine:\n{}", diff.up);

    let reverse = diff_schema_sql_with_options(
        after,
        "",
        &DiffOptions {
            allow_destructive: true,
            default_schema:    None,
        },
    )
    .expect("reverse destructive diff");
    let drop_fn_pos = reverse
        .up
        .find("DROP FUNCTION commerce.normalize_sku;")
        .expect("drop function emitted");
    let drop_table_pos =
        reverse.up.find("DROP TABLE commerce.orders;").expect("drop table emitted");
    let drop_domain_pos = reverse
        .up
        .find("DROP DOMAIN commerce.positive_cents;")
        .expect("drop domain emitted");
    let drop_type_pos =
        reverse.up.find("DROP TYPE commerce.order_status;").expect("drop type emitted");

    assert!(drop_fn_pos < drop_table_pos, "drop routine before drop table:\n{}", reverse.up);
    assert!(
        drop_table_pos < drop_domain_pos,
        "drop table before drop domain:\n{}",
        reverse.up
    );
    assert!(drop_table_pos < drop_type_pos, "drop table before drop type:\n{}", reverse.up);
}
