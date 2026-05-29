//! Shared archive and data-transfer primitives for KalamDB.
//!
//! This crate keeps backup/restore archive handling and table export/import ZIP handling behind
//! one narrow interface so jobs and HTTP handlers can orchestrate transfer workflows without
//! reimplementing packaging, path validation, or metadata parsing.

pub mod archive;
pub mod database_backup;
pub mod database_restore;
pub mod naming;
pub mod table;
pub mod table_export;
pub mod table_import;
pub mod user_data_export;

pub use archive::{
    copy_dir_to_dir, create_archive_staging_dir, create_tar_gz_archive, extract_tar_gz_archive,
    is_tar_gz_path,
};
pub use database_backup::{finalize_database_backup, prepare_database_backup, DatabaseBackupPlan};
pub use database_restore::{
    finalize_database_restore, prepare_database_restore, DatabaseRestorePlan,
};
pub use naming::{
    build_table_export_download_url, build_user_export_download_url, generate_table_export_id,
    generate_table_import_id, generate_user_export_id, table_export_zip_path,
    table_import_zip_path, user_export_zip_path,
};
pub use table::{
    import_segment_filename, is_safe_transfer_id, parse_transfer_user_id,
    read_table_export_archive, sanitize_filename, validate_table_transfer_scope,
    write_table_export_zip, TableExportArchive, TableExportArchiveFile, TableExportArchiveMetadata,
    TableExportFileData, TableExportSegmentData, TableExportZip, TABLE_EXPORT_FORMAT,
    TABLE_EXPORT_METADATA_ENTRY,
};
pub use user_data_export::{UserDataExportZip, UserDataExportZipBuilder};
