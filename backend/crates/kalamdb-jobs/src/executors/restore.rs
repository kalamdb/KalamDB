//! Restore Job Executor
//!
//! Restores the entire KalamDB database from a backup directory or `.tar.gz`
//! / `.tgz` archive created by the backup job. Uses RocksDB's native
//! `BackupEngine` for the key-value store and a recursive directory copy for
//! Parquet and stream files.
//!
//! ## Backup Layout Expected
//! ```text
//! <backup_path>/
//!   rocksdb/      ← RocksDB BackupEngine output
//!   storage/      ← Parquet segment files
//!   snapshots/    ← Parquet snapshot files
//!   streams/      ← Stream commit log files
//!   server.toml   ← Server configuration (optional)
//! ```
//!
//! ## Parameters Format
//! ```json
//! {
//!   "backup_path": "/backups/kalamdb-20260224"
//! }
//! ```
//!
//! ## IMPORTANT
//! Restore requires a server restart after completion to reload the restored data.

use std::{
    fs::{self, File},
    path::{Component, Path, PathBuf},
};

use async_trait::async_trait;
use flate2::read::GzDecoder;
use kalamdb_core::error::KalamDbError;
use kalamdb_system::JobType;
use serde::{Deserialize, Serialize};
use tar::Archive;
use uuid::Uuid;

use crate::executors::{JobContext, JobDecision, JobExecutor, JobParams};

/// Typed parameters for full database restore operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreParams {
    /// Source backup path on the server filesystem.
    ///
    /// The path may be a backup directory or a `.tar.gz` / `.tgz` archive
    /// created by `BACKUP DATABASE TO '<path>'`.
    pub backup_path: String,
}

impl JobParams for RestoreParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        if self.backup_path.is_empty() {
            return Err(KalamDbError::InvalidOperation("backup_path cannot be empty".to_string()));
        }
        Ok(())
    }
}

/// Restore Job Executor
///
/// Uses RocksDB's native `BackupEngine` to restore the key-value store and
/// performs a recursive directory copy for Parquet and stream files.
pub struct RestoreExecutor;

impl RestoreExecutor {
    pub fn new() -> Self {
        Self
    }

    fn is_archive_path(path: &Path) -> bool {
        let value =
            path.to_string_lossy().trim().trim_end_matches(['/', '\\']).to_ascii_lowercase();
        value.ends_with(".tar.gz") || value.ends_with(".tgz")
    }

