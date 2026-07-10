use std::error::Error;

use tokio_postgres::{NoTls, SimpleQueryMessage};

#[tokio::test]
async fn wire_login_and_select_one_smoke() -> Result<(), Box<dyn Error>> {
    let Some(url) = std::env::var("KALAMDB_PGWIRE_TEST_URL").ok() else {
        eprintln!("skipping wire smoke; set KALAMDB_PGWIRE_TEST_URL to run it");
        return Ok(());
    };

    let (client, connection) = tokio_postgres::connect(url.as_str(), NoTls).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let messages = client.simple_query("SELECT 1").await?;
    let saw_one = messages.iter().any(|message| match message {
        SimpleQueryMessage::Row(row) => row.get(0) == Some("1"),
        _ => false,
    });

    assert!(saw_one, "SELECT 1 should return one row with value 1");
    Ok(())
}
