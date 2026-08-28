//! Multipart file payload for SQL `FILE("placeholder")` uploads.

/// File bytes referenced by a `FILE("placeholder")` expression in SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUpload {
    /// Placeholder name used in SQL, without quotes (e.g. `"upload"` → `upload`).
    pub placeholder: String,
    /// Original filename sent to the server.
    pub filename:    String,
    /// Raw file content.
    pub data:        Vec<u8>,
    /// Optional MIME type override.
    pub mime:        Option<String>,
}

impl FileUpload {
    /// Create a new upload payload for a SQL placeholder.
    pub fn new(placeholder: impl Into<String>, filename: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            placeholder: placeholder.into(),
            filename: filename.into(),
            data,
            mime: None,
        }
    }

    /// Set an explicit MIME type for this upload.
    pub fn with_mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }
}

impl FileUpload {
    pub(crate) fn into_owned_tuple(self) -> (String, String, Vec<u8>, Option<String>) {
        (self.placeholder, self.filename, self.data, self.mime)
    }
}

pub(crate) fn uploads_to_owned(
    files: Option<Vec<FileUpload>>,
) -> Option<Vec<(String, String, Vec<u8>, Option<String>)>> {
    files.map(|items| items.into_iter().map(FileUpload::into_owned_tuple).collect())
}
