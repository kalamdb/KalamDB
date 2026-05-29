//! FileRef model for FILE datatype storage.
//!
//! Represents a reference to an uploaded file stored in the KalamDB file storage.
//! Files are stored in table-specific folders with automatic subfolder rotation.

use serde::{Deserialize, Serialize};

/// File reference stored as JSON in FILE columns.
///
/// Contains all metadata needed to locate and serve the file.
/// Stored in RocksDB/Parquet as a JSON string for flexibility.
///
/// # Storage Layout
///
/// For user tables: `{table_path}/{subfolder}/{stored_name}`
/// For shared tables: `{table_path}/shard-{n}/{subfolder}/{stored_name}`
///
/// # Example JSON
///
/// ```json
/// {
///   "id": "1234567890123456789",
///   "sub": "f0001",
///   "name": "document.pdf",
///   "size": 1048576,
///   "mime": "application/pdf",
///   "sha256": "abc123..."
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    /// Unique file identifier (Snowflake ID)
    pub id: String,

    /// Subfolder name (e.g., "f0001", "f0002")
    /// Subfolders are rotated when file count exceeds limit
    pub sub: String,

    /// Original filename (preserved for display/download)
    pub name: String,

    /// File size in bytes
    pub size: u64,

    /// MIME type (e.g., "image/png", "application/pdf")
    pub mime: String,

    /// SHA-256 hash of file content (hex-encoded)
    pub sha256: String,

    /// Optional shard ID for shared tables
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard: Option<u32>,
}

const SUBFOLDER_PREFIX: &str = "f";
const SUBFOLDER_DIGITS: usize = 4;

impl FileRef {
    /// Create a new FileRef
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

    /// Create a FileRef for a shared table with shard info
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

    /// Create a partial FileRef for download operations.
    ///
    /// When downloading, we only have the file_id and subfolder from the URL.
    /// This creates a minimal FileRef sufficient to locate the file.
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

    /// Get the stored filename (sanitized)
    ///
    /// Format: `{id}-{sanitized_name}.{ext}` or `{id}.{ext}` if name is non-ASCII
    pub fn stored_name(&self) -> String {
        let sanitized = Self::sanitize_filename(&self.name);
        let ext = Self::extract_extension(&self.name);

        if sanitized.is_empty() {
            format!("{}.{}", self.id, ext)
        } else {
            format!("{}-{}.{}", self.id, sanitized, ext)
        }
    }

    /// Validate subfolder name format (e.g., "f0001")
    pub fn is_valid_subfolder(subfolder: &str) -> bool {
        subfolder.len() == SUBFOLDER_PREFIX.len() + SUBFOLDER_DIGITS
            && subfolder.starts_with(SUBFOLDER_PREFIX)
            && subfolder[SUBFOLDER_PREFIX.len()..].chars().all(|c| c.is_ascii_digit())
    }

    /// Get the relative path within the table folder
    ///
    /// For user tables: `{subfolder}/{stored_name}`
    /// For shared tables with shard: `shard-{n}/{subfolder}/{stored_name}`
    pub fn relative_path(&self) -> String {
        let stored_name = self.stored_name();
        match self.shard {
            Some(shard_id) => format!("shard-{}/{}/{}", shard_id, self.sub, stored_name),
            None => format!("{}/{}", self.sub, stored_name),
        }
    }

    /// Serialize to JSON string for storage in FILE columns
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Sanitize filename for storage
    ///
    /// - Convert to lowercase
    /// - Keep only a-z, 0-9, and dash
    /// - Replace spaces with dashes
    /// - Limit to 50 characters
    /// - Returns empty string if result is all non-ASCII
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

        // Remove leading/trailing dashes and collapse multiple dashes
        let mut result = String::with_capacity(sanitized.len());
        let mut last_was_dash = true; // Treat start as dash to skip leading dashes
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

    /// Extract file extension, defaulting to "bin" if none
    fn extract_extension(name: &str) -> String {
        name.rsplit_once('.')
            .map(|(_, ext)| {
                let ext_lower = ext.to_ascii_lowercase();
                // Only keep extension if it's ASCII alphanumeric and reasonable length
                if ext_lower.len() <= 10 && ext_lower.chars().all(|c| c.is_ascii_alphanumeric()) {
                    ext_lower
                } else {
                    "bin".to_string()
                }
            })
            .unwrap_or_else(|| "bin".to_string())
    }

    /// Validate MIME type against allowlist
    pub fn validate_mime_type(&self, allowed_mimes: &[String]) -> bool {
        if allowed_mimes.is_empty() {
            return true; // No restrictions
        }
        allowed_mimes.iter().any(|allowed| {
            if allowed.ends_with("/*") {
                // Wildcard match (e.g., "image/*")
                let prefix = allowed.trim_end_matches("/*");
                self.mime.starts_with(prefix)
            } else {
                &self.mime == allowed
            }
        })
    }
}

