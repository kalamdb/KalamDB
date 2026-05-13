//! SQL query execution via HTTP.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Instant,
};

#[cfg(feature = "file-uploads")]
use bytes::Bytes;
#[cfg(feature = "file-uploads")]
use http_body::Frame;
#[cfg(feature = "file-uploads")]
use http_body_util::StreamBody;
use log::{debug, warn};
#[cfg(feature = "file-uploads")]
use reqwest::multipart::{Form, Part};
use serde::Serialize;

use crate::{
    auth::AuthProvider,
    error::{KalamLinkError, Result},
    models::{QueryResponse, UploadProgress},
};

/// Progress callback for multipart file uploads.
pub type UploadProgressCallback = Arc<dyn Fn(UploadProgress) + Send + Sync>;

#[cfg(feature = "file-uploads")]
fn build_progress_stream(
    data: Bytes,
    file_name: Arc<str>,
    file_index: usize,
    total_files: usize,
    progress_cb: UploadProgressCallback,
) -> impl futures_util::Stream<Item = std::result::Result<Frame<Bytes>, std::io::Error>> + Send + 'static
{
    let chunk_size = 64 * 1024;
    futures_util::stream::unfold(0usize, move |offset| {
        let data = data.clone();
        let progress_cb = progress_cb.clone();
        let file_name = Arc::clone(&file_name);
        async move {
            if offset >= data.len() {
                return None;
            }

            let end = (offset + chunk_size).min(data.len());
            let chunk = data.slice(offset..end);
            let total_bytes = data.len() as u64;
            let bytes_sent = end as u64;
            let percent = if total_bytes == 0 {
                100.0
            } else {
                (bytes_sent as f64 / total_bytes as f64) * 100.0
            };

            (progress_cb)(UploadProgress {
                file_index,
                total_files,
                file_name: file_name.to_string(),
                bytes_sent,
                total_bytes,
                percent,
            });

            Some((Ok(Frame::data(chunk)), end))
        }
    })
}

#[cfg(feature = "file-uploads")]
#[derive(Clone)]
struct MultipartUploadFile {
    placeholder_name: String,
    filename: String,
    data: Bytes,
    mime_type: Option<String>,
}

#[cfg(feature = "file-uploads")]
impl From<(String, String, Vec<u8>, Option<String>)> for MultipartUploadFile {
    fn from(
        (placeholder_name, filename, data, mime_type): (String, String, Vec<u8>, Option<String>),
    ) -> Self {
        Self {
            placeholder_name,
            filename,
            data: Bytes::from(data),
            mime_type,
        }
    }
}

#[derive(Serialize)]
struct BorrowedQueryRequest<'a> {
    sql: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<&'a [serde_json::Value]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    namespace_id: Option<&'a str>,
}

/// Async callback that resolves fresh [`AuthProvider`] credentials.
///
/// Called by the executor when a query requires a login exchange or returns
/// `TOKEN_EXPIRED`.
/// Implementations should obtain a fresh JWT (e.g. via login or dynamic
/// auth provider) and return it.
pub type AuthRefreshCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<AuthProvider>> + Send>> + Send + Sync>;

/// Handles SQL query execution via HTTP.
#[derive(Clone)]
pub struct QueryExecutor {
    sql_url: String,
    http_client: reqwest::Client,
    auth: Arc<Mutex<AuthProvider>>,
    max_retries: u32,
    auth_refresher: Option<AuthRefreshCallback>,
}

impl QueryExecutor {
    pub(crate) fn new(
        base_url: String,
        http_client: reqwest::Client,
        auth: AuthProvider,
        max_retries: u32,
    ) -> Self {
        Self {
            sql_url: format!("{}/v1/api/sql", base_url),
            http_client,
            auth: Arc::new(Mutex::new(auth)),
            max_retries,
            auth_refresher: None,
        }
    }

    pub(crate) fn set_auth(&self, auth: AuthProvider) {
        *self.auth.lock().unwrap() = auth;
    }

    pub(crate) fn set_auth_refresher(&mut self, refresher: AuthRefreshCallback) {
        self.auth_refresher = Some(refresher);
    }

