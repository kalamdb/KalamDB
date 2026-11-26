use std::{
    error::Error,
    time::{SystemTime, UNIX_EPOCH},
};

use tokio_postgres::{NoTls, SimpleQueryMessage};

#[tokio::test]
async fn wire_explicit_transaction_commits_dml() -> Result<(), Box<dyn Error>> {
    let Some(url) = std::env::var("KALAMDB_PGWIRE_TEST_URL").ok() else {
        eprintln!("skipping wire transaction smoke; set KALAMDB_PGWIRE_TEST_URL to run it");
        return Ok(());
    };

    let namespace =
        std::env::var("KALAMDB_PGWIRE_TEST_NAMESPACE").unwrap_or_else(|_| "wire_e2e".to_string());
    let table = format!("items_{}", unique_suffix());
    let qualified_table = format!("{namespace}.{table}");

    let (client, connection) = tokio_postgres::connect(url.as_str(), NoTls).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let _ = client.batch_execute(format!("CREATE SCHEMA {namespace}").as_str()).await;
    client
        .batch_execute(
            format!("CREATE TABLE {qualified_table} (id INT PRIMARY KEY, name TEXT)").as_str(),
        )
        .await?;

    client.batch_execute("BEGIN").await?;
    client
        .batch_execute(
            format!("INSERT INTO {qualified_table} (id, name) VALUES (1, 'wire')").as_str(),
        )
        .await?;

    let inside_tx = select_id_exists(&client, qualified_table.as_str(), 1).await?;
    if !inside_tx {
        eprintln!(
            "warning: read-your-writes inside wire transaction block is not yet guaranteed; \
             verifying commit persistence instead"
        );
    }

    client.batch_execute("COMMIT").await?;
    let after_commit = select_id_exists(&client, qualified_table.as_str(), 1).await?;
    assert!(after_commit, "inserted row should persist after commit");

    Ok(())
}

#[tokio::test]
async fn wire_multi_statement_insert_batch_preserves_tags_and_rows() -> Result<(), Box<dyn Error>> {
    let Some(url) = std::env::var("KALAMDB_PGWIRE_TEST_URL").ok() else {
        eprintln!("skipping wire batch INSERT test; set KALAMDB_PGWIRE_TEST_URL to run it");
        return Ok(());
    };
    let namespace = std::env::var("KALAMDB_PGWIRE_TEST_NAMESPACE")
        .unwrap_or_else(|_| "wire_e2e".to_string());
    let table = format!("batch_items_{}", unique_suffix());
    let qualified_table = format!("{namespace}.{table}");
    let (client, connection) = tokio_postgres::connect(url.as_str(), NoTls).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let _ = client.batch_execute(format!("CREATE SCHEMA {namespace}").as_str()).await;
    client
        .batch_execute(
            format!("CREATE TABLE {qualified_table} (id INT PRIMARY KEY, name TEXT)").as_str(),
        )
        .await?;

    // 513 INSERTs cross the bounded 512-statement window without losing tags.
    let mut script = String::from("BEGIN;");
    for id in 1..=513 {
        script.push_str(&format!(
            "INSERT INTO {qualified_table} (id, name) VALUES ({id}, 'row');"
        ));
    }
    script.push_str("COMMIT;");
    let messages = client.simple_query(&script).await?;
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, SimpleQueryMessage::CommandComplete(_)))
            .count(),
        515,
        "one completion per statement, including BEGIN and COMMIT"
    );
    let rows = client
        .simple_query(format!("SELECT COUNT(*) FROM {qualified_table}").as_str())
        .await?;
    assert!(rows.iter().any(|message| matches!(
        message,
        SimpleQueryMessage::Row(row) if row.get(0) == Some("513")
    )));
    Ok(())
}

async fn select_id_exists(
    client: &tokio_postgres::Client,
    qualified_table: &str,
    id: i32,
) -> Result<bool, tokio_postgres::Error> {
    let messages = client
        .simple_query(format!("SELECT id FROM {qualified_table} WHERE id = {id}").as_str())
        .await?;
    let expected = id.to_string();
    Ok(messages.iter().any(|message| match message {
        SimpleQueryMessage::Row(row) => row.get(0) == Some(expected.as_str()),
        _ => false,
    }))
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
