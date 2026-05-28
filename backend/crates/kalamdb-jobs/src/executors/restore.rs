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
    fs,
    path::Path,
};

use async_trait::async_trait;
use kalamdb_core::error::KalamDbError;
use kalamdb_system::JobType;
use kalamdb_transfer::{
    copy_dir_to_dir, finalize_database_restore, prepare_database_restore,
};
use serde::{Deserialize, Serialize};

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
            let plan = prepare_database_restore(&backup_source)?;
            let rocksdb_backup_dir = plan.rocksdb_dir();

            // 1. RocksDB native restore (overwrites current data)
            if rocksdb_backup_dir.exists() {
                storage_backend.restore_from(&rocksdb_backup_dir).map_err(|e| {
                    KalamDbError::InvalidOperation(format!("RocksDB restore failed: {}", e))
                })?;
            }

            // 2. Parquet storage files
            copy_dir_to_dir(&plan.storage_dir(), &dst_storage)?;

            // 3. Snapshot files
            copy_dir_to_dir(&plan.snapshots_dir(), &dst_snapshots)?;

            // 4. Stream commit log files
            copy_dir_to_dir(&plan.streams_dir(), &dst_streams)?;

            // 5. server.toml (optional)
            let backup_toml = plan.server_toml_path();
            if backup_toml.exists() {
                fs::copy(&backup_toml, Path::new("server.toml")).map_err(|e| {
                    KalamDbError::InvalidOperation(format!("Failed to restore server.toml: {}", e))
                })?;
            }

            finalize_database_restore(plan);

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
    use std::fs::File;

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
        kalamdb_transfer::extract_tar_gz_archive(&archive_path, &restore_root)
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
        assert!(kalamdb_transfer::is_tar_gz_path(Path::new("/tmp/kalamdb.tar.gz/")));
        assert!(kalamdb_transfer::is_tar_gz_path(Path::new("/tmp/kalamdb.tgz/")));
    }
}
