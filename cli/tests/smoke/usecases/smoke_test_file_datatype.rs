//! FILE datatype smoke test
//!
//! Tests the full lifecycle of FILE datatype:
//! 1. Create a table with FILE column
//! 2. Upload a file via multipart POST
//! 3. Query the data and verify FileRef JSON
//! 4. Download the file via GET endpoint
//! 5. Delete the row
//!
//! Run with: cargo test --test smoke smoke_test_file_datatype

use reqwest::Client;
use serde_json::Value;

use crate::common::{
    force_auto_test_server_url_async, generate_unique_namespace, get_access_token_for_url,
    get_available_server_urls, is_cluster_mode, is_leader_error,
    is_retryable_cluster_error_for_sql, shared_http_client, test_context,
};

#[tokio::test]
#[ntest::timeout(60000)]
async fn test_file_datatype_upload_and_download() {
    let ctx = test_context();
    let client = shared_http_client();
    let base_url = force_auto_test_server_url_async().await;
    let ns = generate_unique_namespace("file_test");
    let table = "documents";
    let token = get_access_token_for_url(&base_url, &ctx.username, &ctx.password)
        .await
        .expect("Failed to get access token");

    // 1. Create namespace
    let create_ns_sql = format!("CREATE NAMESPACE {}", ns);
    let ns_result =
        execute_sql_via_http_as_for_url(&client, &base_url, &token, &create_ns_sql).await;
    assert!(ns_result.is_ok(), "Failed to create namespace: {:?}", ns_result);

    // 2. Create table with FILE column
    let create_sql = format!(
        "CREATE TABLE {}.{} (id TEXT PRIMARY KEY, name TEXT, attachment FILE)",
        ns, table
    );
    let result = execute_sql_via_http_as_for_url(&client, &base_url, &token, &create_sql).await;
    assert!(result.is_ok(), "CREATE TABLE failed: {:?}", result);

    // 3. Upload file via multipart endpoint
    let test_content = b"This is the file content for testing FILE datatype!";
    let sql = format!(
        "INSERT INTO {}.{} (id, name, attachment) VALUES ('doc1', 'My Document', \
         FILE(\"myfile.txt\"))",
        ns, table
    );
    let boundary = "kalamdb-boundary";
    let mut body: Vec<u8> = Vec::new();

    push_line(&mut body, &format!("--{}", boundary));
    push_line(&mut body, "Content-Disposition: form-data; name=\"sql\"");
    push_line(&mut body, "");
    push_line(&mut body, &sql);

    push_line(&mut body, &format!("--{}", boundary));
    push_line(
        &mut body,
        "Content-Disposition: form-data; name=\"file:myfile.txt\"; \
         filename=\"test-attachment.txt\"",
    );
    push_line(&mut body, "Content-Type: text/plain");
    push_line(&mut body, "");
    body.extend_from_slice(test_content);
    body.extend_from_slice(b"\r\n");
    push_line(&mut body, &format!("--{}--", boundary));

    let content_type = format!("multipart/form-data; boundary={}", boundary);
    let request =
        build_multipart_sql_request(&client, &base_url, &token, &content_type, body.clone())
            .expect("Failed to build multipart request");
    let content_type_header = request
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type_header.contains(boundary),
        "Expected multipart boundary in Content-Type header, got: {}",
        content_type_header
    );

    let (upload_base_url, upload_status, body_text) =
        execute_multipart_sql_with_cluster_retry(&client, &base_url, &token, &content_type, &body)
            .await
            .expect("Failed to send multipart request");
    let body: Value = serde_json::from_str(&body_text)
        .unwrap_or_else(|e| panic!("Failed to parse response: {} (body: {})", e, body_text));

    assert!(
        body["status"] == "success",
        "File upload failed: status={}, body={}",
        upload_status,
        body
    );

    // 4. Query the inserted row and verify FileRef JSON
    let query_sql = format!("SELECT * FROM {}.{} WHERE id = 'doc1'", ns, table);
    let result = execute_sql_via_http_as_for_url(&client, &base_url, &token, &query_sql)
        .await
        .expect("Query failed");

    let schema = result["results"][0]["schema"].as_array().expect("No schema in query result");
    let attachment_schema = schema
        .iter()
        .find(|col| col["name"] == "attachment")
        .expect("attachment column missing in schema");
    match &attachment_schema["data_type"] {
        Value::String(s) => assert_eq!(s, "File", "Expected FILE data_type for attachment"),
        other => panic!("Unexpected attachment data_type format: {:?}", other),
    }

    let rows = result["results"][0]["rows"].as_array().expect("No rows");
    assert_eq!(rows.len(), 1, "Expected 1 row");

    let attachment_json: &str = rows[0][2].as_str().expect("attachment should be a string");
    let file_ref: Value =
        serde_json::from_str(attachment_json).expect("Failed to parse FileRef JSON");

    // Verify FileRef fields
    assert!(file_ref["id"].is_string(), "FileRef should have 'id'");
    assert!(file_ref["sub"].is_string(), "FileRef should have 'sub'");
    assert!(file_ref["name"].is_string(), "FileRef should have 'name'");
    assert!(file_ref["size"].is_number(), "FileRef should have 'size'");
    assert!(file_ref["mime"].is_string(), "FileRef should have 'mime'");
    assert!(file_ref["sha256"].is_string(), "FileRef should have 'sha256'");

    let subfolder = file_ref["sub"].as_str().unwrap();
    let stored_name = stored_filename_from_file_ref(&file_ref);

    // 5. Download the file
    let download_url =
        format!("{}/v1/files/{}/{}/{}/{}", upload_base_url, ns, table, subfolder, stored_name);

    let download_response = client
        .get(&download_url)
        .bearer_auth(&token)
        .send()
        .await
        .expect("Failed to download file");

    let download_status = download_response.status();
    assert!(download_status.is_success(), "File download failed: status={}", download_status);

    // 5.1 Verify headers
    let content_type = download_response
        .headers()
        .get("Content-Type")
        .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("text/plain"),
        "Expected text/plain content type, got: {}",
        content_type
    );

    let content_disposition = download_response
        .headers()
        .get("Content-Disposition")
        .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_disposition.contains(&stored_name),
        "Content-Disposition should include stored filename: {}",
        content_disposition
    );

    let downloaded_content = download_response.bytes().await.expect("Failed to read download body");
    assert_eq!(
        downloaded_content.as_ref(),
        test_content,
        "Downloaded content doesn't match uploaded content"
    );

    // 5.2 Download should require authentication
    let unauthorized_response = client
        .get(&download_url)
        .send()
        .await
        .expect("Failed to send unauthorized download request");
    assert_eq!(
        unauthorized_response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "Expected 401 for unauthenticated download"
    );

    // 5.3 Downloading a missing file should return 404
    let missing_url =
        format!("{}/v1/files/{}/{}/{}/{}", &base_url, ns, table, subfolder, "missing-file.bin");
    let missing_response = client
        .get(&missing_url)
        .bearer_auth(&token)
        .send()
        .await
        .expect("Failed to send missing-file download request");
    assert_eq!(
        missing_response.status(),
        reqwest::StatusCode::NOT_FOUND,
        "Expected 404 for missing file"
    );

    // 6. Delete the row
    let delete_sql = format!("DELETE FROM {}.{} WHERE id = 'doc1'", ns, table);
    let delete_result =
        execute_sql_via_http_as_for_url(&client, &base_url, &token, &delete_sql).await;
    assert!(delete_result.is_ok(), "DELETE failed: {:?}", delete_result);

    // 7. Cleanup
    let drop_sql = format!("DROP TABLE {}.{}", ns, table);
    let _ = execute_sql_via_http_as_for_url(&client, &base_url, &token, &drop_sql).await;
    let drop_ns_sql = format!("DROP NAMESPACE {}", ns);
    let _ = execute_sql_via_http_as_for_url(&client, &base_url, &token, &drop_ns_sql).await;

    println!("✅ FILE datatype smoke test passed!");
}

