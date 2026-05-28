use std::path::{Path, PathBuf};

pub fn build_user_export_download_url(user_id: &str, export_id: &str) -> String {
    format!("/v1/exports/{}/{}", user_id, export_id)
}

pub fn build_table_export_download_url(export_id: &str) -> String {
    format!("/v1/table-exports/{}", export_id)
}

pub fn generate_user_export_id(user_id: &str) -> String {
    format!(
        "export-{}-{}",
        user_id,
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    )
}

pub fn generate_table_export_id() -> String {
    format!(
        "table-export-{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn generate_table_import_id() -> String {
    format!(
        "table-import-{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn user_export_zip_path(exports_root: &Path, user_id: &str, export_id: &str) -> PathBuf {
    exports_root.join(user_id).join(format!("{}.zip", export_id))
}

pub fn table_export_zip_path(exports_root: &Path, export_id: &str) -> PathBuf {
    exports_root.join("tables").join(format!("{}.zip", export_id))
}

pub fn table_import_zip_path(exports_root: &Path, import_id: &str) -> PathBuf {
    exports_root.join("imports").join(format!("{}.zip", import_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_download_urls() {
        assert_eq!(
            build_user_export_download_url("alice", "export-123"),
            "/v1/exports/alice/export-123"
        );
        assert_eq!(
            build_table_export_download_url("table-export-123"),
            "/v1/table-exports/table-export-123"
        );
    }

    #[test]
    fn builds_zip_paths() {
        let root = Path::new("/tmp/exports");
        assert_eq!(
            user_export_zip_path(root, "alice", "export-1"),
            PathBuf::from("/tmp/exports/alice/export-1.zip")
        );
        assert_eq!(
            table_export_zip_path(root, "table-export-1"),
            PathBuf::from("/tmp/exports/tables/table-export-1.zip")
        );
        assert_eq!(
            table_import_zip_path(root, "table-import-1"),
            PathBuf::from("/tmp/exports/imports/table-import-1.zip")
        );
    }
}
