//! Response from a FILE column download request.

/// Bytes and HTTP metadata returned by [`KalamLinkClient::download_file`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDownload {
    /// Raw file content.
    pub bytes:               Vec<u8>,
    /// Value of the `Content-Type` response header, when present.
    pub content_type:        Option<String>,
    /// Value of the `Content-Disposition` response header, when present.
    pub content_disposition: Option<String>,
}
