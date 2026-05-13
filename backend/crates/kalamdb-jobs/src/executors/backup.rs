//! Backup Job Executor
//!
//! Backs up the entire KalamDB database using RocksDB's native `BackupEngine`
//! for the key-value store, plus a recursive directory copy for Parquet storage,
//! snapshots, streams, and the server.toml configuration file.
//!
//! When the target path ends with `.tar.gz` or `.tgz`, the executor writes a
//! single archive file containing the following layout. Otherwise the same
//! layout is written directly into the target directory.
//!
//! ## Backup Layout
//! ```text
//! <backup_path>/
//!   rocksdb/      ← RocksDB BackupEngine output (hot, consistent, deduplicated)
//!   storage/      ← Parquet segment files
//!   snapshots/    ← Parquet snapshot files
//!   streams/      ← Stream commit log files
//!   server.toml   ← Server configuration (if present next to the binary)
//! ```
//!
//! ## Parameters Format
//! ```json
//! {
//!   "backup_path": "/backups/kalamdb-20260224"
//! }
//! ```

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use flate2::{write::GzEncoder, Compression};
use kalamdb_core::error::KalamDbError;
use kalamdb_system::JobType;
use serde::{Deserialize, Serialize};
use tar::Builder;
use uuid::Uuid;

use crate::executors::{JobContext, JobDecision, JobExecutor, JobParams};

/// Typed parameters for full database backup operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupParams {
    /// Destination backup path on the server filesystem.
    ///
    /// If the path ends with `.tar.gz` or `.tgz`, KalamDB writes a single
    /// archive file. Otherwise a backup directory is created and the layout is
    /// written under `<backup_path>/rocksdb`, `<backup_path>/storage`, and so on.
    pub backup_path: String,
}

impl JobParams for BackupParams {
    fn validate(&self) -> Result<(), KalamDbError> {
        if self.backup_path.is_empty() {
            return Err(KalamDbError::InvalidOperation("backup_path cannot be empty".to_string()));
        }
        Ok(())
    }
}

/// Backup Job Executor
///
/// Uses RocksDB's native `BackupEngine` for the key-value store and performs a
/// recursive directory copy for Parquet and stream files.
pub struct BackupExecutor;

impl BackupExecutor {
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
                "Failed to create archive parent directory '{}': {}",
                parent.display(),
                e
            ))
        })?;

        let file_name = archive_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("kalamdb-backup");
        let staging_dir = parent.join(format!(".{}.staging-{}", file_name, Uuid::new_v4()));

        fs::create_dir_all(&staging_dir).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create backup staging directory '{}': {}",
                staging_dir.display(),
                e
            ))
        })?;

        Ok(staging_dir)
    }

    fn create_tar_gz_archive(src_dir: &Path, archive_path: &Path) -> Result<(), KalamDbError> {
        if let Some(parent) = archive_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to create archive output directory '{}': {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        if archive_path.exists() {
            if archive_path.is_dir() {
                return Err(KalamDbError::InvalidOperation(format!(
                    "Archive output path '{}' is a directory",
                    archive_path.display()
                )));
            }

            fs::remove_file(archive_path).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to replace existing archive '{}': {}",
                    archive_path.display(),
                    e
                ))
            })?;
        }

        let archive_file = File::create(archive_path).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create archive '{}': {}",
                archive_path.display(),
                e
            ))
        })?;
        let encoder = GzEncoder::new(archive_file, Compression::default());
        let mut builder = Builder::new(encoder);

        for entry in fs::read_dir(src_dir).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to read archive staging directory '{}': {}",
                src_dir.display(),
                e
            ))
        })? {
            let entry = entry.map_err(|e| {
                KalamDbError::InvalidOperation(format!("Directory entry error: {}", e))
            })?;
            let src_path = entry.path();
            let name = entry.file_name();
            let archive_name = Path::new(name.as_os_str());

            if src_path.is_dir() {
                builder.append_dir_all(archive_name, &src_path).map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to archive directory '{}': {}",
                        src_path.display(),
                        e
                    ))
                })?;
            } else {
                let mut file = File::open(&src_path).map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to open archive input '{}': {}",
                        src_path.display(),
                        e
                    ))
                })?;
                builder.append_file(archive_name, &mut file).map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to archive file '{}': {}",
                        src_path.display(),
                        e
                    ))
                })?;
            }
        }

        builder.finish().map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to finish archive '{}': {}",
                archive_path.display(),
                e
            ))
        })?;
        let encoder = builder.into_inner().map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to finalize archive '{}': {}",
                archive_path.display(),
                e
            ))
        })?;
        encoder.finish().map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to flush archive '{}': {}",
                archive_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Recursively copy `src` into `dst`, creating `dst` if necessary.
    fn copy_dir(src: &Path, dst: &Path) -> Result<u64, KalamDbError> {
        if !src.exists() {
            return Ok(0);
        }
        fs::create_dir_all(dst).map_err(|e| {
            KalamDbError::InvalidOperation(format!(
                "Failed to create directory '{}': {}",
                dst.display(),
                e
            ))
        })?;
        let mut bytes_copied: u64 = 0;
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
                bytes_copied += Self::copy_dir(&src_path, &dst_path)?;
            } else {
                fs::copy(&src_path, &dst_path).map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to copy '{}' to '{}': {}",
                        src_path.display(),
                        dst_path.display(),
                        e
                    ))
                })?;
                bytes_copied += src_path.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        Ok(bytes_copied)
    }
}

