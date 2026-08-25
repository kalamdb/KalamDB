//! Startup helpers shared by the production binary and test server wiring.

use std::sync::Arc;

use anyhow::Result;
use kalamdb_commons::StorageId;
use kalamdb_configs::ServerConfig;
use kalamdb_core::app_context::AppContext;
use kalamdb_system::{providers::storages::models::StorageType, Storage};
use log::{debug, info};

/// Initialize global auth/proxy runtime state from the already-finalized config.
pub fn configure_auth_runtime(config: &ServerConfig) -> Result<()> {
    kalamdb_auth::services::unified::init_auth_config(&config.auth);
    kalamdb_auth::init_trusted_proxy_ranges(&config.security.trusted_proxy_ranges)?;
    Ok(())
}

/// Ensure the default local storage row exists without materializing an Arrow batch.
pub fn create_default_storage_if_needed(
    config: &ServerConfig,
    app_context: &Arc<AppContext>,
) -> Result<usize> {
    let storages_provider = app_context.system_tables().storages();
    let storage_count = storages_provider.list_storages()?.len();

    if storage_count > 0 {
        debug!("Found {} existing storage(s)", storage_count);
        return Ok(storage_count);
    }

    info!("No storages found, creating default 'local' storage");
    let now = chrono::Utc::now().timestamp_millis();
    let default_storage = Storage {
        storage_id:             StorageId::from("local"),
        storage_name:           "Local Filesystem".to_string(),
        description:            Some("Default local filesystem storage".to_string()),
        storage_type:           StorageType::Filesystem,
        base_directory:         config.storage.storage_dir().to_string_lossy().into_owned(),
        credentials:            None,
        config_json:            None,
        shared_tables_template: config.storage.shared_tables_template.clone(),
        user_tables_template:   config.storage.user_tables_template.clone(),
        created_at:             now,
        updated_at:             now,
    };
    storages_provider.insert_storage(default_storage)?;
    info!("Default 'local' storage created successfully");

    Ok(0)
}
