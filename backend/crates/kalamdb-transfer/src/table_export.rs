use std::{
    io::{Cursor, Write},
    path::Path,
};

use kalamdb_commons::{
    models::UserId,
    schemas::{TableDefinition, TableType},
    TableId,
};
use kalamdb_core::error::KalamDbError;
use kalamdb_system::Manifest;
use serde::{Deserialize, Serialize};
use zip::{write::SimpleFileOptions, ZipWriter};

pub const TABLE_EXPORT_FORMAT: &str = "kalamdb.table_export.v1";
pub const TABLE_EXPORT_METADATA_ENTRY: &str = "kalamdb-table-export.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableExportArchiveFile {
    pub source_path: String,
    pub entry_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableExportArchiveMetadata {
    pub format: String,
    pub format_version: u32,
    pub source_table_id: TableId,
    pub source_table_type: TableType,
    pub source_user_id: Option<UserId>,
    pub exported_at: i64,
    pub table_definition: TableDefinition,
    pub manifest: Manifest,
    pub files: Vec<TableExportArchiveFile>,
}

impl TableExportArchiveMetadata {
    pub fn new(
        source_table_id: TableId,
        source_table_type: TableType,
        source_user_id: Option<UserId>,
        table_definition: TableDefinition,
        manifest: Manifest,
    ) -> Self {
        Self {
            format: TABLE_EXPORT_FORMAT.to_string(),
            format_version: 1,
            source_table_id,
            source_table_type,
            source_user_id,
            exported_at: chrono::Utc::now().timestamp_millis(),
            table_definition,
            manifest,
            files: Vec::new(),
        }
    }
}

pub struct TableExportSegmentData {
    pub source_path: String,
    pub data: Vec<u8>,
}

pub struct TableExportZip {
    pub bytes: Vec<u8>,
    pub raw_bytes: usize,
    pub file_count: usize,
}

pub fn write_table_export_zip(
    mut metadata: TableExportArchiveMetadata,
    segment_data: Vec<TableExportSegmentData>,
) -> Result<TableExportZip, KalamDbError> {
    let zip_buffer = Cursor::new(Vec::new());
    let mut zip_writer = ZipWriter::new(zip_buffer);
    let zip_options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));

    let mut archive_files = Vec::with_capacity(segment_data.len());
    let mut raw_bytes = 0usize;

    for (index, segment) in segment_data.into_iter().enumerate() {
        let entry_path = format!("data/{:05}-{}", index, sanitize_filename(&segment.source_path));
        zip_writer.start_file(&entry_path, zip_options).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to add '{}' to ZIP: {}",
                entry_path, error
            ))
        })?;
        zip_writer.write_all(&segment.data).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to write '{}' to ZIP: {}",
                entry_path, error
            ))
        })?;

        raw_bytes += segment.data.len();
        archive_files.push(TableExportArchiveFile {
            source_path: segment.source_path,
            entry_path,
            size_bytes: segment.data.len() as u64,
        });
    }

    metadata.files = archive_files;
    let file_count = metadata.files.len();
    let metadata_bytes = serde_json::to_vec_pretty(&metadata).map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to serialize export metadata: {}", error))
    })?;

    zip_writer
        .start_file(TABLE_EXPORT_METADATA_ENTRY, zip_options)
        .map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to add export metadata: {}", error))
        })?;
    zip_writer.write_all(&metadata_bytes).map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to write export metadata: {}", error))
    })?;

    let finished = zip_writer.finish().map_err(|error| {
        KalamDbError::InvalidOperation(format!("Failed to finalize ZIP archive: {}", error))
    })?;

    Ok(TableExportZip {
        bytes: finished.into_inner(),
        raw_bytes,
        file_count,
    })
}

pub fn parse_transfer_user_id(
    table_type: TableType,
    user_id: Option<&str>,
) -> Result<Option<UserId>, KalamDbError> {
    match table_type {
        TableType::User => {
            let raw =
                user_id.map(str::trim).filter(|value| !value.is_empty()).ok_or_else(|| {
                    KalamDbError::InvalidOperation(
                        "user_id is required for user table transfer".to_string(),
                    )
                })?;
            UserId::try_new(raw).map(Some).map_err(|error| {
                KalamDbError::InvalidOperation(format!("Invalid user_id '{}': {}", raw, error))
            })
        },
        TableType::Shared => {
            if user_id.map(str::trim).filter(|value| !value.is_empty()).is_some() {
                return Err(KalamDbError::InvalidOperation(
                    "user_id must be omitted for shared table transfer".to_string(),
                ));
            }
            Ok(None)
        },
        TableType::Stream | TableType::System => Err(KalamDbError::InvalidOperation(
            "Only user and shared tables can be exported or imported".to_string(),
        )),
    }
}

pub fn validate_table_transfer_scope(
    table_type: TableType,
    user_id: Option<&str>,
) -> Result<(), KalamDbError> {
    parse_transfer_user_id(table_type, user_id).map(|_| ())
}

pub fn is_safe_transfer_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn sanitize_filename(value: &str) -> String {
    let basename = Path::new(value)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("segment.parquet");
    let sanitized = basename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "segment.parquet".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::schemas::TableType;

    use super::*;

    #[test]
    fn transfer_id_rejects_path_and_header_injection() {
        for value in [
            "",
            "../escape",
            "export/alice",
            "export\\alice",
            "export\nalice",
            "export\ralice",
            "export;DROP",
            "export'quote",
            "export.zip",
            "éxport",
        ] {
            assert!(!is_safe_transfer_id(value), "expected rejection for {value:?}");
        }
    }

    #[test]
    fn validate_scope_requires_user_id_for_user_tables() {
        assert!(validate_table_transfer_scope(TableType::User, Some("alice")).is_ok());
        assert!(validate_table_transfer_scope(TableType::User, None).is_err());
        assert!(validate_table_transfer_scope(TableType::Shared, None).is_ok());
        assert!(validate_table_transfer_scope(TableType::Shared, Some("alice")).is_err());
    }
}
