//! Timestamp unit consistency checks over the HTTP SQL API.

use kalam_client::{models::ResponseStatus, KalamCellValue};
use serde_json::Value as JsonValue;

use super::test_support::{
    auth_helper::create_user_auth_header_default,
    consolidated_helpers::{unique_namespace, unique_table},
};

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_current_timestamp_functions_return_microseconds() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let before_micros = chrono::Utc::now().timestamp_micros() - 5_000_000;

    let response = server.execute_sql("SELECT CURRENT_TIMESTAMP(), now()").await?;
    let after_micros = chrono::Utc::now().timestamp_micros() + 5_000_000;

    anyhow::ensure!(
        response.status == ResponseStatus::Success,
        "timestamp function query failed: {:?}",
        response.error
    );

    let row = response
        .first_row_as_map()
        .ok_or_else(|| anyhow::anyhow!("timestamp function query returned no rows"))?;

    let current_timestamp = cell_as_i64(
        row.get("current_timestamp()")
            .ok_or_else(|| anyhow::anyhow!("missing current_timestamp() column"))?,
    )?;
    let now =
        cell_as_i64(row.get("now()").ok_or_else(|| anyhow::anyhow!("missing now() column"))?)?;

    assert_microsecond_epoch("current_timestamp()", current_timestamp, before_micros, after_micros);
    assert_microsecond_epoch("now()", now, before_micros, after_micros);

    Ok(())
}

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_timestamp_column_defaults_return_microseconds() -> anyhow::Result<()> {
    let server = super::test_support::http_server::get_global_server().await;
    let namespace = unique_namespace("ts_units");
    let user = unique_table("user_ts_units");
    let auth = create_user_auth_header_default(server, &user).await?;

    let response = server
        .execute_sql(&format!("CREATE NAMESPACE IF NOT EXISTS {namespace}"))
        .await?;
    anyhow::ensure!(response.status == ResponseStatus::Success);

    let response = server
        .execute_sql_with_auth(
            &format!(
                "CREATE TABLE {namespace}.events (id INT PRIMARY KEY, created TIMESTAMP DEFAULT \
                 CURRENT_TIMESTAMP) WITH (TYPE='USER', STORAGE_ID='local')"
            ),
            &auth,
        )
        .await?;
    anyhow::ensure!(
        response.status == ResponseStatus::Success,
        "create table failed: {:?}",
        response.error
    );

    let before_micros = chrono::Utc::now().timestamp_micros() - 5_000_000;
    let response = server
        .execute_sql_with_auth(&format!("INSERT INTO {namespace}.events (id) VALUES (1)"), &auth)
        .await?;
    let after_insert_micros = chrono::Utc::now().timestamp_micros() + 5_000_000;
    anyhow::ensure!(
        response.status == ResponseStatus::Success,
        "insert failed: {:?}",
        response.error
    );

    let response = server
        .execute_sql_with_auth(
            &format!("SELECT created FROM {namespace}.events WHERE id = 1"),
            &auth,
        )
        .await?;
    anyhow::ensure!(
        response.status == ResponseStatus::Success,
        "select failed: {:?}",
        response.error
    );

    let row = response
        .first_row_as_map()
        .ok_or_else(|| anyhow::anyhow!("timestamp column query returned no rows"))?;
    let created =
        cell_as_i64(row.get("created").ok_or_else(|| anyhow::anyhow!("missing created column"))?)?;

    assert_microsecond_epoch("created", created, before_micros, after_insert_micros);

    Ok(())
}

fn cell_as_i64(cell: &KalamCellValue) -> anyhow::Result<i64> {
    match cell.inner() {
        JsonValue::Number(number) => number
            .as_i64()
            .ok_or_else(|| anyhow::anyhow!("numeric cell is not an i64: {number}")),
        JsonValue::String(value) => value
            .parse::<i64>()
            .map_err(|error| anyhow::anyhow!("string cell is not an i64: {error}")),
        value => Err(anyhow::anyhow!("cell is not numeric: {value}")),
    }
}

fn assert_microsecond_epoch(label: &str, value: i64, lower_bound: i64, upper_bound: i64) {
    assert!(
        (1_000_000_000_000_000..10_000_000_000_000_000).contains(&value),
        "{label} should be a microsecond epoch value, got {value}"
    );
    assert!(
        value >= lower_bound && value <= upper_bound,
        "{label} should be near the current time in microseconds, got {value}, expected between \
         {lower_bound} and {upper_bound}"
    );
}
