use std::{fs, path::{Path, PathBuf}};

use kalamdb_core::error::KalamDbError;

use crate::{create_archive_staging_dir, extract_tar_gz_archive, is_tar_gz_path};

pub struct DatabaseRestorePlan {
    pub restore_root: PathBuf,
    archive_input: bool,
}

impl DatabaseRestorePlan {
    pub fn rocksdb_dir(&self) -> PathBuf {
        self.restore_root.join("rocksdb")
    }

    pub fn storage_dir(&self) -> PathBuf {
        self.restore_root.join("storage")
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.restore_root.join("snapshots")
    }

    pub fn streams_dir(&self) -> PathBuf {
        self.restore_root.join("streams")
    }

    pub fn server_toml_path(&self) -> PathBuf {
        self.restore_root.join("server.toml")
    }
}

pub fn prepare_database_restore(backup_source: &Path) -> Result<DatabaseRestorePlan, KalamDbError> {
    let archive_input = is_tar_gz_path(backup_source);
    let restore_root = if archive_input {
        let staging_dir = create_archive_staging_dir(backup_source, "restore")?;
        extract_tar_gz_archive(backup_source, &staging_dir)?;
        staging_dir
    } else {
        backup_source.to_path_buf()
    };

    Ok(DatabaseRestorePlan {
        restore_root,
        archive_input,
    })
}

pub fn finalize_database_restore(plan: DatabaseRestorePlan) {
    if plan.archive_input {
        let _ = fs::remove_dir_all(&plan.restore_root);
    }
}
