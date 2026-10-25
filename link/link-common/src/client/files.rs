//! FILE datatype upload and download helpers on [`KalamLinkClient`].
//!
//! Uploads use multipart SQL (`FILE("placeholder")` in SQL). Downloads call
//! `GET /v1/files/{namespace}/{table}/{sub}/{stored_name}` with the same auth
//! flow as other HTTP requests.

use super::KalamLinkClient;
use crate::{
    error::{KalamLinkError, Result},
    models::{BoundFileRef, FileDownload, FileRef},
};

impl KalamLinkClient {
    /// Download a file referenced by a FILE column value.
    ///
    /// Uses [`FileRef::download_url`] and the client's configured auth. For USER
    /// tables, downloads default to the authenticated caller's scope. Pass
    /// `target_user_id` when a DBA/system role needs to download another user's
    /// file (same rules as the HTTP API's `user_id` query parameter).
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # async fn example() -> kalam_client::Result<()> {
    /// # use kalam_client::{AuthProvider, FileRef, KalamLinkClient};
    /// # let client = KalamLinkClient::builder()
    /// #     .base_url("http://localhost:2900")
    /// #     .auth(AuthProvider::system_user_auth("secret".into()))
    /// #     .build()?;
    /// let rows = client
    ///     .execute_query(
    ///         "SELECT attachment FROM docs.files WHERE id = $1",
    ///         None,
    ///         Some(vec![QueryParam::from("doc1")]),
    ///         None,
    ///     )
    ///     .await?;
    /// let cell = &rows.results[0].rows[0][0];
    /// let table_id = kalamdb_commons::TableId::from_strings("docs", "files");
    /// let file_ref = cell.as_bound_file(&table_id).expect("FILE column");
    /// let download = client.download_bound_file(&file_ref, None).await?;
    /// assert_eq!(download.bytes.len(), file_ref.file_ref().size as usize);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn download_file(
        &self,
        file_ref: &FileRef,
        namespace: &str,
        table: &str,
        target_user_id: Option<&str>,
    ) -> Result<FileDownload> {
        let mut url = file_ref.download_url(self.base_url(), namespace, table);
        if let Some(user_id) = target_user_id {
            url.push_str("?user_id=");
            url.push_str(user_id);
        }

        let auth = self.jwt_for_http_request().await?;
        let request = self.http_client().get(&url);
        let request = auth.apply_to_request(request)?;

        let response = request.send().await.map_err(|error| {
            KalamLinkError::NetworkError(format!("file download request failed: {error}"))
        })?;

        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(KalamLinkError::ServerError {
                status_code: status.as_u16(),
                message: if message.is_empty() {
                    format!("file download failed with status {status}")
                } else {
                    message
                },
            });
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let content_disposition = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let bytes = response.bytes().await.map_err(|error| {
            KalamLinkError::NetworkError(format!("failed to read file download body: {error}"))
        })?;

        Ok(FileDownload {
            bytes: bytes.to_vec(),
            content_type,
            content_disposition,
        })
    }

    /// Download a file reference that already has namespace/table context.
    pub async fn download_bound_file(
        &self,
        file_ref: &BoundFileRef,
        target_user_id: Option<&str>,
    ) -> Result<FileDownload> {
        self.download_file(
            file_ref.file_ref(),
            file_ref.namespace(),
            file_ref.table(),
            target_user_id,
        )
        .await
    }
}
