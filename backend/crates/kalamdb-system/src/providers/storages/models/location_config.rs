use serde::{Deserialize, Serialize};

use super::{AzureStorageConfig, GcsStorageConfig, LocalStorageConfig, S3StorageConfig};

/// Type-safe JSON configuration for `system.storages` locations.
///
/// Stored as raw JSON text in the `config_json` column, but can be decoded
/// into this strongly typed enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StorageLocationConfig {
    Local(LocalStorageConfig),
    S3(S3StorageConfig),
    Gcs(GcsStorageConfig),
    Azure(AzureStorageConfig),
}

impl StorageLocationConfig {}
