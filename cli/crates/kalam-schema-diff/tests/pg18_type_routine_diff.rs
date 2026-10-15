use kalam_schema_diff::{diff_schema_sql, diff_schema_sql_with_options, DiffOptions};

#[test]
fn pg18_type_and_routine_diff_emits_alter_and_respects_destructive_flag() {
    let before = r#"
        CREATE SCHEMA commerce;
        CREATE TYPE commerce.order_status AS ENUM ('pending', 'paid');
        CREATE TYPE commerce.shipping_address AS (
            street TEXT,
            city TEXT,
            legacy_code TEXT
        );
        CREATE DOMAIN commerce.positive_cents AS BIGINT CHECK (VALUE > 0);
        CREATE FUNCTION commerce.normalize_sku(raw_sku TEXT)
        RETURNS TEXT
        LANGUAGE JAVASCRIPT
        IMMUTABLE
        AS 'export default function normalize_sku(raw_sku) { return raw_sku.trim(); }';
        CREATE PROCEDURE commerce.old_cleanup(batch_size INT)
        LANGUAGE JAVASCRIPT
        AS 'export default async function old_cleanup(ctx, batch_size) {}';
    "#;

    let after = r#"
        CREATE SCHEMA commerce;
        CREATE TYPE commerce.order_status AS ENUM ('pending', 'paid', 'shipped');
        CREATE TYPE commerce.shipping_address AS (
            street TEXT,
            city TEXT,
            postal_code TEXT
        );
        CREATE DOMAIN commerce.positive_cents AS BIGINT NOT NULL DEFAULT 100 CHECK (VALUE > 0) CHECK (VALUE < 1000000);
        COMMENT ON DOMAIN commerce.positive_cents IS 'Strictly positive amount in cents';
        CREATE FUNCTION commerce.normalize_sku(raw_sku TEXT)
        RETURNS TEXT
        LANGUAGE JAVASCRIPT
        IMMUTABLE
        PARALLEL SAFE
        AS 'export default function normalize_sku(raw_sku) { return raw_sku.trim().toUpperCase(); }';
        COMMENT ON FUNCTION commerce.normalize_sku IS 'Normalizes SKU strings';
    "#;

    let safe_diff = diff_schema_sql(before, after).expect("safe diff");
    assert!(safe_diff
        .up
        .contains("ALTER TYPE commerce.order_status ADD VALUE 'shipped' AFTER 'paid';"));
    assert!(safe_diff
        .up
        .contains("ALTER TYPE commerce.shipping_address ADD ATTRIBUTE postal_code TEXT;"));
    assert!(safe_diff.up.contains(
        "-- destructive change skipped: ALTER TYPE commerce.shipping_address DROP ATTRIBUTE legacy_code"
    ));
    assert!(safe_diff
        .up
        .contains("ALTER DOMAIN commerce.positive_cents SET DEFAULT 100;"));
    assert!(safe_diff
        .up
        .contains("ALTER DOMAIN commerce.positive_cents SET NOT NULL;"));
    assert!(safe_diff
        .up
        .contains("ALTER DOMAIN commerce.positive_cents ADD CHECK (VALUE < 1000000);"));
    assert!(safe_diff.up.contains(
        "COMMENT ON DOMAIN commerce.positive_cents IS 'Strictly positive amount in cents';"
    ));
    assert!(safe_diff
        .up
        .contains("CREATE OR REPLACE FUNCTION commerce.normalize_sku"));
    assert!(safe_diff
        .up
        .contains("COMMENT ON FUNCTION commerce.normalize_sku IS 'Normalizes SKU strings';"));
    assert!(safe_diff.up.contains(
        "-- destructive change skipped: DROP PROCEDURE commerce.old_cleanup"
    ));

    let destructive_diff = diff_schema_sql_with_options(
        before,
        after,
        &DiffOptions {
            allow_destructive: true,
            default_schema: None,
        },
    )
    .expect("destructive diff");
    assert!(destructive_diff
        .up
        .contains("ALTER TYPE commerce.shipping_address DROP ATTRIBUTE legacy_code;"));
    assert!(destructive_diff
        .up
        .contains("DROP PROCEDURE commerce.old_cleanup;"));
}
