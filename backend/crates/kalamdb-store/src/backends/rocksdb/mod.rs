//! RocksDB backend implementation and supporting helpers.

mod backend;
mod cf_tuning;
mod init;
mod keyspace;
mod restore;
pub mod test_utils;

use std::{path::Path, sync::Arc};

pub use backend::RocksDBBackend;
pub use init::RocksDbInit;

use crate::storage_trait::StorageBackend;

/// Open the RocksDB storage backend and return it through the generic storage trait.
pub fn open_storage_backend(
    db_path: &Path,
    settings: &kalamdb_configs::RocksDbSettings,
) -> anyhow::Result<(Arc<dyn StorageBackend>, usize)> {
    let db_init = RocksDbInit::new(db_path.to_string_lossy().into_owned(), settings.clone());
    let settings = db_init.settings().clone();
    let (db, cf_names, block_cache) = db_init.open_with_cf_names_and_cache()?;
    let backend = Arc::new(RocksDBBackend::with_options_settings_and_cache(
        db,
        settings.sync_writes,
        settings.disable_wal,
        settings,
        block_cache,
    ));
    backend.set_known_cf_names(cf_names.clone());
    Ok((backend, cf_names.len()))
}
