use std::{fs, path::{Path, PathBuf}};

use kalamdb_core::error::KalamDbError;

use crate::{create_archive_staging_dir, create_tar_gz_archive, is_tar_gz_path};

pub struct DatabaseBackupPlan {
    pub backup_root: PathBuf,
    archive_output: bool,
}

impl DatabaseBackupPlan {
    pub fn rocksdb_dir(&self) -> PathBuf {
        self.backup_root.join("rocksdb")
    }

    pub fn storage_dir(&self) -> PathBuf {
        self.backup_root.join("storage")
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.backup_root.join("snapshots")
    }

    pub fn streams_dir(&self) -> PathBuf {
        self.backup_root.join("streams")
    }

    pub fn server_toml_path(&self) -> PathBuf {
        self.backup_root.join("server.toml")
    }
}

pub fn prepare_database_backup(backup_target: &Path) -> Result<DatabaseBackupPlan, KalamDbError> {
    let archive_output = is_tar_gz_path(backup_target);
    let backup_root = if archive_output {
        create_archive_staging_dir(backup_target, "staging")?
    } else {
        backup_target.to_path_buf()
    };

    fs::create_dir_all(&backup_root).map_err(|error| {
        KalamDbError::InvalidOperation(format!(
            "Failed to create backup root '{}': {}",
            backup_root.display(),
            error
        ))
    })?;

    Ok(DatabaseBackupPlan {
        backup_root,
        archive_output,
    })
}

pub fn finalize_database_backup(
    plan: DatabaseBackupPlan,
    backup_target: &Path,
    total_bytes: u64,
) -> Result<u64, KalamDbError> {
    if plan.archive_output {
        create_tar_gz_archive(&plan.backup_root, backup_target)?;
        let archive_size = backup_target.metadata().map(|metadata| metadata.len()).map_err(|error| {
            KalamDbError::InvalidOperation(format!(
                "Failed to inspect archive '{}': {}",
                backup_target.display(),
                error
            ))
        })?;
        let _ = fs::remove_dir_all(&plan.backup_root);
        Ok(archive_size)
    } else {
        Ok(total_bytes)
    }
}
