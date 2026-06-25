//! FILE column reference helpers for KalamDB SDKs.
//!
//! The core [`FileRef`] JSON model lives in `kalamdb-commons`. This module adds
//! client-side context binding so SDK callers can generate URLs or download a
//! file without passing namespace/table repeatedly.

use std::ops::Deref;

use kalamdb_commons::TableId;
use serde::{Deserialize, Serialize};

pub use kalamdb_commons::FileRef;

/// Table context needed to locate a FILE column value for downloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileRefContext(TableId);

impl FileRefContext {
    /// Create a new file reference context from a [`TableId`].
    pub fn new(table_id: TableId) -> Self {
        Self(table_id)
    }

    /// Create a file reference context from namespace and table name strings.
    pub fn from_strings(namespace: &str, table: &str) -> Self {
        Self(TableId::from_strings(namespace, table))
    }

    /// Borrow the table identity attached to this context.
    pub fn table_id(&self) -> &TableId {
        &self.0
    }
}

/// A [`FileRef`] with table context attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundFileRef {
    file_ref: FileRef,
    context: FileRefContext,
}

impl BoundFileRef {
    /// Create a bound file reference from a raw file reference and context.
    pub fn new(file_ref: FileRef, context: FileRefContext) -> Self {
        Self { file_ref, context }
    }

    /// Borrow the underlying storage/wire file reference.
    pub fn file_ref(&self) -> &FileRef {
        &self.file_ref
    }

    /// Consume the bound reference and return the underlying file reference.
    pub fn into_file_ref(self) -> FileRef {
        self.file_ref
    }

    /// Borrow the context attached to this file reference.
    pub fn context(&self) -> &FileRefContext {
        &self.context
    }

    /// Table that owns the FILE column.
    pub fn table_id(&self) -> &TableId {
        self.context.table_id()
    }

    /// Namespace that owns the table.
    pub fn namespace(&self) -> &str {
        self.context.table_id().namespace_id().as_str()
    }

    /// Table that owns the FILE column.
    pub fn table(&self) -> &str {
        self.context.table_id().table_name().as_str()
    }

    /// Full download URL for this file.
    pub fn download_url(&self, base_url: &str) -> String {
        self.file_ref.download_url(base_url, self.namespace(), self.table())
    }

    /// Relative HTTP path for this file.
    pub fn relative_url(&self) -> String {
        self.file_ref.relative_url(self.namespace(), self.table())
    }
}

impl Deref for BoundFileRef {
    type Target = FileRef;

