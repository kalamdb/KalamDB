//! FileRef model for KalamDB FILE columns.
//!
//! FILE columns store this value as JSON. The JSON is intentionally table-agnostic:
//! table identity is bound by callers when constructing download URLs.

use serde::{Deserialize, Serialize};

/// File reference stored as JSON in FILE columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    /// Unique file identifier.
    pub id:     String,
    /// Subfolder name, for example `"f0001"`.
    pub sub:    String,
    /// Original filename preserved for display and download.
    pub name:   String,
    /// File size in bytes.
    pub size:   u64,
    /// MIME type.
    pub mime:   String,
    /// SHA-256 hash of file content, hex encoded.
    pub sha256: String,
    /// Optional shard ID for shared tables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard:  Option<u32>,
}

const SUBFOLDER_PREFIX: &str = "f";
const SUBFOLDER_DIGITS: usize = 4;

impl FileRef {
    /// Create a new file reference.
    pub fn new(
        id: String,
        subfolder: String,
        name: String,
        size: u64,
        mime: String,
        sha256: String,
    ) -> Self {
        Self {
            id,
            sub: subfolder,
            name,
            size,
            mime,
            sha256,
            shard: None,
        }
    }

    /// Create a new file reference with shared-table shard metadata.
    pub fn with_shard(
        id: String,
        subfolder: String,
        name: String,
        size: u64,
        mime: String,
        sha256: String,
        shard: u32,
    ) -> Self {
        Self {
            id,
            sub: subfolder,
            name,
            size,
            mime,
            sha256,
            shard: Some(shard),
        }
    }

    /// Create a partial file reference for download path validation.
    pub fn partial(id: String, subfolder: String) -> Self {
        Self {
            id,
            sub: subfolder,
            name: String::new(),
            size: 0,
            mime: String::new(),
            sha256: String::new(),
            shard: None,
        }
    }

    /// Parse a file reference from a raw JSON string.
    pub fn from_json(json: &str) -> Option<Self> {
        Self::try_from_json(json).ok()
    }

    /// Parse a file reference from a raw JSON string, preserving serde errors.
    pub fn try_from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Try to extract a file reference from a JSON value.
    pub fn from_json_value(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::String(s) => Self::from_json(s),
            serde_json::Value::Object(_) => serde_json::from_value(value.clone()).ok(),
            _ => None,
        }
    }

    /// Serialize to JSON for storage in FILE columns.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Full download URL for this file.
    pub fn download_url(&self, base_url: &str, namespace: &str, table: &str) -> String {
        let base = base_url.trim_end_matches('/');
        format!("{}/v1/files/{}/{}/{}/{}", base, namespace, table, self.sub, self.stored_name())
    }

    /// Relative HTTP path for this file.
    pub fn relative_url(&self, namespace: &str, table: &str) -> String {
        format!("/v1/files/{}/{}/{}/{}", namespace, table, self.sub, self.stored_name())
    }

    /// Stored filename on disk.
    pub fn stored_name(&self) -> String {
        let sanitized = Self::sanitize_filename(&self.name);
        let ext = Self::extract_extension(&self.name);

        if sanitized.is_empty() {
            format!("{}.{}", self.id, ext)
        } else {
            format!("{}-{}.{}", self.id, sanitized, ext)
        }
    }

    /// Validate subfolder name format, for example `"f0001"`.
    pub fn is_valid_subfolder(subfolder: &str) -> bool {
        subfolder.len() == SUBFOLDER_PREFIX.len() + SUBFOLDER_DIGITS
            && subfolder.starts_with(SUBFOLDER_PREFIX)
            && subfolder[SUBFOLDER_PREFIX.len()..].chars().all(|c| c.is_ascii_digit())
    }

    /// Relative path within the table folder.
    pub fn relative_path(&self) -> String {
        let stored_name = self.stored_name();
        match self.shard {
            Some(shard_id) => format!("shard-{}/{}/{}", shard_id, self.sub, stored_name),
            None => format!("{}/{}", self.sub, stored_name),
        }
    }

    /// Returns true if the MIME type indicates an image.
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }

    /// Returns true if the MIME type indicates a video.
    pub fn is_video(&self) -> bool {
        self.mime.starts_with("video/")
    }

    /// Returns true if the MIME type indicates audio.
    pub fn is_audio(&self) -> bool {
        self.mime.starts_with("audio/")
    }

    /// Returns true if the MIME type indicates a PDF.
    pub fn is_pdf(&self) -> bool {
        self.mime == "application/pdf"
    }

    /// Human-readable file type description.
    pub fn type_description(&self) -> String {
        if self.is_image() {
            return "Image".to_string();
        }
        if self.is_video() {
            return "Video".to_string();
        }
        if self.is_audio() {
            return "Audio".to_string();
        }
        if self.is_pdf() {
            return "PDF Document".to_string();
        }
        if let Some((_type_part, subtype)) = self.mime.split_once('/') {
            format!("{} File", subtype.to_uppercase())
        } else {
            "File".to_string()
        }
    }

    /// Format file size in human-readable units.
    pub fn format_size(&self) -> String {
        const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
        let mut size = self.size as f64;
        let mut idx = 0;

        while size >= 1024.0 && idx < UNITS.len() - 1 {
            size /= 1024.0;
            idx += 1;
        }

        if idx == 0 {
            format!("{} {}", size as u64, UNITS[idx])
        } else {
            format!("{:.1} {}", size, UNITS[idx])
        }
    }

    /// Validate MIME type against an allowlist.
    pub fn validate_mime_type(&self, allowed_mimes: &[String]) -> bool {
        if allowed_mimes.is_empty() {
            return true;
        }
        allowed_mimes.iter().any(|allowed| {
            if allowed.ends_with("/*") {
                let prefix = allowed.trim_end_matches("/*");
                self.mime.starts_with(prefix)
            } else {
                &self.mime == allowed
            }
        })
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_shape_remains_table_agnostic() {
        let file_ref = FileRef::new(
            "123".to_string(),
            "f0001".to_string(),
            "test.pdf".to_string(),
            1024,
            "application/pdf".to_string(),
            "abc123".to_string(),
        );

        let json = file_ref.to_json();
        assert!(json.contains("\"id\":\"123\""));
        assert!(!json.contains("namespace"));
        assert!(!json.contains("table"));
        assert_eq!(FileRef::from_json(&json), Some(file_ref));
    }

    #[test]
    fn stored_name_and_relative_path_match_existing_shape() {
        let file_ref = FileRef::with_shard(
            "42".to_string(),
            "f0001".to_string(),
            "My Document.pdf".to_string(),
            100,
            "application/pdf".to_string(),
            "hash".to_string(),
            3,
        );

        assert_eq!(file_ref.stored_name(), "42-my-document.pdf");
        assert_eq!(file_ref.relative_path(), "shard-3/f0001/42-my-document.pdf");
    }

    #[test]
    fn parses_json_values_and_formats_metadata() {
        let value = serde_json::json!({
            "id": "1",
            "sub": "f0001",
            "name": "photo.png",
            "size": 1048576,
            "mime": "image/png",
            "sha256": "hash"
        });
        let file_ref = FileRef::from_json_value(&value).expect("file ref");

        assert!(file_ref.is_image());
        assert_eq!(file_ref.type_description(), "Image");
        assert_eq!(file_ref.format_size(), "1.0 MB");
    }

    #[test]
    fn validates_subfolder_names() {
        assert!(FileRef::is_valid_subfolder("f0001"));
        assert!(!FileRef::is_valid_subfolder("f001"));
        assert!(!FileRef::is_valid_subfolder("../x"));
    }
}
