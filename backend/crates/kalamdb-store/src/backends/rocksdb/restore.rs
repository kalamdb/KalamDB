//! Pending RocksDB restore promotion and leftover cleanup.
//!
//! `restore_from` cannot overwrite an open live database, so it writes a sibling
//! staging directory. On the next open, that directory is swapped onto the live
//! path and leftover staging dirs are deleted.
//!
//! Staging dirs are only promoted when they include a `.kalamdb_restore_ready`
//! marker written after BackupEngine finishes. Unmarked leftovers are deleted.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

const RESTORE_PENDING_INFIX: &str = "_restore_pending_";
const RESTORE_REPLACED_SUFFIX: &str = "_replaced";
const ROCKSDB_CURRENT_FILE: &str = "CURRENT";
const RESTORE_READY_MARKER: &str = ".kalamdb_restore_ready";

/// Staging directory used by [`crate::storage_trait::StorageBackend::restore_from`].
pub(crate) fn restore_staging_path(db_path: &Path, restore_token: &str) -> PathBuf {
    sibling_with_suffix(db_path, &format!("{RESTORE_PENDING_INFIX}{restore_token}"))
}

/// Marks a staged restore as complete so the next open will promote it.
pub(crate) fn mark_restore_staging_ready(staging_path: &Path) -> anyhow::Result<()> {
    fs::write(staging_path.join(RESTORE_READY_MARKER), b"ready").map_err(|e| {
        anyhow::anyhow!(
            "failed to mark restore staging dir '{}' ready: {}",
            staging_path.display(),
            e
        )
    })
}

/// Promote the newest complete staged restore onto `db_path` and delete leftovers.
///
/// Complete staging dirs contain a RocksDB `CURRENT` file and a
/// `.kalamdb_restore_ready` marker written after BackupEngine finishes.
/// Unmarked leftover dirs (including pre-fix staging copies) are deleted
/// without replacing the live database. After a successful swap, the previous
/// live directory (temporarily renamed with a `_replaced` suffix) is deleted.
///
/// # Errors
///
/// Returns an error if a staging directory cannot be listed, renamed, or removed.
pub(crate) fn apply_pending_rocksdb_restore(db_path: &Path) -> anyhow::Result<()> {
    let pending_dirs = list_pending_restore_dirs(db_path)?;
    let mut complete = Vec::with_capacity(pending_dirs.len());
    for dir in pending_dirs {
        if is_ready_pending_restore_dir(&dir) {
            complete.push(dir);
        } else {
            log::warn!("removing incomplete rocksdb restore staging dir {}", dir.display());
            remove_dir_all_if_exists(&dir)?;
        }
    }

    if let Some(winner) = newest_complete_dir(&complete) {
        for dir in &complete {
            if dir != &winner {
                log::info!("removing superseded rocksdb restore staging dir {}", dir.display());
                remove_dir_all_if_exists(dir)?;
            }
        }
        promote_staging_dir(db_path, &winner)?;
    }

    if is_complete_rocksdb_dir(db_path) {
        remove_dir_all_if_exists(&sibling_with_suffix(db_path, RESTORE_REPLACED_SUFFIX))?;
    }

    Ok(())
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

fn restore_pending_name_prefix(db_path: &Path) -> Option<String> {
    db_path
        .file_name()
        .map(|name| format!("{}{RESTORE_PENDING_INFIX}", name.to_string_lossy()))
}

fn list_pending_restore_dirs(db_path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let Some(prefix) = restore_pending_name_prefix(db_path) else {
        return Ok(Vec::new());
    };
    let parent = match db_path.parent() {
        Some(parent) if parent.as_os_str().is_empty() => Path::new("."),
        Some(parent) => parent,
        None => return Ok(Vec::new()),
    };
    if !parent.exists() {
        return Ok(Vec::new());
    }

    let mut dirs = Vec::new();
    for entry in fs::read_dir(parent).map_err(|e| {
        anyhow::anyhow!(
            "failed to list rocksdb restore staging dirs in '{}': {}",
            parent.display(),
            e
        )
    })? {
        let path = entry
            .map_err(|e| {
                anyhow::anyhow!(
                    "failed to read rocksdb restore staging entry in '{}': {}",
                    parent.display(),
                    e
                )
            })?
            .path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        if name.to_string_lossy().starts_with(&prefix) {
            dirs.push(path);
        }
    }
    Ok(dirs)
}

fn is_complete_rocksdb_dir(path: &Path) -> bool {
    path.is_dir() && path.join(ROCKSDB_CURRENT_FILE).is_file()
}

fn is_ready_pending_restore_dir(path: &Path) -> bool {
    is_complete_rocksdb_dir(path) && path.join(RESTORE_READY_MARKER).is_file()
}

fn newest_complete_dir(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| {
            let modified = fs::metadata(dir)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, dir)
        })
        .max_by(|(left_time, left_path), (right_time, right_path)| {
            left_time
                .cmp(right_time)
                .then_with(|| left_path.as_os_str().cmp(right_path.as_os_str()))
        })
        .map(|(_, dir)| dir.clone())
}

