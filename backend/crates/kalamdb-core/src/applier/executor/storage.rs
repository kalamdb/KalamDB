//! Storage Executor - CREATE/DROP STORAGE operations
//!
//! This is the SINGLE place where storage mutations happen.
//! All methods use spawn_blocking to avoid blocking the tokio runtime
//! with synchronous RocksDB calls.

use std::{env, fs, path::Path, sync::Arc};

use kalamdb_commons::models::{schemas::TableOptions, StorageId};
use kalamdb_system::{Storage, StorageType};

use crate::{
    app_context::AppContext,
    applier::{executor::utils::run_blocking_applier, ApplierError},
};

/// Executor for storage operations
pub struct StorageExecutor {
    app_context: Arc<AppContext>,
}

impl StorageExecutor {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    /// Execute CREATE STORAGE
    pub async fn create_storage(&self, storage: &Storage) -> Result<String, ApplierError> {
        log::info!("CommandExecutorImpl: Creating storage {}", storage.storage_id);
        let app_context = self.app_context.clone();
        let storage = storage.clone();
        run_blocking_applier(move || {
            let storage_id = storage.storage_id.clone();
            app_context
                .system_tables()
                .storages()
                .create_storage(storage)
                .map_err(|e| ApplierError::Execution(format!("Failed to create storage: {}", e)))?;
            Ok(format!("Storage {} created successfully", storage_id))
        })
        .await
    }

    /// Execute DROP STORAGE
    pub async fn drop_storage(&self, storage_id: &StorageId) -> Result<String, ApplierError> {
        log::info!("CommandExecutorImpl: Dropping storage {}", storage_id);
        let app_context = self.app_context.clone();
        let storage_id = storage_id.clone();
        run_blocking_applier(move || {
            let storage =
                app_context.system_tables().storages().get_storage(&storage_id).map_err(|e| {
                    ApplierError::Execution(format!("Failed to read storage: {}", e))
                })?;
            app_context
                .system_tables()
                .storages()
                .delete_storage(&storage_id)
                .map_err(|e| ApplierError::Execution(format!("Failed to drop storage: {}", e)))?;
            if let Some(storage) = storage {
                release_filesystem_storage(&app_context, &storage);
            }
            Ok(format!("Storage {} dropped successfully", storage_id))
        })
        .await
    }
}

/// Delete a dropped filesystem storage directory once nothing still references it.
///
/// Catalog removal alone leaves Parquet segments on the configured path. The next
/// process that recreates the same path treats those files as live cold rows.
fn release_filesystem_storage(app_context: &AppContext, storage: &Storage) {
    if storage.storage_type != StorageType::Filesystem {
        return;
    }
    if storage_still_referenced(app_context, &storage.storage_id) {
        log::info!(
            "Storage {} still backs a table; leaving filesystem data in place",
            storage.storage_id
        );
        return;
    }
    let Some(path) = removable_storage_dir(&storage.base_directory) else {
        log::info!(
            "Storage {} path '{}' is not a removable directory",
            storage.storage_id,
            storage.base_directory
        );
        return;
    };
    match fs::remove_dir_all(&path) {
        Ok(()) => log::info!(
            "Removed filesystem data for storage {} at {}",
            storage.storage_id,
            path.display()
        ),
        Err(error) => log::warn!(
            "Failed to remove filesystem data for storage {} at {}: {}",
            storage.storage_id,
            path.display(),
            error
        ),
    }
}

fn storage_still_referenced(app_context: &AppContext, storage_id: &StorageId) -> bool {
    match app_context.schema_registry().scan_all_table_definitions() {
        Ok(tables) => tables.iter().any(|table| match &table.table_options {
            TableOptions::User(options) => &options.storage_id == storage_id,
            TableOptions::Shared(options) => &options.storage_id == storage_id,
            TableOptions::Stream(_) | TableOptions::System(_) => false,
        }),
        Err(error) => {
            log::warn!(
                "Could not check storage {} references; leaving files in place: {}",
                storage_id,
                error
            );
            true
        },
    }
}

fn removable_storage_dir(base_directory: &str) -> Option<std::path::PathBuf> {
    let trimmed = base_directory.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." || trimmed == "/" {
        return None;
    }
    let path = Path::new(trimmed);
    if !path.is_dir() {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    if canonical.parent().is_none() {
        return None;
    }
    let cwd = env::current_dir().ok()?;
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    if canonical == cwd || cwd.starts_with(&canonical) {
        return None;
    }
    if let Ok(data_dir) = env::var("KALAMDB_DATA_DIR") {
        let data_dir = std::path::PathBuf::from(data_dir);
        if let Ok(data_dir) = data_dir.canonicalize() {
            if canonical == data_dir || data_dir.starts_with(&canonical) {
                return None;
            }
        }
    }
    Some(canonical)
}
