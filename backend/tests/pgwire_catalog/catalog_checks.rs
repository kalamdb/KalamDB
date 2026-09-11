//! Shared catalog query definitions and parity helpers for US9 wire e2e tests.
//!
//! See `specs/033-unified-backend-pgwire/validation/us9-client-catalog.md`.

use std::collections::BTreeSet;

use tokio_postgres::{Client, SimpleQueryMessage};

pub const FIXTURE_NAMESPACE: &str = "catalog_e2e";
pub const FIXTURE_TABLE: &str = "items";

/// Catalog surfaces clients rely on (pg_catalog shims + information_schema).
pub struct CatalogQuery {
    pub label:    &'static str,
    pub sql:      &'static str,
    pub min_rows: usize,
}

pub fn required_pg_catalog_queries() -> &'static [CatalogQuery] {
    &[
        CatalogQuery {
            label:    "pg_namespace",
            sql:      "SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname NOT IN \
                       ('pg_catalog', 'information_schema', 'pg_toast') ORDER BY 1",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "pg_class_fixture",
            sql:      "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'items'",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "pg_attribute_fixture",
            sql:      "SELECT a.attname FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c \
                       ON a.attrelid = c.oid WHERE c.relname = 'items' AND a.attnum > 0 ORDER BY \
                       a.attnum",
            min_rows: 2,
        },
        CatalogQuery {
            label:    "pg_database",
            sql:      "SELECT datname FROM pg_catalog.pg_database WHERE datistemplate = false",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "pg_database_tabularis",
            sql:      "SELECT datname::text FROM pg_database WHERE datistemplate = false ORDER BY \
                       datname",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "pg_database_allowconn",
            sql:      "SELECT datname FROM pg_catalog.pg_database WHERE datallowconn AND NOT \
                       datistemplate",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "current_setting_server_version_num",
            sql:      "SELECT current_setting('server_version_num')::int4 AS v",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "information_schema_triggers",
            sql:      "SELECT trigger_name FROM information_schema.triggers LIMIT 1",
            min_rows: 0,
        },
        CatalogQuery {
            label:    "pg_proc_routines",
            sql:      "SELECT proname, prokind FROM pg_proc WHERE pronamespace = (SELECT oid FROM \
                       pg_namespace WHERE nspname = 'catalog_e2e') AND prokind IN ('f', 'p') \
                       ORDER BY proname",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "pg_settings_empty",
            sql:      "SELECT name FROM pg_settings LIMIT 1",
            min_rows: 0,
        },
        CatalogQuery {
            label:    "pg_roles_empty",
            sql:      "SELECT rolname FROM pg_roles LIMIT 1",
            min_rows: 0,
        },
        CatalogQuery {
            label:    "columns_is_identity",
            sql:      "SELECT column_name, is_identity FROM information_schema.columns WHERE \
                       table_schema = 'catalog_e2e' AND table_name = 'items' LIMIT 1",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "pg_type_unqualified_dbeaver",
            sql:      "SELECT n.nspname as schema, t.typname as typename, t.oid::integer as \
                       typeid FROM pg_type t LEFT JOIN pg_catalog.pg_namespace n ON n.oid = \
                       t.typnamespace WHERE (t.typrelid = 0 OR (SELECT c.relkind = 'c' FROM \
                       pg_catalog.pg_class c WHERE c.oid = t.typrelid)) AND n.nspname NOT IN \
                       ('pg_catalog', 'information_schema') AND t.typname !~ '^_'",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "pg_class_pk_index",
            sql:      "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'items_pkey' AND \
                       relkind = 'i'",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "tabularis_columns",
            sql:      "SELECT c.column_name::text FROM information_schema.columns c WHERE \
                       c.table_schema = 'catalog_e2e' AND c.table_name = 'items' ORDER BY \
                       c.ordinal_position",
            min_rows: 2,
        },
        CatalogQuery {
            label:    "tabularis_column_pk",
            sql:      "SELECT c.column_name::text, EXISTS ( SELECT 1 FROM pg_constraint pk_con \
                       JOIN pg_class pk_table ON pk_table.oid = pk_con.conrelid JOIN pg_namespace \
                       pk_schema ON pk_schema.oid = pk_table.relnamespace JOIN \
                       unnest(pk_con.conkey) AS pk_col(attnum) ON true JOIN pg_attribute pk_att \
                       ON pk_att.attrelid = pk_table.oid AND pk_att.attnum = pk_col.attnum AND \
                       NOT pk_att.attisdropped WHERE pk_con.contype = 'p' AND pk_schema.nspname = \
                       c.table_schema AND pk_table.relname = c.table_name AND pk_att.attname = \
                       c.column_name ) AS is_pk FROM information_schema.columns c WHERE \
                       c.table_schema = 'catalog_e2e' AND c.table_name = 'items' ORDER BY \
                       c.ordinal_position",
            min_rows: 2,
        },
        CatalogQuery {
            label:    "tabularis_indexes",
            sql:      "SELECT i.relname AS index_name FROM pg_class t JOIN pg_namespace n ON \
                       t.relnamespace = n.oid JOIN pg_index ix ON t.oid = ix.indrelid JOIN \
                       pg_class i ON i.oid = ix.indexrelid CROSS JOIN LATERAL \
                       unnest(string_to_array(ix.indkey::text, ' ')::int2[]) WITH ORDINALITY AS \
                       k(attnum, n) LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = \
                       k.attnum AND k.attnum <> 0 WHERE t.relkind IN ('r', 'm') AND n.nspname = \
                       'catalog_e2e' AND t.relname = 'items' ORDER BY i.relname, k.n",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "tabularis_fkeys",
            sql:      "SELECT con.conname::text AS constraint_name FROM pg_constraint con JOIN \
                       pg_class src_cl ON src_cl.oid = con.conrelid JOIN pg_namespace src_nsp ON \
                       src_nsp.oid = src_cl.relnamespace JOIN pg_class ref_cl ON ref_cl.oid = \
                       con.confrelid JOIN pg_namespace ref_nsp ON ref_nsp.oid = \
                       ref_cl.relnamespace JOIN unnest(con.conkey, con.confkey) AS \
                       cols(src_attnum, ref_attnum) ON true JOIN pg_attribute src_att ON \
                       src_att.attrelid = src_cl.oid AND src_att.attnum = cols.src_attnum AND NOT \
                       src_att.attisdropped JOIN pg_attribute ref_att ON ref_att.attrelid = \
                       ref_cl.oid AND ref_att.attnum = cols.ref_attnum AND NOT \
                       ref_att.attisdropped WHERE con.contype = 'f' AND con.conparentid = 0 AND \
                       src_nsp.nspname = 'catalog_e2e' AND src_cl.relname = 'items' ORDER BY \
                       con.conname, cols.src_attnum",
            min_rows: 0,
        },
        CatalogQuery {
            label:    "pg_get_indexdef",
            sql:      "SELECT COALESCE(pg_get_indexdef(1, 1, true), '') AS indexdef",
            min_rows: 1,
        },
    ]
}

