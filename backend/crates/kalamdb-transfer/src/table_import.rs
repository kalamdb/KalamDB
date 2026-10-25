use std::{
    io::{Cursor, Read},
    path::{Component, Path},
};

use kalamdb_core::error::KalamDbError;
use zip::ZipArchive;

use crate::table_export::{
    sanitize_filename, TableExportArchiveFile, TableExportArchiveMetadata, TABLE_EXPORT_FORMAT,
    TABLE_EXPORT_METADATA_ENTRY,
};

pub struct TableExportFileData {
    pub metadata: TableExportArchiveFile,
    pub data: Vec<u8>,
}

pub struct TableExportArchive {
    pub metadata: TableExportArchiveMetadata,
    pub files: Vec<TableExportFileData>,
}

pub fn read_table_export_archive(zip_bytes: Vec<u8>) -> Result<TableExportArchive, KalamDbError> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(|error| {
        KalamDbError::InvalidOperation(format!("Invalid import ZIP archive: {}", error))
    })?;
    let metadata = read_export_metadata(&mut archive)?;
    let mut files = Vec::with_capacity(metadata.files.len());

    for file in &metadata.files {
        if !is_safe_zip_entry(&file.entry_path) || !file.entry_path.starts_with("data/") {
            return Err(KalamDbError::InvalidOperation(format!(
                "Unsafe table export ZIP entry: {}",
                file.entry_path
            )));
        }

        let mut zip_file = archive.by_name(&file.entry_path).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Missing table export ZIP entry '{}': {}",
                file.entry_path, error
            ))
        })?;
        let mut data = Vec::with_capacity(zip_file.size().min(usize::MAX as u64) as usize);
        zip_file.read_to_end(&mut data).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to read table export ZIP entry '{}': {}",
                file.entry_path, error
            ))
        })?;

        files.push(TableExportFileData {
            metadata: file.clone(),
            data,
        });
    }

    Ok(TableExportArchive { metadata, files })
}

pub fn import_segment_filename(import_id: &str, index: usize, source_path: &str) -> String {
    format!("import-{}-{:05}-{}", import_id, index, sanitize_filename(source_path))
}

fn read_export_metadata(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
) -> Result<TableExportArchiveMetadata, KalamDbError> {
    let mut metadata_file = archive.by_name(TABLE_EXPORT_METADATA_ENTRY).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Import ZIP is not a KalamDB table export archive: missing {} ({})",
            TABLE_EXPORT_METADATA_ENTRY, error
        ))
    })?;
    let mut metadata_json = String::new();
    metadata_file.read_to_string(&mut metadata_json).map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to read table export metadata: {}", error))
    })?;
    let metadata: TableExportArchiveMetadata =
        serde_json::from_str(&metadata_json).map_err(|error| {
            KalamDbError::InvalidOperation(format!("Invalid table export metadata: {}", error))
        })?;
    if metadata.format != TABLE_EXPORT_FORMAT || metadata.format_version != 1 {
        return Err(KalamDbError::InvalidOperation(format!(
            "Unsupported table export archive format '{}' version {}",
            metadata.format, metadata.format_version
        )));
    }
    Ok(metadata)
}

fn is_safe_zip_entry(value: &str) -> bool {
    if value.is_empty() || value.starts_with('/') || value.contains('\\') {
        return false;
    }

    Path::new(value)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}
