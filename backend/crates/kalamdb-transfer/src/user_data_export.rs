use std::io::{Cursor, Write};

use kalamdb_core::error::KalamDbError;
use zip::{write::SimpleFileOptions, ZipWriter};

pub struct UserDataExportZip {
    pub bytes:      Vec<u8>,
    pub raw_bytes:  usize,
    pub file_count: usize,
}

pub struct UserDataExportZipBuilder {
    zip_writer:  ZipWriter<Cursor<Vec<u8>>>,
    zip_options: SimpleFileOptions,
    raw_bytes:   usize,
    file_count:  usize,
}

impl UserDataExportZipBuilder {
    pub fn new() -> Self {
        Self {
            zip_writer:  ZipWriter::new(Cursor::new(Vec::new())),
            zip_options: SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .compression_level(Some(6)),
            raw_bytes:   0,
            file_count:  0,
        }
    }

    pub fn add_file(
        &mut self,
        namespace_id: &str,
        table_name: &str,
        file_name: &str,
        data: &[u8],
    ) -> Result<(), KalamDbError> {
        let zip_entry = format!("{}/{}/{}", namespace_id, table_name, file_name);

        self.zip_writer.start_file(&zip_entry, self.zip_options).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to add '{}' to ZIP: {}",
                zip_entry, error
            ))
        })?;
        self.zip_writer.write_all(data).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to write '{}' to ZIP: {}",
                zip_entry, error
            ))
        })?;

        self.raw_bytes += data.len();
        self.file_count += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<UserDataExportZip, KalamDbError> {
        let finished = self.zip_writer.finish().map_err(|error| {
            KalamDbError::InvalidOperation(format!("Failed to finalize ZIP archive: {}", error))
        })?;

        Ok(UserDataExportZip {
            bytes:      finished.into_inner(),
            raw_bytes:  self.raw_bytes,
            file_count: self.file_count,
        })
    }
}

impl Default for UserDataExportZipBuilder {
    fn default() -> Self {
        Self::new()
    }
}