fn promote_staging_dir(db_path: &Path, staging: &Path) -> anyhow::Result<()> {
    let replaced = sibling_with_suffix(db_path, RESTORE_REPLACED_SUFFIX);
    remove_dir_all_if_exists(&replaced)?;

    if db_path.exists() {
        if is_complete_rocksdb_dir(db_path) {
            fs::rename(db_path, &replaced).map_err(|e| {
                anyhow::anyhow!(
                    "failed to move live rocksdb dir '{}' aside before restore: {}",
                    db_path.display(),
                    e
                )
            })?;
        } else {
            remove_dir_all_if_exists(db_path)?;
        }
    }

    fs::rename(staging, db_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to promote pending rocksdb restore '{}' onto '{}': {}",
            staging.display(),
            db_path.display(),
            e
        )
    })?;
    log::info!(
        "promoted pending rocksdb restore from {} onto {}",
        staging.display(),
        db_path.display()
    );
    remove_dir_all_if_exists(&replaced)?;
    Ok(())
}

fn remove_dir_all_if_exists(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path)
        .map_err(|e| anyhow::anyhow!("failed to remove '{}': {}", path.display(), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn write_current(dir: &Path, contents: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(ROCKSDB_CURRENT_FILE), contents).unwrap();
    }

    fn write_ready_staging(dir: &Path, contents: &str) {
        write_current(dir, contents);
        mark_restore_staging_ready(dir).unwrap();
    }

    fn current_contents(dir: &Path) -> String {
        fs::read_to_string(dir.join(ROCKSDB_CURRENT_FILE)).unwrap()
    }

    #[test]
    fn apply_pending_restore_is_noop_when_no_staging_dirs() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("rocksdb");
        write_current(&db_path, "live");

        apply_pending_rocksdb_restore(&db_path).unwrap();

        assert_eq!(current_contents(&db_path), "live");
        assert!(db_path.exists());
    }

    #[test]
    fn apply_pending_restore_promotes_staging_dir_onto_live_path() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("rocksdb");
        write_current(&db_path, "old-live");
        let staging = restore_staging_path(&db_path, "RS-new");
        write_ready_staging(&staging, "restored");

        apply_pending_rocksdb_restore(&db_path).unwrap();

        assert_eq!(current_contents(&db_path), "restored");
        assert!(!staging.exists());
        assert!(!sibling_with_suffix(&db_path, RESTORE_REPLACED_SUFFIX).exists());
    }

    #[test]
    fn apply_pending_restore_keeps_newest_complete_staging_dir() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("rocksdb");
        write_current(&db_path, "old-live");

        let older = restore_staging_path(&db_path, "RS-old");
        write_ready_staging(&older, "older-restore");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newer = restore_staging_path(&db_path, "RS-new");
        write_ready_staging(&newer, "newer-restore");

        apply_pending_rocksdb_restore(&db_path).unwrap();

        assert_eq!(current_contents(&db_path), "newer-restore");
        assert!(!older.exists());
        assert!(!newer.exists());
    }

    #[test]
    fn apply_pending_restore_deletes_incomplete_staging_dirs() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("rocksdb");
        write_current(&db_path, "live");
        let incomplete = restore_staging_path(&db_path, "RS-partial");
        fs::create_dir_all(&incomplete).unwrap();
        fs::write(incomplete.join("MANIFEST-1"), "partial").unwrap();

        apply_pending_rocksdb_restore(&db_path).unwrap();

        assert_eq!(current_contents(&db_path), "live");
        assert!(!incomplete.exists());
    }

    #[test]
    fn apply_pending_restore_replaces_empty_live_directory() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("rocksdb");
        fs::create_dir_all(&db_path).unwrap();
        let staging = restore_staging_path(&db_path, "RS-new");
        write_ready_staging(&staging, "restored");

        apply_pending_rocksdb_restore(&db_path).unwrap();

        assert_eq!(current_contents(&db_path), "restored");
        assert!(!staging.exists());
    }

    #[test]
    fn apply_pending_restore_deletes_leftover_replaced_dir_when_live_is_complete() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("rocksdb");
        write_current(&db_path, "live");
        let replaced = sibling_with_suffix(&db_path, RESTORE_REPLACED_SUFFIX);
        write_current(&replaced, "old-replaced");

        apply_pending_rocksdb_restore(&db_path).unwrap();

        assert_eq!(current_contents(&db_path), "live");
        assert!(!replaced.exists());
    }

    #[test]
    fn apply_pending_restore_deletes_unmarked_leftover_staging_dirs() {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("rocksdb");
        write_current(&db_path, "live");
        let leftover = restore_staging_path(&db_path, "RS-old");
        write_current(&leftover, "old-unmarked-restore");

        apply_pending_rocksdb_restore(&db_path).unwrap();

        assert_eq!(current_contents(&db_path), "live");
        assert!(!leftover.exists());
    }
}
