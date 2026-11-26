use std::{
    fs::File,
    io::{BufWriter, Cursor, Write},
    path::{Path, PathBuf},
};

use crate::error::TransferError;
use zip::{write::SimpleFileOptions, ZipWriter};

pub struct UserDataExportZip {
    pub bytes: Vec<u8>,
    pub raw_bytes: usize,
    pub file_count: usize,
}

pub struct UserDataExportStats {
    pub raw_bytes: usize,
    pub file_count: usize,
    pub zip_bytes: u64,
}

/// Writes a user export directly to disk, one file at a time.
pub struct UserDataExportFile {
    zip_writer: ZipWriter<BufWriter<File>>,
    zip_options: SimpleFileOptions,
    path: PathBuf,
    raw_bytes: usize,
    file_count: usize,
}

pub struct UserDataExportZipBuilder {
    zip_writer: ZipWriter<Cursor<Vec<u8>>>,
    zip_options: SimpleFileOptions,
    raw_bytes: usize,
    file_count: usize,
}

impl UserDataExportZipBuilder {
    pub fn new() -> Self {
        Self {
            zip_writer: ZipWriter::new(Cursor::new(Vec::new())),
            zip_options: SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(6)),
            raw_bytes: 0,
            file_count: 0,
        }
    }

    pub fn add_file(
        &mut self,
        namespace_id: &str,
        table_name: &str,
        file_name: &str,
        data: &[u8],
    ) -> Result<(), TransferError> {
        let zip_entry = format!("{}/{}/{}", namespace_id, table_name, file_name);

        self.zip_writer.start_file(&zip_entry, self.zip_options).map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to add '{}' to ZIP: {}",
                zip_entry, error
            ))
        })?;
        self.zip_writer.write_all(data).map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to write '{}' to ZIP: {}",
                zip_entry, error
            ))
        })?;

        self.raw_bytes += data.len();
        self.file_count += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<UserDataExportZip, TransferError> {
        let finished = self.zip_writer.finish().map_err(|error| {
            TransferError::InvalidOperation(format!("Failed to finalize ZIP archive: {}", error))
        })?;

        Ok(UserDataExportZip {
            bytes: finished.into_inner(),
            raw_bytes: self.raw_bytes,
            file_count: self.file_count,
        })
    }
}

impl Default for UserDataExportZipBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl UserDataExportFile {
    pub fn create(path: &Path) -> Result<Self, TransferError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                TransferError::InvalidOperation(format!(
                    "Failed to create user export directory '{}': {}",
                    parent.display(),
                    error
                ))
            })?;
        }
        let file = File::create(path).map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to create user export ZIP '{}': {}",
                path.display(),
                error
            ))
        })?;
        let writer = BufWriter::with_capacity(crate::TRANSFER_STREAM_BUFFER_BYTES, file);
        Ok(Self {
            zip_writer: ZipWriter::new(writer),
            zip_options: SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(6)),
            path: path.to_path_buf(),
            raw_bytes: 0,
            file_count: 0,
        })
    }

    pub fn add_file(
        &mut self,
        namespace_id: &str,
        table_name: &str,
        file_name: &str,
        data: &[u8],
    ) -> Result<(), TransferError> {
        let zip_entry = format!("{}/{}/{}", namespace_id, table_name, file_name);
        self.zip_writer.start_file(&zip_entry, self.zip_options).map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to add '{}' to ZIP: {}",
                zip_entry, error
            ))
        })?;
        self.zip_writer.write_all(data).map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to write '{}' to ZIP: {}",
                zip_entry, error
            ))
        })?;
        self.raw_bytes += data.len();
        self.file_count += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<UserDataExportStats, TransferError> {
        let raw_bytes = self.raw_bytes;
        let file_count = self.file_count;
        let mut finished = self.zip_writer.finish().map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to finalize ZIP archive '{}': {}",
                self.path.display(),
                error
            ))
        })?;
        finished.flush().map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to flush ZIP archive '{}': {}",
                self.path.display(),
                error
            ))
        })?;
        let file = finished.into_inner().map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to finish ZIP archive '{}': {}",
                self.path.display(),
                error
            ))
        })?;
        let zip_bytes = file.metadata().map(|metadata| metadata.len()).map_err(|error| {
            TransferError::InvalidOperation(format!(
                "Failed to stat ZIP archive '{}': {}",
                self.path.display(),
                error
            ))
        })?;
        Ok(UserDataExportStats {
            raw_bytes,
            file_count,
            zip_bytes,
        })
    }
}