#[async_trait]
impl JobExecutor for BackupExecutor {
    type Params = BackupParams;

    fn job_type(&self) -> JobType {
        JobType::Backup
    }

    fn name(&self) -> &'static str {
        "BackupExecutor"
    }

    async fn execute(&self, ctx: &JobContext<Self::Params>) -> Result<JobDecision, KalamDbError> {
        let params = ctx.params();
        let backup_target = std::path::PathBuf::from(&params.backup_path);

        let config = ctx.app_ctx.config();
        let storage = &config.storage;

        ctx.log_info(&format!("Starting full database backup to '{}'", backup_target.display()));

        let storage_backend = ctx.app_ctx.storage_backend();
        let src_storage = storage.storage_dir();
        let src_snapshots = storage.resolved_snapshots_dir();
        let src_streams = storage.streams_dir();

        let result = tokio::task::spawn_blocking(move || -> Result<u64, KalamDbError> {
            let archive_output = Self::is_archive_path(&backup_target);
            let backup_root = if archive_output {
                Self::create_archive_staging_dir(&backup_target)?
            } else {
                backup_target.clone()
            };

            fs::create_dir_all(&backup_root).map_err(|e| {
                KalamDbError::InvalidOperation(format!(
                    "Failed to create backup root '{}': {}",
                    backup_root.display(),
                    e
                ))
            })?;

            // 1. RocksDB native backup (hot, consistent, with flush)
            storage_backend.backup_to(&backup_root.join("rocksdb")).map_err(|e| {
                KalamDbError::InvalidOperation(format!("RocksDB backup failed: {}", e))
            })?;

            let mut total_bytes: u64 = 0;

            // 2. Parquet storage files
            total_bytes += Self::copy_dir(&src_storage, &backup_root.join("storage"))?;

            // 3. Snapshot files
            total_bytes += Self::copy_dir(&src_snapshots, &backup_root.join("snapshots"))?;

            // 4. Stream commit log files
            total_bytes += Self::copy_dir(&src_streams, &backup_root.join("streams"))?;

            // 5. server.toml (look next to the binary / CWD)
            let server_toml = Path::new("server.toml");
            if server_toml.exists() {
                fs::copy(server_toml, backup_root.join("server.toml")).map_err(|e| {
                    KalamDbError::InvalidOperation(format!("Failed to copy server.toml: {}", e))
                })?;
                total_bytes += server_toml.metadata().map(|m| m.len()).unwrap_or(0);
            }

            if archive_output {
                Self::create_tar_gz_archive(&backup_root, &backup_target)?;
                let archive_size = backup_target.metadata().map(|m| m.len()).map_err(|e| {
                    KalamDbError::InvalidOperation(format!(
                        "Failed to inspect archive '{}': {}",
                        backup_target.display(),
                        e
                    ))
                })?;
                let _ = fs::remove_dir_all(&backup_root);
                Ok(archive_size)
            } else {
                Ok(total_bytes)
            }
        })
        .await
        .map_err(|e| KalamDbError::InvalidOperation(format!("Backup task panicked: {}", e)))?;

        match result {
            Ok(total_bytes) => {
                let size_mb = total_bytes as f64 / (1024.0 * 1024.0);
                ctx.log_info(&format!(
                    "Backup completed: '{}' ({:.2} MB written)",
                    params.backup_path, size_mb
                ));
                Ok(JobDecision::Completed {
                    message: Some(format!(
                        "Backup completed: '{}' ({:.2} MB)",
                        params.backup_path, size_mb
                    )),
                })
            },
            Err(e) => {
                ctx.log_error(&format!("Backup failed: {}", e));
                Ok(JobDecision::Failed {
                    message: format!("Backup failed: {}", e),
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

impl Default for BackupExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use flate2::read::GzDecoder;
    use kalamdb_core::test_helpers::test_app_context_simple;
    use tar::Archive;
    use tempfile::tempdir;

    use super::*;
    use crate::executors::{JobContext, JobDecision};

    #[test]
    fn test_executor_properties() {
        let executor = BackupExecutor::new();
        assert_eq!(executor.job_type(), JobType::Backup);
        assert_eq!(executor.name(), "BackupExecutor");
    }

    #[test]
    fn test_create_tar_gz_archive_preserves_backup_layout() {
        let temp_dir = tempdir().expect("temp dir");
        let source_root = temp_dir.path().join("backup-root");
        fs::create_dir_all(source_root.join("rocksdb")).expect("rocksdb dir");
        fs::create_dir_all(source_root.join("storage/app/messages")).expect("storage dir");
        fs::write(source_root.join("rocksdb/CURRENT"), "manifest").expect("rocksdb file");
        fs::write(source_root.join("storage/app/messages/part-1.parquet"), "parquet")
            .expect("storage file");
        fs::write(source_root.join("server.toml"), "port = 2900\n").expect("config file");

        let archive_path = temp_dir.path().join("backup.tar.gz");
        BackupExecutor::create_tar_gz_archive(&source_root, &archive_path)
            .expect("archive creation");

        let extract_root = temp_dir.path().join("extract");
        fs::create_dir_all(&extract_root).expect("extract root");
        let archive_file = File::open(&archive_path).expect("open archive");
        let decoder = GzDecoder::new(archive_file);
        let mut archive = Archive::new(decoder);
        archive.unpack(&extract_root).expect("unpack archive");

        assert_eq!(
            fs::read_to_string(extract_root.join("rocksdb/CURRENT")).expect("read rocksdb file"),
            "manifest"
        );
        assert_eq!(
            fs::read_to_string(extract_root.join("storage/app/messages/part-1.parquet"))
                .expect("read storage file"),
            "parquet"
        );
        assert_eq!(
            fs::read_to_string(extract_root.join("server.toml")).expect("read config file"),
            "port = 2900\n"
        );
    }

    #[test]
    fn test_is_archive_path_accepts_trailing_separator() {
        assert!(BackupExecutor::is_archive_path(Path::new("/tmp/kalamdb.tar.gz/")));
        assert!(BackupExecutor::is_archive_path(Path::new("/tmp/kalamdb.tgz/")));
    }

    #[tokio::test]
    async fn test_execute_writes_archive_for_archive_path() {
        let app_ctx = test_app_context_simple();
        let temp_dir = tempdir().expect("temp dir");
        let archive_path = temp_dir.path().join("kalamdb-backup.tar.gz");
        let params = BackupParams {
            backup_path: archive_path.to_string_lossy().to_string(),
        };
        let ctx = JobContext::new(app_ctx, "BK-test-archive".to_string(), params);

        let decision = BackupExecutor::new().execute(&ctx).await.expect("backup execute");
        assert!(matches!(decision, JobDecision::Completed { .. }));
        assert!(archive_path.is_file(), "expected archive file at {}", archive_path.display());
    }
}