    fn validate_request_auth(auth: &AuthProvider) -> Result<()> {
        if matches!(auth, AuthProvider::BasicAuth(_, _)) {
            return Err(KalamLinkError::AuthenticationError(
                "User/password credentials can only be used with /v1/api/auth/login; exchange \
                 them for a JWT before executing SQL requests."
                    .to_string(),
            ));
        }

        Ok(())
    }

    async fn ensure_request_auth(&self) -> Result<AuthProvider> {
        let current_auth = self.auth.lock().unwrap().clone();
        if !matches!(current_auth, AuthProvider::BasicAuth(_, _)) {
            Self::validate_request_auth(&current_auth)?;
            return Ok(current_auth);
        }

        let refresher = self.auth_refresher.as_ref().ok_or_else(|| {
            KalamLinkError::AuthenticationError(
                "User/password credentials require a login exchange before executing SQL requests."
                    .to_string(),
            )
        })?;

        let refreshed_auth = refresher().await?;
        Self::validate_request_auth(&refreshed_auth)?;
        *self.auth.lock().unwrap() = refreshed_auth.clone();
        Ok(refreshed_auth)
    }

    fn is_retry_safe_sql(sql: &str) -> bool {
        let Some(keyword) = Self::first_keyword(sql) else {
            return false;
        };

        keyword.eq_ignore_ascii_case("SELECT")
            || keyword.eq_ignore_ascii_case("SHOW")
            || keyword.eq_ignore_ascii_case("DESCRIBE")
            || keyword.eq_ignore_ascii_case("EXPLAIN")
    }

    fn first_keyword(sql: &str) -> Option<&str> {
        let bytes = sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Skip whitespace
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                return None;
            }