async fn execute_sql_via_http_as_for_url(
    client: &Client,
    base_url: &str,
    token: &str,
    sql: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let max_attempts = if is_cluster_mode() { 5 } else { 1 };
    let mut last_error = None;

    for attempt in 0..max_attempts {
        let urls = if is_cluster_mode() {
            get_available_server_urls()
        } else {
            vec![base_url.to_string()]
        };

        for url in urls {
            let response = match client
                .post(format!("{}/v1/api/sql", url))
                .bearer_auth(token)
                .json(&serde_json::json!({ "sql": sql }))
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let message = error.to_string();
                    if is_cluster_mode() && is_retryable_cluster_error_for_sql(sql, &message) {
                        last_error = Some(message);
                        continue;
                    }
                    return Err(error.into());
                },
            };

            let status = response.status();
            let body_text = response.text().await?;
            let parsed: Value = serde_json::from_str(&body_text)?;
            let response_status = parsed.get("status").and_then(Value::as_str).unwrap_or("");

            if status.is_success() && response_status.eq_ignore_ascii_case("success") {
                return Ok(parsed);
            }

            let message = json_error_message(&parsed)
                .unwrap_or_else(|| format!("SQL failed: status={}, body={}", status, body_text));
            if is_retryable_cluster_error_for_sql(sql, &message) {
                last_error = Some(message);
                continue;
            }

            return Err(message.into());
        }

        if attempt + 1 < max_attempts {
            tokio::time::sleep(std::time::Duration::from_millis(200 + attempt as u64 * 150)).await;
        }
    }

    Err(last_error
        .unwrap_or_else(|| "SQL request failed on all cluster nodes".to_string())
        .into())
}

