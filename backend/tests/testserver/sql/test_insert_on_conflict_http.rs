//! PostgreSQL-style INSERT ... ON CONFLICT ... DO UPDATE over the real HTTP SQL API.

use std::collections::HashMap;

use kalam_client::models::{KalamCellValue, ResponseStatus};

use super::test_support::consolidated_helpers::unique_namespace;

async fn create_items_table(
    server: &super::test_support::http_server::HttpTestServer,
    namespace: &str,
) -> anyhow::Result<()> {
    let resp = server
        .execute_sql(&format!("CREATE NAMESPACE IF NOT EXISTS {}", namespace))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success);

    let resp = server
        .execute_sql(&format!(
            "CREATE TABLE {}.items (id BIGINT PRIMARY KEY, name TEXT) WITH (TYPE='SHARED', \
             STORAGE_ID='local')",
            namespace
        ))
        .await?;
    anyhow::ensure!(
        resp.status == ResponseStatus::Success,
        "CREATE items table failed: {:?}",
        resp.error
    );

    Ok(())
}

fn row_i64(row: &HashMap<String, KalamCellValue>, column_name: &str) -> i64 {
    row.get(column_name)
        .and_then(|value| {
            value.as_i64().or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or_else(|| panic!("expected integer column '{}'", column_name))
}

fn row_text<'a>(row: &'a HashMap<String, KalamCellValue>, column_name: &str) -> &'a str {
    row.get(column_name)
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| panic!("expected text column '{}'", column_name))
}

#[tokio::test]
#[ntest::timeout(5000)]
async fn test_insert_on_conflict_do_update_returning_over_http() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("insert_conflict_http_returning");

    create_items_table(server, &namespace).await?;

    let resp = server
        .execute_sql(&format!("INSERT INTO {}.items (id, name) VALUES (1, 'alpha')", namespace))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "seed insert failed: {:?}", resp.error);

    let resp = server
        .execute_sql(&format!(
            "INSERT INTO {}.items (id, name) VALUES (1, 'beta') ON CONFLICT (id) DO UPDATE SET \
             name = EXCLUDED.name RETURNING id, name",
            namespace
        ))
        .await?;
    anyhow::ensure!(
        resp.status == ResponseStatus::Success,
        "upsert returning failed: {:?}",
        resp.error
    );

    let rows = resp.rows_as_maps();
    anyhow::ensure!(rows.len() == 1, "expected one returned row, got {}", rows.len());
    anyhow::ensure!(row_i64(&rows[0], "id") == 1);
    anyhow::ensure!(row_text(&rows[0], "name") == "beta");

    Ok(())
}

#[tokio::test]
#[ntest::timeout(5000)]
async fn test_insert_on_conflict_do_update_transaction_over_http() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("insert_conflict_http_tx");

    create_items_table(server, &namespace).await?;

    let resp = server
        .execute_sql(&format!(
            "BEGIN; INSERT INTO {}.items (id, name) VALUES (2, 'alpha'); INSERT INTO {}.items \
             (id, name) VALUES (2, 'gamma') ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name \
             RETURNING id, name; COMMIT;",
            namespace, namespace
        ))
        .await?;
    anyhow::ensure!(
        resp.status == ResponseStatus::Success,
        "transactional upsert failed: {:?}",
        resp.error
    );

    let returning_rows =
        resp.results.iter().flat_map(|result| result.rows_as_maps()).collect::<Vec<_>>();
    anyhow::ensure!(
        returning_rows.len() == 1,
        "expected one returned transaction row, got {}",
        returning_rows.len()
    );
    anyhow::ensure!(row_i64(&returning_rows[0], "id") == 2);
    anyhow::ensure!(row_text(&returning_rows[0], "name") == "gamma");

    let resp = server
        .execute_sql(&format!("SELECT id, name FROM {}.items WHERE id = 2", namespace))
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "select failed: {:?}", resp.error);
    let rows = resp.rows_as_maps();
    anyhow::ensure!(rows.len() == 1, "expected persisted row, got {}", rows.len());
    anyhow::ensure!(row_i64(&rows[0], "id") == 2);
    anyhow::ensure!(row_text(&rows[0], "name") == "gamma");

    Ok(())
}

#[tokio::test]
#[ntest::timeout(5000)]
async fn test_parameterized_insert_returning_over_http() -> anyhow::Result<()> {
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("insert_conflict_http_param_returning");

    create_items_table(server, &namespace).await?;

    let resp = server
        .execute_sql_with_params(
            &format!(
                "INSERT INTO {}.items (id, name) VALUES ($1, $2) RETURNING id, name",
                namespace
            ),
            vec![serde_json::json!(1), serde_json::json!("alpha")],
        )
        .await?;
    anyhow::ensure!(
        resp.status == ResponseStatus::Success,
        "parameterized insert returning failed: {:?}",
        resp.error
    );

    let rows = resp.rows_as_maps();
    anyhow::ensure!(rows.len() == 1, "expected one returned row, got {}", rows.len());
    anyhow::ensure!(row_i64(&rows[0], "id") == 1);
    anyhow::ensure!(row_text(&rows[0], "name") == "alpha");

    Ok(())
}

#[tokio::test]
#[ntest::timeout(5000)]
async fn test_parameterized_insert_on_conflict_do_update_returning_over_http() -> anyhow::Result<()>
{
    let _guard = super::test_support::http_server::acquire_test_lock().await;
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("insert_conflict_http_param_upsert");

    create_items_table(server, &namespace).await?;

    let resp = server
        .execute_sql_with_params(
            &format!("INSERT INTO {}.items (id, name) VALUES ($1, $2)", namespace),
            vec![serde_json::json!(1), serde_json::json!("alpha")],
        )
        .await?;
    anyhow::ensure!(resp.status == ResponseStatus::Success, "seed insert failed: {:?}", resp.error);

    let resp = server
        .execute_sql_with_params(
            &format!(
                "INSERT INTO {}.items (id, name) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET \
                 name = EXCLUDED.name RETURNING id, name",
                namespace
            ),
            vec![serde_json::json!(1), serde_json::json!("beta")],
        )
        .await?;
    anyhow::ensure!(
        resp.status == ResponseStatus::Success,
        "parameterized upsert returning failed: {:?}",
        resp.error
    );

    let rows = resp.rows_as_maps();
    anyhow::ensure!(rows.len() == 1, "expected one returned row, got {}", rows.len());
    anyhow::ensure!(row_i64(&rows[0], "id") == 1);
    anyhow::ensure!(row_text(&rows[0], "name") == "beta");

    Ok(())
}
