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

use std::{fs, path::Path};

use async_trait::async_trait;
use kalamdb_core::error::KalamDbError;
use kalamdb_system::JobType;
use kalamdb_transfer::{
    copy_dir_to_dir, finalize_database_backup, prepare_database_backup,
};
use serde::{Deserialize, Serialize};

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
            let plan = prepare_database_backup(&backup_target)?;

            // 1. RocksDB native backup (hot, consistent, with flush)
            storage_backend.backup_to(&plan.rocksdb_dir()).map_err(|e| {
                KalamDbError::InvalidOperation(format!("RocksDB backup failed: {}", e))
            })?;

            let mut total_bytes: u64 = 0;

            // 2. Parquet storage files
            total_bytes += copy_dir_to_dir(&src_storage, &plan.storage_dir())?;

            // 3. Snapshot files
            total_bytes += copy_dir_to_dir(&src_snapshots, &plan.snapshots_dir())?;

            // 4. Stream commit log files
            total_bytes += copy_dir_to_dir(&src_streams, &plan.streams_dir())?;

            // 5. server.toml (look next to the binary / CWD)
            let server_toml = Path::new("server.toml");
            if server_toml.exists() {
                fs::copy(server_toml, plan.server_toml_path()).map_err(|e| {
                    KalamDbError::InvalidOperation(format!("Failed to copy server.toml: {}", e))
                })?;
                total_bytes += server_toml.metadata().map(|m| m.len()).unwrap_or(0);
            }

            finalize_database_backup(plan, &backup_target, total_bytes)
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
    use std::fs::File;

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
        kalamdb_transfer::create_tar_gz_archive(&source_root, &archive_path)
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
        assert!(kalamdb_transfer::is_tar_gz_path(Path::new("/tmp/kalamdb.tar.gz/")));
        assert!(kalamdb_transfer::is_tar_gz_path(Path::new("/tmp/kalamdb.tgz/")));
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
