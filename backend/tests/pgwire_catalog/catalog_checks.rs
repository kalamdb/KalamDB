//! Shared catalog query definitions and parity helpers for US9 wire e2e tests.
//!
//! See `specs/033-unified-backend-pgwire/validation/us9-client-catalog.md`.

use std::collections::BTreeSet;

use tokio_postgres::{Client, SimpleQueryMessage};

pub const FIXTURE_NAMESPACE: &str = "catalog_e2e";
pub const FIXTURE_TABLE: &str = "items";

/// Catalog surfaces clients rely on (pg_catalog shims + information_schema).
pub struct CatalogQuery {
    pub label: &'static str,
    pub sql: &'static str,
    pub min_rows: usize,
}

pub fn required_pg_catalog_queries() -> &'static [CatalogQuery] {
    &[
        CatalogQuery {
            label: "pg_namespace",
            sql: "SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname NOT IN \
                   ('pg_catalog', 'information_schema', 'pg_toast') ORDER BY 1",
            min_rows: 1,
        },
        CatalogQuery {
            label: "pg_class_fixture",
            sql: "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'items'",
            min_rows: 1,
        },
        CatalogQuery {
            label: "pg_attribute_fixture",
            sql: "SELECT a.attname FROM pg_catalog.pg_attribute a \
                   JOIN pg_catalog.pg_class c ON a.attrelid = c.oid \
                   WHERE c.relname = 'items' AND a.attnum > 0 ORDER BY a.attnum",
            min_rows: 2,
        },
        CatalogQuery {
            label: "pg_database",
            sql: "SELECT datname FROM pg_catalog.pg_database WHERE datistemplate = false",
            min_rows: 1,
        },
        CatalogQuery {
            label: "pg_type_unqualified_dbeaver",
            sql: "SELECT n.nspname as schema, t.typname as typename, t.oid::integer as typeid \
                   FROM pg_type t \
                   LEFT JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
                   WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM pg_catalog.pg_class c \
                   WHERE c.oid = t.typrelid)) \
                     AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
                     AND t.typname !~ '^_'",
            min_rows: 1,
        },
    ]
}

pub fn required_information_schema_queries() -> &'static [CatalogQuery] {
    &[
        CatalogQuery {
            label: "schemata",
            sql: "SELECT schema_name FROM information_schema.schemata \
                   WHERE schema_name = 'catalog_e2e'",
            min_rows: 1,
        },
        CatalogQuery {
            label: "tables",
            sql: "SELECT table_name FROM information_schema.tables \
                   WHERE table_schema = 'catalog_e2e' AND table_name = 'items'",
            min_rows: 1,
        },
        CatalogQuery {
            label: "columns_udt_name",
            sql: "SELECT column_name, udt_name FROM information_schema.columns \
                   WHERE table_schema = 'catalog_e2e' AND table_name = 'items' \
                   ORDER BY ordinal_position",
            min_rows: 2,
        },
        CatalogQuery {
            label: "columns",
            sql: "SELECT column_name FROM information_schema.columns \
                   WHERE table_schema = 'catalog_e2e' AND table_name = 'items' \
                   ORDER BY ordinal_position",
            min_rows: 2,
        },
    ]
}

async fn simple_query_first_column(client: &Client, sql: &str) -> Result<Vec<String>, String> {
    let messages = client
        .simple_query(sql)
        .await
        .map_err(|error| format!("simple query failed: {error}"))?;

    Ok(messages
        .into_iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => row.get(0).map(str::to_string),
            _ => None,
        })
        .collect())
}

pub async fn assert_catalog_query(client: &Client, query: &CatalogQuery) -> Result<(), String> {
    let rows = simple_query_first_column(client, query.sql)
        .await
        .map_err(|error| format!("{} failed: {error}", query.label))?;

    if rows.len() < query.min_rows {
        return Err(format!(
            "{} returned {} rows, expected at least {}",
            query.label,
            rows.len(),
            query.min_rows
        ));
    }

    Ok(())
}

pub async fn assert_all_catalog_queries(client: &Client) -> Result<(), String> {
    for query in required_pg_catalog_queries() {
        assert_catalog_query(client, query).await?;
    }
    for query in required_information_schema_queries() {
        assert_catalog_query(client, query).await?;
    }
    Ok(())
}

/// SC-011: fixture table visible via pg_class and information_schema.tables over the same connection.
pub async fn assert_pg_class_matches_system_tables(client: &Client) -> Result<(), String> {
    let pg_rows = simple_query_first_column(
        client,
        "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'items'",
    )
    .await
    .map_err(|error| format!("pg_class parity query failed: {error}"))?;

    let system_rows = simple_query_first_column(
        client,
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'catalog_e2e' AND table_name = 'items'",
    )
    .await
    .map_err(|error| format!("information_schema.tables parity query failed: {error}"))?;

    let pg_names: BTreeSet<String> = pg_rows.into_iter().collect();
    let system_names: BTreeSet<String> = system_rows.into_iter().collect();

    if pg_names != system_names {
        return Err(format!(
            "pg_class vs information_schema.tables mismatch: pg_class={pg_names:?} \
             information_schema={system_names:?}"
        ));
    }

    Ok(())
}

/// SC-011: column names via pg_attribute vs information_schema.columns for the fixture table.
pub async fn assert_pg_attribute_matches_system_columns(client: &Client) -> Result<(), String> {
    let pg_rows = simple_query_first_column(
        client,
        "SELECT a.attname FROM pg_catalog.pg_attribute a \
         JOIN pg_catalog.pg_class c ON a.attrelid = c.oid \
         WHERE c.relname = 'items' AND a.attnum > 0",
    )
    .await
    .map_err(|error| format!("pg_attribute parity query failed: {error}"))?;

    let system_rows = simple_query_first_column(
        client,
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = 'catalog_e2e' AND table_name = 'items'",
    )
    .await
    .map_err(|error| format!("information_schema.columns parity query failed: {error}"))?;

    let pg_cols: BTreeSet<String> = pg_rows.into_iter().collect();
    let system_cols: BTreeSet<String> = system_rows.into_iter().collect();

    if pg_cols != system_cols {
        return Err(format!(
            "pg_attribute vs information_schema.columns mismatch: pg={pg_cols:?} \
             information_schema={system_cols:?}"
        ));
    }

    Ok(())
}

pub async fn assert_admin_pg_stat_activity(client: &Client) -> Result<(), String> {
    let rows = simple_query_first_column(client, "SELECT usename FROM pg_catalog.pg_stat_activity")
        .await
        .map_err(|error| format!("pg_stat_activity failed: {error}"))?;

    if rows.is_empty() {
        return Err("pg_stat_activity returned no rows for admin connection".into());
    }

    Ok(())
}