    fn deref(&self) -> &Self::Target {
        &self.file_ref
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_from_json_string() {
        let json = r#"{"id":"123","sub":"f0001","name":"test.png","size":1024,"mime":"image/png","sha256":"abc"}"#;
        let fr = FileRef::from_json(json).unwrap();
        assert_eq!(fr.id, "123");
        assert_eq!(fr.sub, "f0001");
        assert_eq!(fr.name, "test.png");
        assert_eq!(fr.size, 1024);
        assert!(fr.is_image());
    }

    #[test]
    fn parse_from_json_value_object() {
        let val = serde_json::json!({
            "id": "456", "sub": "f0002", "name": "doc.pdf",
            "size": 2048, "mime": "application/pdf", "sha256": "def"
        });
        let fr = FileRef::from_json_value(&val).unwrap();
        assert!(fr.is_pdf());
        assert_eq!(fr.type_description(), "PDF Document");
    }

    #[test]
    fn parse_from_json_value_string() {
        let inner = r#"{"id":"789","sub":"f0001","name":"a.txt","size":10,"mime":"text/plain","sha256":"x"}"#;
        let val = serde_json::Value::String(inner.to_string());
        let fr = FileRef::from_json_value(&val).unwrap();
        assert_eq!(fr.id, "789");
    }

    #[test]
    fn download_url_generation() {
        let fr = FileRef {
            id: "123".into(),
            sub: "f0001".into(),
            name: "t.png".into(),
            size: 0,
            mime: "image/png".into(),
            sha256: String::new(),
            shard: None,
        };
        assert_eq!(
            fr.download_url("http://localhost:2900", "default", "users"),
            "http://localhost:2900/v1/files/default/users/f0001/123-t.png"
        );
        assert_eq!(fr.relative_url("default", "users"), "/v1/files/default/users/f0001/123-t.png");
    }

    #[test]
    fn bound_file_ref_uses_table_context_for_urls() {
        let fr = FileRef {
            id: "123".into(),
            sub: "f0001".into(),
            name: "t.png".into(),
            size: 0,
            mime: "image/png".into(),
            sha256: String::new(),
            shard: None,
        };
        let ctx = FileRefContext::from_strings("default", "users");
        let bound = BoundFileRef::new(fr, ctx);

        assert_eq!(bound.namespace(), "default");
        assert_eq!(bound.table(), "users");
        assert_eq!(
            bound.download_url("http://localhost:2900/"),
            "http://localhost:2900/v1/files/default/users/f0001/123-t.png"
        );
        assert_eq!(bound.relative_url(), "/v1/files/default/users/f0001/123-t.png");
        assert_eq!(bound.file_ref().name, "t.png");
    }

    #[test]
    fn format_size_units() {
        let mk = |size: u64| FileRef {
            id: String::new(),
            sub: String::new(),
            name: String::new(),
            size,
            mime: String::new(),
            sha256: String::new(),
            shard: None,
        };
        assert_eq!(mk(0).format_size(), "0 B");
        assert_eq!(mk(512).format_size(), "512 B");
        assert_eq!(mk(1024).format_size(), "1.0 KB");
        assert_eq!(mk(1_048_576).format_size(), "1.0 MB");
    }

    #[test]
    fn stored_name_and_path() {
        let fr = FileRef {
            id: "42".into(),
            sub: "f0001".into(),
            name: "My Document.pdf".into(),
            size: 100,
            mime: "application/pdf".into(),
            sha256: String::new(),
            shard: None,
        };
        assert_eq!(fr.stored_name(), "42-my-document.pdf");
        assert_eq!(fr.relative_path(), "f0001/42-my-document.pdf");
    }

    #[test]
    fn stored_name_with_shard() {
        let fr = FileRef {
            id: "42".into(),
            sub: "f0001".into(),
            name: "test.png".into(),
            size: 100,
            mime: "image/png".into(),
            sha256: String::new(),
            shard: Some(3),
        };
        assert_eq!(fr.relative_path(), "shard-3/f0001/42-test.png");
    }

    #[test]
    fn cell_as_file() {
        use super::super::kalam_cell_value::KalamCellValue;

        let cell = KalamCellValue::from(serde_json::json!({
            "id": "1", "sub": "f0001", "name": "a.png",
            "size": 10, "mime": "image/png", "sha256": "x"
        }));
        let fr = cell.as_file().unwrap();
        assert_eq!(fr.id, "1");
        assert!(fr.is_image());

        assert!(KalamCellValue::text("Alice").as_file().is_none());
        assert!(KalamCellValue::null().as_file().is_none());
    }

    #[test]
    fn cell_as_bound_file_attaches_context() {
        use super::super::kalam_cell_value::KalamCellValue;

        let cell = KalamCellValue::from(serde_json::json!({
            "id": "1", "sub": "f0001", "name": "a.png",
            "size": 10, "mime": "image/png", "sha256": "x"
        }));

        let table_id = TableId::from_strings("docs", "files");
        let bound = cell.as_bound_file(&table_id).expect("FILE column");
        assert_eq!(bound.namespace(), "docs");
        assert_eq!(bound.table(), "files");
        assert_eq!(bound.table_id(), &table_id);
        assert_eq!(bound.file_ref().id, "1");
        assert_eq!(bound.relative_url(), "/v1/files/docs/files/f0001/1-a.png");
    }
}
