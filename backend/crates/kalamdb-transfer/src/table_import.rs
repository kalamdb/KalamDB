use std::{
    fs::File,
    io::{BufReader, Cursor, Read, Seek},
    path::{Component, Path},
};

use crate::error::TransferError;
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

/// Reads one committed segment at a time from a table-export ZIP on disk.
pub struct TableExportArchiveReader {
    archive: ZipArchive<BufReader<File>>,
    metadata: TableExportArchiveMetadata,
    next_index: usize,
}

impl TableExportArchiveReader {
    pub fn open(path: &Path) -> Result<Self, TransferError> {
        let file = File::open(path).map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to open import ZIP '{}': {}",
                path.display(),
                error
            ))
        })?;
        let reader = BufReader::with_capacity(crate::TRANSFER_STREAM_BUFFER_BYTES, file);
        let mut archive = ZipArchive::new(reader).map_err(|error| {
            TransferError::InvalidOperation(format!("Invalid import ZIP archive: {}", error))
        })?;
        let metadata = read_export_metadata(&mut archive)?;
        Ok(Self {
            archive,
            metadata,
            next_index: 0,
        })
    }

    pub fn metadata(&self) -> &TableExportArchiveMetadata {
        &self.metadata
    }

    pub fn read_next_file(&mut self) -> Result<Option<TableExportFileData>, TransferError> {
        let Some(file) = self.metadata.files.get(self.next_index).cloned() else {
            return Ok(None);
        };
        self.next_index += 1;
        let data = read_zip_entry(&mut self.archive, &file)?;
        Ok(Some(TableExportFileData {
            metadata: file,
            data,
        }))
    }
}

pub fn read_table_export_archive(zip_bytes: Vec<u8>) -> Result<TableExportArchive, TransferError> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(|error| {
        TransferError::InvalidOperation(format!("Invalid import ZIP archive: {}", error))
    })?;
    let metadata = read_export_metadata(&mut archive)?;
    let mut files = Vec::with_capacity(metadata.files.len());

    for file in &metadata.files {
        let data = read_zip_entry(&mut archive, file)?;
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

fn read_zip_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    file: &TableExportArchiveFile,
) -> Result<Vec<u8>, TransferError> {
    if !is_safe_zip_entry(&file.entry_path) || !file.entry_path.starts_with("data/") {
        return Err(TransferError::InvalidOperation(format!(
            "Unsafe table export ZIP entry: {}",
            file.entry_path
        )));
    }
    let mut zip_file = archive.by_name(&file.entry_path).map_err(|error| {
        TransferError::InvalidOperation(format!(
            "Missing table export ZIP entry '{}': {}",
            file.entry_path, error
        ))
    })?;
    let mut data = Vec::with_capacity(zip_file.size().min(usize::MAX as u64) as usize);
    zip_file.read_to_end(&mut data).map_err(|error| {
        TransferError::InvalidOperation(format!(
            "Failed to read table export ZIP entry '{}': {}",
            file.entry_path, error
        ))
    })?;
    Ok(data)
}

fn read_export_metadata<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<TableExportArchiveMetadata, TransferError> {
    let mut metadata_file = archive.by_name(TABLE_EXPORT_METADATA_ENTRY).map_err(|error| {
        TransferError::InvalidOperation(format!(
            "Import ZIP is not a KalamDB table export archive: missing {} ({})",
            TABLE_EXPORT_METADATA_ENTRY, error
        ))
    })?;
    let mut metadata_json = String::new();
    metadata_file.read_to_string(&mut metadata_json).map_err(|error| {
        TransferError::InvalidOperation(format!("Failed to read table export metadata: {}", error))
    })?;
    let metadata: TableExportArchiveMetadata =
        serde_json::from_str(&metadata_json).map_err(|error| {
            TransferError::InvalidOperation(format!("Invalid table export metadata: {}", error))
        })?;
    if metadata.format != TABLE_EXPORT_FORMAT || metadata.format_version != 1 {
        return Err(TransferError::InvalidOperation(format!(
            "Unsupported table export archive format '{}' version {}",
            metadata.format, metadata.format_version
        )));
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::{
        schemas::{ColumnDefinition, TableDefinition, TableOptions, TableType},
        KalamDataType, SchemaId, TableId, TableName,
    };
    use kalamdb_system::Manifest;

    use super::*;
    use crate::table_export::{StreamingTableExport, TableExportArchiveMetadata};

    #[test]
    fn streaming_export_reads_back_one_segment_at_a_time() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("export.zip");
        let table_id = TableId::new(SchemaId::new("public"), TableName::new("events"));
        let table = TableDefinition::new(
            SchemaId::new("public"),
            TableName::new("events"),
            TableType::Shared,
            vec![ColumnDefinition::primary_key(
                1,
                "id",
                1,
                KalamDataType::BigInt,
            )],
            TableOptions::shared(),
            None,
        )
        .expect("table");
        let manifest = Manifest::new(table_id.clone(), None);
        let mut writer = StreamingTableExport::create(&path).expect("writer");
        writer.write_segment("batch-1.parquet", b"aaa").expect("segment");
        writer.write_segment("batch-2.parquet", b"bbbb").expect("segment");
        let metadata =
            TableExportArchiveMetadata::new(table_id, TableType::Shared, None, table, manifest);
        let stats = writer.finish(metadata).expect("finish");
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.raw_bytes, 7);

        let mut reader = TableExportArchiveReader::open(&path).expect("reader");
        let first = reader.read_next_file().expect("read").expect("first");
        assert_eq!(first.data, b"aaa");
        let second = reader.read_next_file().expect("read").expect("second");
        assert_eq!(second.data, b"bbbb");
        assert!(reader.read_next_file().expect("read").is_none());
    }
}

fn is_safe_zip_entry(value: &str) -> bool {
    if value.is_empty() || value.starts_with('/') || value.contains('\\') {
        return false;
    }

    Path::new(value)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}