    fn create_archive_staging_dir(archive_path: &Path) -> Result<PathBuf, KalamDbError> {
        let parent = archive_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create restore staging parent '{}': {}",
                parent.display(),
                e
            ))
        })?;

        let file_name = archive_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("kalamdb-restore");
        let staging_dir = parent.join(format!(".{}.restore-{}", file_name, Uuid::new_v4()));
        fs::create_dir_all(&staging_dir).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create restore staging directory '{}': {}",
                staging_dir.display(),
                e
            ))
        })?;

        Ok(staging_dir)
    }

    fn extract_tar_gz_archive(archive_path: &Path, output_dir: &Path) -> Result<(), KalamDbError> {
        fs::create_dir_all(output_dir).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create restore extraction directory '{}': {}",
                output_dir.display(),
                e
            ))
        })?;

        let archive_file = File::open(archive_path).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to open backup archive '{}': {}",
                archive_path.display(),
                e
            ))
        })?;
        let decoder = GzDecoder::new(archive_file);
        let mut archive = Archive::new(decoder);

        for entry in archive.entries().map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to enumerate backup archive '{}': {}",
                archive_path.display(),
                e
            ))
        })? {
            let mut entry = entry.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Archive entry error: {}", e))
            })?;

            let entry_type = entry.header().entry_type();
            if entry_type.is_symlink() || entry_type.is_hard_link() {
                return Err(KalamDbError::InvalidOperation(
                    "Backup archive cannot contain symlinks".to_string(),
                ));
            }

            let relative_path = entry.path().map_err(|e| {
                KalamDbError::InvalidOperation(format!("Invalid archive path: {}", e))
            })?;
            let relative_path = relative_path.into_owned();

            if relative_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                return Err(KalamDbError::InvalidOperation(
                    "Backup archive contains an unsafe path".to_string(),
                ));
            }

            let output_path = output_dir.join(&relative_path);
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to create restore path '{}': {}",
                        parent.display(),
                        e
                    ))
                })?;
            }

            entry.unpack(&output_path).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to extract '{}' from backup archive: {}",
                    relative_path.display(),
                    e
                ))
            })?;
        }

        Ok(())
    }

    /// Recursively copy `src` into `dst`, creating `dst` if needed.
    /// Overwrites existing files. Skips if `src` doesn't exist.
    fn copy_dir(src: &Path, dst: &Path) -> Result<(), KalamDbError> {
        if !src.exists() {
            return Ok(());
        }
        fs::create_dir_all(dst).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create directory '{}': {}",
                dst.display(),
                e
            ))
        })?;
        for entry in fs::read_dir(src).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to read directory '{}': {}",
                src.display(),
                e
            ))
        })? {
            let entry = entry.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Directory entry error: {}", e))
            })?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if src_path.is_dir() {
                Self::copy_dir(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path).map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to copy '{}' to '{}': {}",
                        src_path.display(),
                        dst_path.display(),
                        e
                    ))
                })?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl JobExecutor for RestoreExecutor {
    type Params = RestoreParams;

    fn job_type(&self) -> JobType {
        JobType::Restore
    }

    fn name(&self) -> &'static str {
        "RestoreExecutor"
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        let params = ctx.params();
        let backup_source = std::path::PathBuf::from(&params.backup_path);

        let config = ctx.app_ctx.config();
        let storage = &config.storage;

        ctx.log_info(&format!("Starting full database restore from '{}'", backup_source.display()));

        let storage_backend = ctx.app_ctx.storage_backend();

        let dst_storage = storage.storage_dir();
        let dst_snapshots = storage.resolved_snapshots_dir();
        let dst_streams = storage.streams_dir();

        let result = tokio::task::spawn_blocking(move || -> Result<(), KalamDbError> {
            let archive_input = Self::is_archive_path(&backup_source);
            let restore_root = if archive_input {
                let staging_dir = Self::create_archive_staging_dir(&backup_source)?;
                Self::extract_tar_gz_archive(&backup_source, &staging_dir)?;
                staging_dir
            } else {
                backup_source.clone()
            };

            let rocksdb_backup_dir = restore_root.join("rocksdb");

            // 1. RocksDB native restore (overwrites current data)
            if rocksdb_backup_dir.exists() {
                storage_backend.restore_from(&rocksdb_backup_dir).map_err(|e| {
                    KalamDbError::InvalidOperation(format!("RocksDB restore failed: {}", e))
                })?;
            }

            // 2. Parquet storage files
            Self::copy_dir(&restore_root.join("storage"), &dst_storage)?;

            // 3. Snapshot files
            Self::copy_dir(&restore_root.join("snapshots"), &dst_snapshots)?;

            // 4. Stream commit log files
            Self::copy_dir(&restore_root.join("streams"), &dst_streams)?;

            // 5. server.toml (optional)
            let backup_toml = restore_root.join("server.toml");
            if backup_toml.exists() {
                fs::copy(&backup_toml, Path::new("server.toml")).map_err(|e| {
                    KalamDbError::InvalidOperation(format!("Failed to restore server.toml: {}", e))
                })?;
            }

            if archive_input {
                let _ = fs::remove_dir_all(&restore_root);
            }

            Ok(())
        })
        .await
        .map_err(|e| KalamDbError::InvalidOperation(format!("Restore task panicked: {}", e)))?;

        match result {
            Ok(()) => {
                ctx.log_info(&format!(
                    "Restore staged from '{}'. RocksDB restore is pending server restart.",
                    params.backup_path
                ));
                Ok(JobDecision::Completed {
                    message: Some(format!(
                        "Restore staged from '{}'. Restart server to activate restored data.",
                        params.backup_path
                    )),
                })
            },
            Err(e) => {
                ctx.log_error(&format!("Restore failed: {}", e));
                Ok(JobDecision::Failed {
                    message: format!("Restore failed: {}", e),
                    exception_trace: Some(format!("{:?}", e)),
                })
            },
        }
    }

    async fn execute_leader(
        &self,
        ctx: &JobContext<Self::Params>,
    ) -> Result<JobDecision, KalamDbError> {
        self.execute(ctx).await
    }
}

impl Default for RestoreExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use flate2::{write::GzEncoder, Compression};
    use tar::Builder;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_executor_properties() {
        let executor = RestoreExecutor::new();
        assert_eq!(executor.job_type(), JobType::Restore);
        assert_eq!(executor.name(), "RestoreExecutor");
    }

    #[test]
    fn test_extract_tar_gz_archive_restores_backup_layout() {
        let temp_dir = tempdir().expect("temp dir");
        let source_root = temp_dir.path().join("source-root");
        fs::create_dir_all(source_root.join("rocksdb")).expect("rocksdb dir");
        fs::create_dir_all(source_root.join("storage/app/messages")).expect("storage dir");
        fs::write(source_root.join("rocksdb/CURRENT"), "manifest").expect("rocksdb file");
        fs::write(source_root.join("storage/app/messages/part-1.parquet"), "parquet")
            .expect("storage file");

        let archive_path = temp_dir.path().join("restore.tar.gz");
        let archive_file = File::create(&archive_path).expect("archive file");
        let encoder = GzEncoder::new(archive_file, Compression::default());
        let mut builder = Builder::new(encoder);
        builder
            .append_dir_all("rocksdb", source_root.join("rocksdb"))
            .expect("archive rocksdb");
        builder
            .append_dir_all("storage", source_root.join("storage"))
            .expect("archive storage");
        builder.finish().expect("finish archive");
        let encoder = builder.into_inner().expect("archive encoder");
        encoder.finish().expect("flush archive");

        let restore_root = temp_dir.path().join("restore-root");
        RestoreExecutor::extract_tar_gz_archive(&archive_path, &restore_root)
            .expect("extract archive");

        assert_eq!(
            fs::read_to_string(restore_root.join("rocksdb/CURRENT")).expect("read rocksdb file"),
            "manifest"
        );
        assert_eq!(
            fs::read_to_string(restore_root.join("storage/app/messages/part-1.parquet"))
                .expect("read storage file"),
            "parquet"
        );
    }

    #[test]
    fn test_is_archive_path_accepts_trailing_separator() {
        assert!(RestoreExecutor::is_archive_path(Path::new("/tmp/kalamdb.tar.gz/")));
        assert!(RestoreExecutor::is_archive_path(Path::new("/tmp/kalamdb.tgz/")));
    }
}
