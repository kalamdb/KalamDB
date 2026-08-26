//! US9 client catalog validation over PostgreSQL wire (SC-011, SC-012).
//!
//! Connects with `tokio-postgres` to a running KalamDB wire listener and asserts
//! `pg_catalog` shims and `information_schema` tables return real metadata aligned
//! with `system.*`.
//!
//! Spec: `specs/033-unified-backend-pgwire/validation/us9-client-catalog.md`
//!
//! Run (when wire + shims are implemented):
//!   cd backend && cargo test --test pgwire_catalog --features e2e-tests

mod catalog_checks;
mod jdbc;

use std::env;

use catalog_checks::{
    assert_admin_pg_stat_activity, assert_all_catalog_queries,
    assert_pg_attribute_matches_system_columns, assert_pg_class_matches_system_tables,
    FIXTURE_NAMESPACE, FIXTURE_TABLE,
};
use tokio_postgres::{Config, NoTls};

fn pgwire_connect_config() -> Option<Config> {
    let host = env::var("KALAMDB_PGWIRE_HOST").ok()?;
    let port: u16 = env::var("KALAMDB_PGWIRE_PORT").ok()?.parse().ok()?;
    let user = env::var("KALAMDB_PGWIRE_USER").unwrap_or_else(|_| "root".to_string());
    let password = env::var("KALAMDB_PGWIRE_PASSWORD").unwrap_or_default();

    let mut config = Config::new();
    config.host(&host).port(port).user(&user).password(&password);
    Some(config)
}

async fn connect_pgwire() -> Result<tokio_postgres::Client, String> {
    let config = pgwire_connect_config().ok_or_else(|| {
        "Set KALAMDB_PGWIRE_HOST and KALAMDB_PGWIRE_PORT to run against a live server".to_string()
    })?;

    let (client, connection) =
        config.connect(NoTls).await.map_err(|e| format!("wire connect failed: {e}"))?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("pgwire connection error: {e}");
        }
    });

    Ok(client)
}

async fn seed_fixture_metadata(client: &tokio_postgres::Client) -> Result<(), String> {
    let ns_sql = format!("CREATE NAMESPACE IF NOT EXISTS {FIXTURE_NAMESPACE}");
    client
        .batch_execute(&ns_sql)
        .await
        .map_err(|e| format!("CREATE NAMESPACE fixture failed: {e}"))?;

    let table_sql = format!(
        "CREATE TABLE IF NOT EXISTS {FIXTURE_NAMESPACE}.{FIXTURE_TABLE} (id INT PRIMARY KEY, name \
         TEXT)"
    );
    client
        .batch_execute(&table_sql)
        .await
        .map_err(|e| format!("CREATE TABLE fixture failed: {e}"))?;

    Ok(())
}

/// Primary US9 acceptance test — wire client, all required catalog surfaces, SC-011 parity.
///
/// Requires: running server with `postgres_wire.enabled = true`.
/// Future: replace env-based connect with embedded test server (extend `http_server` harness).
#[tokio::test]
#[ignore = "requires postgres wire listener (US2) and pg_catalog shims (US9); see \
            validation/us9-client-catalog.md"]
async fn wire_client_catalog_returns_data_and_matches_system_views() {
    let client = connect_pgwire().await.expect("connect");
    seed_fixture_metadata(&client).await.expect("seed fixture");

    assert_all_catalog_queries(&client).await.expect("required catalog queries");

    assert_pg_class_matches_system_tables(&client)
        .await
        .expect("pg_class vs information_schema.tables");

    assert_pg_attribute_matches_system_columns(&client)
        .await
        .expect("pg_attribute vs information_schema.columns");

    assert_admin_pg_stat_activity(&client).await.expect("admin pg_stat_activity");
}

#[tokio::test]
#[ignore = "requires postgres wire listener (US2) and pg_catalog shims (US9)"]
async fn wire_client_catalog_disabled_pg_catalog_not_queryable() {
    let client = connect_pgwire().await.expect("connect");

    let result = client.query("SELECT relname FROM pg_catalog.pg_class LIMIT 1", &[]).await;

    match result {
        Ok(rows) if !rows.is_empty() => {
            panic!(
                "expected pg_catalog disabled server to fail or return no metadata, got {} rows",
                rows.len()
            );
        },
        Ok(_) => {},
        Err(_) => {},
    }
}