pub fn required_information_schema_queries() -> &'static [CatalogQuery] {
    &[
        CatalogQuery {
            label:    "schemata",
            sql:      "SELECT schema_name FROM information_schema.schemata WHERE schema_name = \
                       'catalog_e2e'",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "tables",
            sql:      "SELECT table_name FROM information_schema.tables WHERE table_schema = \
                       'catalog_e2e' AND table_name = 'items'",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "columns_udt_name",
            sql:      "SELECT column_name, udt_name FROM information_schema.columns WHERE \
                       table_schema = 'catalog_e2e' AND table_name = 'items' ORDER BY \
                       ordinal_position",
            min_rows: 2,
        },
        CatalogQuery {
            label:    "routines",
            sql:      "SELECT routine_name, routine_type FROM information_schema.routines WHERE \
                       routine_schema = 'catalog_e2e' AND routine_name = 'ping'",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "parameters",
            sql:      "SELECT p.parameter_name FROM information_schema.parameters p JOIN \
                       information_schema.routines r ON p.specific_name = r.specific_name WHERE \
                       r.routine_schema = 'catalog_e2e' AND r.routine_name = 'ping' ORDER BY \
                       p.ordinal_position",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "routine_definition",
            sql:      "SELECT pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON \
                       p.pronamespace = n.oid WHERE n.nspname = 'catalog_e2e' AND p.proname = \
                       'ping'",
            min_rows: 1,
        },
        CatalogQuery {
            label:    "columns",
            sql:      "SELECT column_name FROM information_schema.columns WHERE table_schema = \
                       'catalog_e2e' AND table_name = 'items' ORDER BY ordinal_position",
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

/// SC-011: fixture table visible via pg_class and information_schema.tables over the same
/// connection.
pub async fn assert_pg_class_matches_system_tables(client: &Client) -> Result<(), String> {
    let pg_rows = simple_query_first_column(
        client,
        "SELECT relname FROM pg_catalog.pg_class WHERE relname = 'items'",
    )
    .await
    .map_err(|error| format!("pg_class parity query failed: {error}"))?;

    let system_rows = simple_query_first_column(
        client,
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'catalog_e2e' AND \
         table_name = 'items'",
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
        "SELECT a.attname FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON a.attrelid \
         = c.oid JOIN pg_catalog.pg_namespace n ON c.relnamespace = n.oid WHERE n.nspname = \
         'catalog_e2e' AND c.relname = 'items' AND a.attnum > 0",
    )
    .await
    .map_err(|error| format!("pg_attribute parity query failed: {error}"))?;

    let system_rows = simple_query_first_column(
        client,
        "SELECT column_name FROM information_schema.columns WHERE table_schema = 'catalog_e2e' \
         AND table_name = 'items'",
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