            // Skip line comments: -- ...\n
            if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }

            // Skip block comments: /* ... */
            if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < bytes.len() {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }

            // Read keyword
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            if start == i {
                return None;
            }
            return Some(&sql[start..i]);
        }

        None
    }

    /// Execute a SQL query with optional parameters and namespace.
    pub async fn execute(
        &self,
        sql: &str,
        files: Option<Vec<(String, String, Vec<u8>, Option<String>)>>,
        params: Option<Vec<serde_json::Value>>,
        namespace_id: Option<String>,
    ) -> Result<QueryResponse> {
        self.execute_with_progress(sql, files, params, namespace_id, None).await
    }

    /// Execute a SQL query with optional parameters and namespace, with upload progress callback.
    pub async fn execute_with_progress(
        &self,
        sql: &str,
        files: Option<Vec<(String, String, Vec<u8>, Option<String>)>>,
        params: Option<Vec<serde_json::Value>>,
        namespace_id: Option<String>,
        progress: Option<UploadProgressCallback>,
    ) -> Result<QueryResponse> {
        self.execute_with_progress_ref(sql, files, params, namespace_id.as_deref(), progress)
            .await
    }

    pub(crate) async fn execute_with_progress_ref(
        &self,
        sql: &str,
        files: Option<Vec<(String, String, Vec<u8>, Option<String>)>>,
        params: Option<Vec<serde_json::Value>>,
        namespace_id: Option<&str>,
        progress: Option<UploadProgressCallback>,
    ) -> Result<QueryResponse> {
        let has_files = files.as_ref().map(|f| !f.is_empty()).unwrap_or(false);

        #[cfg(not(feature = "file-uploads"))]
        if has_files {
            return Err(KalamLinkError::ConfigurationError(
                "This SDK build does not include file upload support. Rebuild with the \
                 `file-uploads` feature."
                    .to_string(),
            ));
        }

        #[cfg(feature = "file-uploads")]
        if has_files {
            let files = files
                .unwrap_or_default()
                .into_iter()
                .map(MultipartUploadFile::from)
                .collect::<Vec<_>>();
            let auth_snapshot = self.ensure_request_auth().await?;
            let mut result = self
                .execute_multipart_once(
                    &self.sql_url,
                    &auth_snapshot,
                    sql,
                    &files,
                    params.as_ref(),
                    namespace_id.as_deref(),
                    progress.clone(),
                )
                .await;

            if let Some(leader_sql_url) = Self::multipart_leader_retry_url(&result, &self.sql_url) {
                warn!(
                    "[LINK_HTTP] Leader redirect for multipart request - retrying against {}",
                    leader_sql_url
                );
                result = self
                    .execute_multipart_once(
                        &leader_sql_url,
                        &auth_snapshot,
                        sql,
                        &files,
                        params.as_ref(),
                        namespace_id.as_deref(),
                        progress.clone(),
                    )
                    .await;
            }

            let result = result?;

            // Auto-refresh on TOKEN_EXPIRED for subsequent requests.
            if result.is_token_expired() {
                if let Some(refresher) = &self.auth_refresher {
                    warn!(
                        "[LINK_HTTP] TOKEN_EXPIRED on multipart request — refreshing auth for \
                         subsequent requests"
                    );
                    if let Ok(new_auth) = refresher().await {
                        if Self::validate_request_auth(&new_auth).is_ok() {
                            *self.auth.lock().unwrap() = new_auth;
                        }
                    }
                }
            }

            return Ok(result);
        }

        let _ = progress;

        let request = BorrowedQueryRequest {
            sql,
            params: params.as_deref(),
            namespace_id,
        };

        let mut retries: u32 = 0;
        let max_retries = self.max_retries;
        let retry_safe_sql = Self::is_retry_safe_sql(sql);

        let sql_preview = if sql.len() > 80 {
            format!("{}...", &sql[..80])
        } else {
            sql.to_string()
        };
        debug!(
            "[LINK_QUERY] Starting query: \"{}\" (len={})",
            sql_preview.replace('\n', " "),
            sql.len()
        );

        let overall_start = Instant::now();

        loop {
            let auth_snapshot = self.ensure_request_auth().await?;
            let mut req_builder = self.http_client.post(&self.sql_url).json(&request);
            req_builder = auth_snapshot.apply_to_request(req_builder)?;

            let attempt_start = Instant::now();
            debug!(
                "[LINK_HTTP] Sending POST to {} (attempt {}/{})",
                self.sql_url,
                retries + 1,
                max_retries + 1
            );

            match req_builder.send().await {
                Ok(response) => {
                    let http_duration_ms = attempt_start.elapsed().as_millis();
                    debug!(
                        "[LINK_HTTP] Response received: status={} duration_ms={}",
                        response.status(),
                        http_duration_ms
                    );

                    let result = Self::handle_response(response, sql).await;

                    // Auto-refresh on TOKEN_EXPIRED: get fresh auth and retry once.
                    if let Ok(ref resp) = result {
                        if resp.is_token_expired() {
                            if let Some(refresher) = &self.auth_refresher {
                                warn!("[LINK_HTTP] TOKEN_EXPIRED — reauthenticating and retrying");
                                match refresher().await {
                                    Ok(new_auth) => {
                                        Self::validate_request_auth(&new_auth)?;
                                        *self.auth.lock().unwrap() = new_auth.clone();
                                        // Retry exactly once with fresh credentials.
                                        let mut retry_builder =
                                            self.http_client.post(&self.sql_url).json(&request);
                                        retry_builder = new_auth.apply_to_request(retry_builder)?;
                                        match retry_builder.send().await {
                                            Ok(retry_resp) => {
                                                return Self::handle_response(retry_resp, sql).await
                                            },
                                            Err(e) => return Err(e.into()),
                                        }
                                    },
                                    Err(e) => {
                                        warn!("[LINK_HTTP] Auth refresh failed: {}", e);
                                        // Return the original TOKEN_EXPIRED response.
                                    },
                                }
                            }
                        }
                    }

                    if result.is_ok() {
                        let total_duration_ms = overall_start.elapsed().as_millis();
                        debug!(
                            "[LINK_QUERY] Success: http_ms={} total_ms={}",
                            http_duration_ms, total_duration_ms
                        );
                    }

                    return result;
                },
                Err(e) if retry_safe_sql && retries < max_retries && Self::is_retriable(&e) => {
                    let http_duration_ms = attempt_start.elapsed().as_millis();
                    warn!(
                        "[LINK_HTTP] Retriable error (attempt {}/{}): {} duration_ms={}",
                        retries + 1,
                        max_retries + 1,
                        e,
                        http_duration_ms
                    );
                    retries += 1;
                    tokio::time::sleep(tokio::time::Duration::from_millis(100 * retries as u64))
                        .await;
                    continue;
                },
                Err(e) => {
                    let http_duration_ms = attempt_start.elapsed().as_millis();
                    warn!(
                        "[LINK_HTTP] Fatal error: {} duration_ms={} total_ms={}",
                        e,
                        http_duration_ms,
                        overall_start.elapsed().as_millis()
                    );
                    return Err(e.into());
                },
            }
        }
    }

    fn is_retriable(err: &reqwest::Error) -> bool {
        err.is_timeout() || err.is_connect()
    }

    fn multipart_leader_retry_url(
        result: &Result<QueryResponse>,
        current_sql_url: &str,
    ) -> Option<String> {
        let leader_url = match result {
            Ok(response) => Self::leader_url_from_query_response(response),
            Err(KalamLinkError::ServerError { message, .. }) => {
                Self::leader_url_from_error_text(message)
            },
            Err(_) => None,
        }?;

        let retry_url = format!("{}/v1/api/sql", leader_url.trim_end_matches('/'));
        (retry_url != current_sql_url).then_some(retry_url)
    }

    fn leader_url_from_query_response(response: &QueryResponse) -> Option<String> {
        let error = response.error.as_ref()?;
        Self::extract_leader_url(&error.message)
            .or_else(|| error.details.as_deref().and_then(Self::extract_leader_url))
    }

    fn leader_url_from_error_text(error_text: &str) -> Option<String> {
        serde_json::from_str::<QueryResponse>(error_text)
            .ok()
            .and_then(|response| Self::leader_url_from_query_response(&response))
            .or_else(|| Self::extract_leader_url(error_text))
    }

    fn extract_leader_url(text: &str) -> Option<String> {
        let marker = "Leader:";
        let index = text.find(marker)?;
        let mut leader = text[index + marker.len()..].trim();

        if let Some(stripped) = leader.strip_prefix("Some(\"") {
            leader = stripped;
            leader = &leader[..leader.find("\")").unwrap_or(leader.len())];
        }

        leader = leader.trim_matches(|ch| matches!(ch, '"' | '\\' | ')' | '(' | '[' | ']'));
        leader = leader.split_whitespace().next().unwrap_or(leader).trim_end_matches([',', ';']);

        if leader.starts_with("http://") || leader.starts_with("https://") {
            Some(leader.to_string())
        } else {
            None
        }
    }

    #[cfg(feature = "file-uploads")]
    async fn execute_multipart_once(
        &self,
        sql_url: &str,
        auth_snapshot: &AuthProvider,
        sql: &str,
        files: &[MultipartUploadFile],
        params: Option<&Vec<serde_json::Value>>,
        namespace_id: Option<&str>,
        progress: Option<UploadProgressCallback>,
    ) -> Result<QueryResponse> {
        let mut form = Form::new().text("sql", sql.to_string());

        if let Some(p) = params {
            form = form.text("params", serde_json::to_string(p)?);
        }

        if let Some(ns) = namespace_id {
            form = form.text("namespace_id", ns.to_string());
        }

        let total_files = files.len();
        for (index, file) in files.iter().enumerate() {
            let total_bytes = file.data.len() as u64;
            let field_name = format!("file:{}", file.placeholder_name);

            let part = if let Some(progress_cb) = progress.clone() {
                let file_name = Arc::<str>::from(file.filename.clone());
                let file_index = index + 1;

                let stream = build_progress_stream(
                    file.data.clone(),
                    Arc::clone(&file_name),
                    file_index,
                    total_files,
                    progress_cb,
                );

                let body = reqwest::Body::wrap(StreamBody::new(stream));
                Part::stream_with_length(body, total_bytes)
            } else {
                Part::stream_with_length(reqwest::Body::from(file.data.clone()), total_bytes)
            };

            let part = part
                .file_name(file.filename.clone())
                .mime_str(file.mime_type.as_deref().unwrap_or("application/octet-stream"))
                .map_err(|e| {
                    KalamLinkError::ConfigurationError(format!("Invalid MIME type: {}", e))
                })?;

            form = form.part(field_name, part);
        }

        let mut req_builder = self.http_client.post(sql_url).multipart(form);
        req_builder = auth_snapshot.apply_to_request(req_builder)?;

        let attempt_start = Instant::now();
        debug!("[LINK_HTTP] Sending multipart POST to {}", sql_url);

        let response = req_builder.send().await?;
        let http_duration_ms = attempt_start.elapsed().as_millis();
        debug!(
            "[LINK_HTTP] Response received: status={} duration_ms={}",
            response.status(),
            http_duration_ms
        );

        Self::handle_response(response, sql).await
    }

    async fn handle_response(response: reqwest::Response, _sql: &str) -> Result<QueryResponse> {
        let status = response.status();
        if status.is_success() {
            let parse_start = Instant::now();
            let query_response: QueryResponse = response.json().await?;
            let parse_duration_ms = parse_start.elapsed().as_millis();
            debug!("[LINK_QUERY] Parsed response in {}ms", parse_duration_ms);
            return Ok(query_response);
        }

        let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());

        if status.is_client_error() {
            let status_code = status.as_u16();

            // If the body is a valid QueryResponse with TOKEN_EXPIRED, return
            // it as Ok so the caller's auto-refresh retry logic can handle it.
            if let Ok(query_response) = serde_json::from_str::<QueryResponse>(&error_text) {
                if query_response.is_token_expired() {
                    debug!("[LINK_HTTP] TOKEN_EXPIRED returned with HTTP {}", status_code);
                    return Ok(query_response);
                }
            }

            warn!(
                "[LINK_HTTP] Authentication/client error: status={} message=\"{}\"",
                status, error_text
            );
            return Err(KalamLinkError::ServerError {
                status_code,
                message: error_text,
            });
        }

        if let Ok(json_response) = serde_json::from_str::<QueryResponse>(&error_text) {
            return Ok(json_response);
        }

        warn!("[LINK_HTTP] Server error: status={} message=\"{}\"", status, error_text);

        Err(KalamLinkError::ServerError {
            status_code: status.as_u16(),
            message: error_text,
        })
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "file-uploads")]
    use std::sync::{Arc, Mutex};

    #[cfg(feature = "file-uploads")]
    use futures_util::StreamExt;

    #[cfg(feature = "file-uploads")]
    use super::{build_progress_stream, UploadProgress, UploadProgressCallback};

    #[cfg(feature = "file-uploads")]
    #[tokio::test]
    async fn progress_stream_reports_completion() {
        let data = bytes::Bytes::from(vec![1u8; 128 * 1024]);
        let file_name = Arc::<str>::from("example.txt");
        let last_progress = Arc::new(Mutex::new(None::<UploadProgress>));

        let last_progress_clone = Arc::clone(&last_progress);
        let progress_cb: UploadProgressCallback = Arc::new(move |progress| {
            *last_progress_clone.lock().unwrap() = Some(progress);
        });

        let stream = build_progress_stream(data.clone(), Arc::clone(&file_name), 2, 3, progress_cb);

        futures_util::pin_mut!(stream);
        while let Some(frame) = stream.next().await {
            frame.unwrap();
        }

        let progress = last_progress.lock().unwrap().clone().expect("no progress reported");
        assert_eq!(progress.file_index, 2);
        assert_eq!(progress.total_files, 3);
        assert_eq!(progress.file_name, "example.txt");
        assert_eq!(progress.total_bytes, data.len() as u64);
        assert_eq!(progress.bytes_sent, data.len() as u64);
        assert!((progress.percent - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn extract_leader_url_handles_optional_url_hint() {
        let url = super::QueryExecutor::extract_leader_url(
            "Statement 1 failed: Not leader for shard. Leader: Some(\"http://127.0.0.1:2903\")",
        )
        .expect("leader hint should parse");

        assert_eq!(url, "http://127.0.0.1:2903");
    }

    #[test]
    fn leader_retry_url_reads_structured_query_error() {
        let error_text = r#"{
            "status": "error",
            "results": [],
            "error": {
                "code": "SQL_EXECUTION_ERROR",
                "message": "Statement 1 failed: Not leader for shard. Leader: Some(\"http://127.0.0.1:2903\")"
            }
        }"#;

        let url = super::QueryExecutor::leader_url_from_error_text(error_text)
            .expect("structured query error should yield leader URL");

        assert_eq!(url, "http://127.0.0.1:2903");
    }
}
