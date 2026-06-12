//! Upload a FILE column with multipart SQL, read the row back, and download bytes.

use std::time::Duration;

use kalam_client::{AuthProvider, FileUpload, KalamLinkClient, QueryParam, TableId};

fn server_url() -> String {
    std::env::var("KALAMDB_SERVER_URL").unwrap_or_else(|_| "http://localhost:2900".to_string())
}

fn auth() -> AuthProvider {
    let password =
        std::env::var("KALAMDB_ROOT_PASSWORD").unwrap_or_else(|_| "kalamdb123".to_string());
    AuthProvider::system_user_auth(password)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = KalamLinkClient::builder()
        .base_url(server_url())
        .auth(auth())
        .timeout(Duration::from_secs(30))
        .build()?;

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let ns = format!("rust_file_ex_{suffix}");
    let table = "documents";
    let table_id = TableId::from_strings(&ns, table);

    client
        .execute_query(&format!("CREATE NAMESPACE {ns}"), None, None, None)
        .await?;

    client
        .execute_query(
            &format!(
                "CREATE TABLE {ns}.{table} (id TEXT PRIMARY KEY, name TEXT, attachment FILE)"
            ),
            None,
            None,
            None,
        )
        .await?;

    let content = b"Hello from the Rust SDK file-attachments example".to_vec();
    let files = vec![
        FileUpload::new("attachment", "note.txt", content.clone()).with_mime("text/plain"),
    ];

    client
        .execute_with_files(
            &format!(
                "INSERT INTO {ns}.{table} (id, name, attachment) VALUES ($1, 'Example doc', \
                 FILE(\"attachment\"))"
            ),
            files,
            Some(vec![QueryParam::from("doc1")]),
            None,
        )
        .await?;

    let query = client
        .execute_query(
            &format!("SELECT id, name, attachment FROM {ns}.{table} WHERE id = $1"),
            None,
            Some(vec![QueryParam::from("doc1")]),
            None,
        )
        .await?;

    let result = query
        .results
        .first()
        .ok_or("expected one query result")?;
    let rows = result.rows.as_ref().ok_or("expected positional rows")?;
    let row = rows.first().ok_or("expected one row")?;

    let bound = row[2]
        .as_bound_file(&table_id)
        .ok_or("attachment cell should parse as BoundFileRef")?;

    println!("row id: {}", row[0].as_text().unwrap_or("<null>"));
    println!("file name: {}", bound.name);
    println!("stored size: {} bytes", bound.file_ref().size);
    println!("relative url: {}", bound.relative_url());

    let download = client.download_bound_file(&bound, None).await?;
    let text = String::from_utf8_lossy(&download.bytes);
    println!("downloaded: {text:?}");
    println!("content-type: {:?}", download.content_type);

    if download.bytes != content {
        return Err("downloaded bytes did not match uploaded payload".into());
    }

    let _ = client
        .execute_query(&format!("DROP NAMESPACE {ns} CASCADE"), None, None, None)
        .await;

    client.disconnect().await;
    Ok(())
}