fn json_error_message(parsed: &Value) -> Option<String> {
    let error = parsed.get("error")?;
    let message = error.get("message")?.as_str().unwrap_or("");
    let details = error.get("details").and_then(Value::as_str);
    if let Some(details) = details {
        return Some(format!("{} ({})", message, details));
    }
    Some(message.to_string())
}

fn build_multipart_sql_request(
    client: &Client,
    base_url: &str,
    token: &str,
    content_type: &str,
    body: Vec<u8>,
) -> Result<reqwest::Request, reqwest::Error> {
    client
        .post(format!("{}/v1/api/sql", base_url))
        .bearer_auth(token)
        .header("Accept", "application/json")
        .header("Content-Type", content_type)
        .body(body)
        .build()
}

async fn execute_multipart_sql_with_cluster_retry(
    client: &Client,
    initial_base_url: &str,
    token: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(String, reqwest::StatusCode, String), Box<dyn std::error::Error>> {
    let max_attempts = if is_cluster_mode() { 5 } else { 1 };
    let mut last_error = None;

    for attempt in 0..max_attempts {
        let mut urls = if is_cluster_mode() {
            get_available_server_urls()
        } else {
            vec![initial_base_url.to_string()]
        };
        if !urls.iter().any(|url| url == initial_base_url) {
            urls.push(initial_base_url.to_string());
        }

        for base_url in urls {
            let response = match client
                .execute(build_multipart_sql_request(
                    client,
                    &base_url,
                    token,
                    content_type,
                    body.to_vec(),
                )?)
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    let message = error.to_string();
                    if is_cluster_mode() && is_retryable_cluster_error_for_sql("", &message) {
                        last_error = Some(message);
                        continue;
                    }
                    return Err(error.into());
                },
            };

            let status = response.status();
            let response_content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let body_text = response.text().await?;

            if status.is_success() {
                return Ok((base_url, status, body_text));
            }

            let message = format!(
                "File upload failed: status={}, content-type={}, body={}",
                status, response_content_type, body_text
            );
            if is_cluster_mode() && is_leader_error(&message) {
                last_error = Some(message);
                break;
            }

            return Err(message.into());
        }

        if attempt + 1 < max_attempts {
            tokio::time::sleep(std::time::Duration::from_millis(250 + attempt as u64 * 150)).await;
        }
    }

    Err(last_error
        .unwrap_or_else(|| "File upload failed on all cluster nodes".to_string())
        .into())
}

fn push_line(body: &mut Vec<u8>, line: &str) {
    body.extend_from_slice(line.as_bytes());
    body.extend_from_slice(b"\r\n");
}

fn stored_filename_from_file_ref(file_ref: &Value) -> String {
    let file_id = file_ref["id"].as_str().unwrap_or_default();
    let name = file_ref["name"].as_str().unwrap_or_default();

    let ext = extract_extension(name);
    let sanitized = sanitize_filename(name);

    if sanitized.is_empty() {
        format!("{}.{}", file_id, ext)
    } else {
        format!("{}-{}.{}", file_id, sanitized, ext)
    }
}

fn sanitize_filename(name: &str) -> String {
    let name_without_ext = name.rsplit_once('.').map(|(n, _)| n).unwrap_or(name);

    let sanitized: String = name_without_ext
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '_' || c == '-' {
                Some('-')
            } else {
                None
            }
        })
        .take(50)
        .collect();

    let mut result = String::with_capacity(sanitized.len());
    let mut last_was_dash = true;
    for c in sanitized.chars() {
        if c == '-' {
            if !last_was_dash {
                result.push(c);
            }
            last_was_dash = true;
        } else {
            result.push(c);
            last_was_dash = false;
        }
    }
    result.trim_end_matches('-').to_string()
}

fn extract_extension(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, ext)| {
            let ext_lower = ext.to_ascii_lowercase();
            if ext_lower.len() <= 10 && ext_lower.chars().all(|c| c.is_ascii_alphanumeric()) {
                ext_lower
            } else {
                "bin".to_string()
            }
        })
        .unwrap_or_else(|| "bin".to_string())
}
