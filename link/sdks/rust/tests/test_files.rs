//! FILE datatype integration tests — upload via multipart SQL and download via SDK.

mod common;

use kalam_client::{AuthProvider, FileUpload, KalamLinkClient, KalamLinkError, QueryParam, TableId};

fn create_client() -> Result<KalamLinkClient, KalamLinkError> {
    let token = common::root_access_token_blocking()
        .map_err(|e| KalamLinkError::ConfigurationError(e.to_string()))?;
    KalamLinkClient::builder()
        .base_url(common::server_url())
        .auth(AuthProvider::jwt_token(token))
        .build()
}

#[tokio::test]
#[ntest::timeout(60_000)]
async fn file_upload_select_and_download_roundtrip() {
    if !common::is_server_running().await {
        eprintln!("Skipping file upload/download test: server not running");
        return;
    }

    let client = create_client().expect("client");
    let ns = common::unique_ident("rust_file_sdk");
    let table = "documents";

    client
        .execute_query(&format!("CREATE NAMESPACE {ns}"), None, None, None)
        .await
        .expect("create namespace");

    client
        .execute_query(
            &format!("CREATE TABLE {ns}.{table} (id TEXT PRIMARY KEY, name TEXT, attachment FILE)"),
            None,
            None,
            None,
        )
        .await
        .expect("create table");

    let content = b"Rust SDK FILE datatype roundtrip payload".to_vec();
    let files =
        vec![FileUpload::new("myfile", "myfile.txt", content.clone()).with_mime("text/plain")];
    let insert_sql = format!(
        "INSERT INTO {ns}.{table} (id, name, attachment) VALUES ($1, 'My Document', \
         FILE(\"myfile\"))"
    );

    client
        .execute_with_files(&insert_sql, files, Some(vec![QueryParam::from("doc1")]), None)
        .await
        .expect("insert with file");

    let query = client
        .execute_query(
            &format!("SELECT id, name, attachment FROM {ns}.{table} WHERE id = $1"),
            None,
            Some(vec![QueryParam::from("doc1")]),
            None,
        )
        .await
        .expect("select file row");

    let result = &query.results[0];
    let rows = result.rows.as_ref().expect("positional rows");
    assert_eq!(rows.len(), 1, "expected one row");

    let attachment_schema = result
        .schema
        .iter()
        .find(|field| field.name == "attachment")
        .expect("attachment column in schema");
    assert_eq!(
        attachment_schema.data_type,
        kalam_client::KalamDataType::File,
        "attachment should be FILE type"
    );

    let table_id = TableId::from_strings(&ns, table);
    let attachment_cell = &rows[0][2];
    let file_ref = attachment_cell
        .as_bound_file(&table_id)
        .expect("FILE cell should parse as bound FileRef");
    assert_eq!(file_ref.file_ref().name, "myfile.txt");
    assert!(file_ref.file_ref().size > 0);
    assert!(!file_ref.file_ref().sha256.is_empty());

    let download = client.download_bound_file(&file_ref, None).await.expect("download file");

    assert_eq!(download.bytes, content);
    assert_eq!(
        download.content_type.as_deref(),
        Some("text/plain"),
        "download should preserve MIME type"
    );

    let url = file_ref.download_url(common::server_url());
    assert!(url.contains(&file_ref.file_ref().sub));
    assert!(url.ends_with(&file_ref.file_ref().stored_name()));

    let _ = client
        .execute_query(&format!("DROP NAMESPACE {ns} CASCADE"), None, None, None)
        .await;
}