/// File subfolder tracking for manifest
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileSubfolderState {
    /// Current subfolder index (1-based: f0001, f0002, ...)
    pub subfolder: u32,

    /// Number of files in current subfolder
    pub count: u32,
}

impl FileSubfolderState {
    /// Create new state starting at subfolder 1
    pub fn new() -> Self {
        Self {
            subfolder: 1,
            count: 0,
        }
    }

    /// Get current subfolder name (e.g., "f0001")
    pub fn subfolder_name(&self) -> String {
        format!("{}{:04}", SUBFOLDER_PREFIX, self.subfolder)
    }

    /// Increment file count and rotate subfolder if needed
    ///
    /// Returns the subfolder name to use for the new file
    pub fn allocate_file(&mut self, max_files_per_folder: u32) -> String {
        if self.count >= max_files_per_folder {
            self.subfolder += 1;
            self.count = 0;
        }
        self.count += 1;
        self.subfolder_name()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_ref_creation() {
        let file_ref = FileRef::new(
            "12345".to_string(),
            "f0001".to_string(),
            "test.pdf".to_string(),
            1024,
            "application/pdf".to_string(),
            "abc123".to_string(),
        );

        assert_eq!(file_ref.id, "12345");
        assert_eq!(file_ref.sub, "f0001");
        assert_eq!(file_ref.name, "test.pdf");
        assert_eq!(file_ref.size, 1024);
        assert_eq!(file_ref.mime, "application/pdf");
        assert_eq!(file_ref.shard, None);
    }

    #[test]
    fn test_file_ref_with_shard() {
        let file_ref = FileRef::with_shard(
            "12345".to_string(),
            "f0001".to_string(),
            "test.pdf".to_string(),
            1024,
            "application/pdf".to_string(),
            "abc123".to_string(),
            3,
        );

        assert_eq!(file_ref.shard, Some(3));
        assert_eq!(file_ref.relative_path(), "shard-3/f0001/12345-test.pdf");
    }

    #[test]
    fn test_sanitize_filename() {
        // Normal case
        let file_ref = FileRef::new(
            "123".to_string(),
            "f0001".to_string(),
            "My Document.pdf".to_string(),
            100,
            "application/pdf".to_string(),
            "hash".to_string(),
        );
        assert_eq!(file_ref.stored_name(), "123-my-document.pdf");

        // Non-ASCII filename
        let file_ref = FileRef::new(
            "456".to_string(),
            "f0001".to_string(),
            "文档.pdf".to_string(),
            100,
            "application/pdf".to_string(),
            "hash".to_string(),
        );
        assert_eq!(file_ref.stored_name(), "456.pdf");

        // Long filename (should be truncated)
        let long_name = "a".repeat(100) + ".txt";
        let file_ref = FileRef::new(
            "789".to_string(),
            "f0001".to_string(),
            long_name,
            100,
            "text/plain".to_string(),
            "hash".to_string(),
        );
        let stored = file_ref.stored_name();
        // Should be id + dash + 50 chars + .txt
        assert!(stored.len() <= 60);
    }

    #[test]
    fn test_relative_path() {
        let file_ref = FileRef::new(
            "123".to_string(),
            "f0002".to_string(),
            "image.png".to_string(),
            2048,
            "image/png".to_string(),
            "hash".to_string(),
        );
        assert_eq!(file_ref.relative_path(), "f0002/123-image.png");
    }

    #[test]
    fn test_json_roundtrip() {
        let original = FileRef::new(
            "123".to_string(),
            "f0001".to_string(),
            "test.pdf".to_string(),
            1024,
            "application/pdf".to_string(),
            "abc123".to_string(),
        );

        let json = original.to_json();
        let parsed = FileRef::from_json(&json).unwrap();

        assert_eq!(original, parsed);
    }

    #[test]
    fn test_mime_validation() {
        let file_ref = FileRef::new(
            "123".to_string(),
            "f0001".to_string(),
            "image.png".to_string(),
            1024,
            "image/png".to_string(),
            "hash".to_string(),
        );

        // Exact match
        assert!(file_ref.validate_mime_type(&["image/png".to_string()]));

        // Wildcard match
        assert!(file_ref.validate_mime_type(&["image/*".to_string()]));

        // No match
        assert!(!file_ref.validate_mime_type(&["application/pdf".to_string()]));

        // Empty allowlist = allow all
        assert!(file_ref.validate_mime_type(&[]));
    }

    #[test]
    fn test_subfolder_state() {
        let mut state = FileSubfolderState::new();
        assert_eq!(state.subfolder_name(), "f0001");

        // Allocate files within limit
        for _ in 0..5 {
            state.allocate_file(10);
        }
        assert_eq!(state.count, 5);
        assert_eq!(state.subfolder, 1);

        // Allocate more to trigger rotation
        for _ in 0..6 {
            state.allocate_file(10);
        }
        // Should have rotated to subfolder 2
        assert_eq!(state.subfolder, 2);
        assert_eq!(state.subfolder_name(), "f0002");
    }
}
