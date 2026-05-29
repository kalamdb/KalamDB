//! `\export` and `\import` meta-command implementations.
//!
//! Calls the KalamDB REST API (`/v1/api/table-exports` and
//! `/v1/api/table-imports`) directly over HTTP, polls until the job
//! reaches a terminal state, and for exports downloads the resulting ZIP.

use std::time::{Duration, Instant};

use colored::Colorize;
use kalam_client::KalamLinkError;
use reqwest::multipart;

use super::CLISession;
use crate::error::{CLIError, Result};

/// Maximum time to wait for an export or import job to complete.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(600);
/// How long to wait between status-poll attempts.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Split a `namespace.table` reference into `(namespace, table_name)`.
fn parse_table_ref(table_ref: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = table_ref.splitn(2, '.').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(CLIError::ParseError(format!(
            "Expected <namespace>.<table>, got '{}'",
            table_ref
        )));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Build a reqwest `Client` with a 60-second timeout.
fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).build()
}

impl CLISession {
    /// Handle `\export <namespace.table> [--user-id ...] [--output ...]`
    pub(super) async fn cmd_export_table(
        &self,
        table_ref: &str,
        user_id: Option<&str>,
        output: Option<&str>,
    ) -> Result<()> {
        let (namespace, table_name) = parse_table_ref(table_ref)?;

        let client = http_client().map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!(
                "Failed to create HTTP client: {}",
                e
            )))
        })?;

        let base = self.api_base();

        // Build JSON body
        let mut body = serde_json::json!({
            "namespace_id": namespace,
            "table_name": table_name,
        });
        if let Some(uid) = user_id {
            body["user_id"] = serde_json::Value::String(uid.to_string());
        }

        println!("Starting export of {}.{}…", namespace, table_name);

        let mut req = client.post(format!("{}/v1/api/table-exports", base)).json(&body);

        if let Some(auth_header) = self.authorization_header_value() {
            req = req.header("Authorization", auth_header);
        }

        let resp = req.send().await.map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!(
                "Export request failed: {}",
                e
            )))
        })?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await.map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!(
                "Failed to read export response: {}",
                e
            )))
        })?;

        if !status.is_success() {
            return Err(CLIError::LinkError(KalamLinkError::ServerError {
                status_code: status.as_u16(),
                message: format!("Export failed: {}", json),
            }));
        }

        let job_id = json["job_id"]
            .as_str()
            .ok_or_else(|| CLIError::ParseError("Export response missing job_id".to_string()))?
            .to_string();

        println!("Export job started: {}", job_id.cyan());

        // Poll until terminal
        let download_url = self
            .poll_transfer_job_until_done(
                &client,
                &base,
                "/v1/api/table-exports",
                &job_id,
                "export",
            )
            .await?;

        // Download the ZIP
        let Some(url) = download_url else {
            return Err(CLIError::ParseError(
                "Export completed but no download_url was returned".to_string(),
            ));
        };

        let output_path = output
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}.{}-export.zip", namespace, table_name));

        self.download_export_zip(&client, &url, &output_path).await?;

        println!("{} Export saved to: {}", "✓".green().bold(), output_path.cyan());
        Ok(())
    }

    /// Handle `\import <namespace.table> <file.zip> [--user-id ...]`
    pub(super) async fn cmd_import_table(
        &self,
        table_ref: &str,
        zip_path: &str,
        user_id: Option<&str>,
    ) -> Result<()> {
        let (namespace, table_name) = parse_table_ref(table_ref)?;

        // Read the ZIP file
        let zip_bytes = tokio::fs::read(zip_path)
            .await
            .map_err(|e| CLIError::ParseError(format!("Cannot read '{}': {}", zip_path, e)))?;

        let client = http_client().map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!(
                "Failed to create HTTP client: {}",
                e
            )))
        })?;

        let base = self.api_base();

        println!("Importing {} into {}.{}…", zip_path, namespace, table_name);

        let mut form = multipart::Form::new()
            .text("namespace_id", namespace.clone())
            .text("table_name", table_name.clone());

        if let Some(uid) = user_id {
            form = form.text("user_id", uid.to_string());
        }

        let file_name = std::path::Path::new(zip_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("import.zip")
            .to_string();

        form = form.part(
            "file",
            multipart::Part::bytes(zip_bytes)
                .file_name(file_name)
                .mime_str("application/zip")
                .map_err(|e| CLIError::ParseError(format!("MIME error: {}", e)))?,
        );

        let mut req = client.post(format!("{}/v1/api/table-imports", base)).multipart(form);

        if let Some(auth_header) = self.authorization_header_value() {
            req = req.header("Authorization", auth_header);
        }

        let resp = req.send().await.map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!(
                "Import request failed: {}",
                e
            )))
        })?;

        let status = resp.status();
        let json: serde_json::Value = resp.json().await.map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!(
                "Failed to read import response: {}",
                e
            )))
        })?;

        if !status.is_success() {
            return Err(CLIError::LinkError(KalamLinkError::ServerError {
                status_code: status.as_u16(),
                message: format!("Import failed: {}", json),
            }));
        }

        let job_id = json["job_id"]
            .as_str()
            .ok_or_else(|| CLIError::ParseError("Import response missing job_id".to_string()))?
            .to_string();

        println!("Import job started: {}", job_id.cyan());

        self.poll_transfer_job_until_done(
            &client,
            &base,
            "/v1/api/table-imports",
            &job_id,
            "import",
        )
        .await?;

        println!(
            "{} Import of {}.{} completed successfully",
            "✓".green().bold(),
            namespace,
            table_name
        );
        Ok(())
    }

    /// Poll a table-transfer job (export or import) until it reaches a terminal state.
    ///
    /// Returns `Some(download_url)` for export jobs, `None` for import jobs.
    async fn poll_transfer_job_until_done(
        &self,
        client: &reqwest::Client,
        base: &str,
        endpoint_prefix: &str,
        job_id: &str,
        kind: &str,
    ) -> Result<Option<String>> {
        let deadline = Instant::now() + TRANSFER_TIMEOUT;
        let status_url = format!("{}{}/{}", base, endpoint_prefix, job_id);

        loop {
            if Instant::now() > deadline {
                return Err(CLIError::LinkError(KalamLinkError::TimeoutError(format!(
                    "{} job {} timed out after {}s",
                    kind,
                    job_id,
                    TRANSFER_TIMEOUT.as_secs()
                ))));
            }

            tokio::time::sleep(POLL_INTERVAL).await;

            let mut req = client.get(&status_url);
            if let Some(auth_header) = self.authorization_header_value() {
                req = req.header("Authorization", auth_header);
            }

            let resp = req.send().await.map_err(|e| {
                CLIError::LinkError(KalamLinkError::NetworkError(format!(
                    "Status poll failed: {}",
                    e
                )))
            })?;

            if !resp.status().is_success() {
                // Transient — keep polling
                continue;
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| {
                CLIError::LinkError(KalamLinkError::NetworkError(format!(
                    "Failed to read status response: {}",
                    e
                )))
            })?;

            let job_status = json["status"].as_str().unwrap_or("").to_lowercase();
            match job_status.as_str() {
                "completed" => {
                    let download_url = json["download_url"].as_str().map(|s| s.to_string());
                    return Ok(download_url);
                },
                "failed" => {
                    let msg = json["error_message"].as_str().unwrap_or("unknown error").to_string();
                    return Err(CLIError::LinkError(KalamLinkError::ServerError {
                        status_code: 500,
                        message: format!("{} job {} failed: {}", kind, job_id, msg),
                    }));
                },
                s => {
                    println!("  {} status: {}", kind, s);
                },
            }
        }
    }

    /// Download the export ZIP from `url` and write it to `output_path`.
    async fn download_export_zip(
        &self,
        client: &reqwest::Client,
        url: &str,
        output_path: &str,
    ) -> Result<()> {
        // Resolve relative URLs against base
        let full_url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("{}{}", self.api_base(), url)
        };

        let mut req = client.get(&full_url);
        if let Some(auth_header) = self.authorization_header_value() {
            req = req.header("Authorization", auth_header);
        }

        println!("Downloading ZIP from {}…", full_url);

        let resp = req.send().await.map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!("Download failed: {}", e)))
        })?;

        if !resp.status().is_success() {
            return Err(CLIError::LinkError(KalamLinkError::ServerError {
                status_code: resp.status().as_u16(),
                message: format!("Download failed with status: {}", resp.status()),
            }));
        }

        let bytes = resp.bytes().await.map_err(|e| {
            CLIError::LinkError(KalamLinkError::NetworkError(format!(
                "Failed to read download body: {}",
                e
            )))
        })?;

        tokio::fs::write(output_path, &bytes).await.map_err(|e| {
            CLIError::ParseError(format!("Failed to write '{}': {}", output_path, e))
        })?;

        Ok(())
    }
}
