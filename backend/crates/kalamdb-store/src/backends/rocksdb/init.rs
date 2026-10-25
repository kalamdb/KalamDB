//! RocksDB initialization utilities for KalamDB.
//!
//! Provides a thin helper to open a RocksDB instance with required
//! fixed physical column families present.

use std::{path::Path, sync::Arc};

use anyhow::Result;
use kalamdb_configs::RocksDbSettings;
use rocksdb::{
    BlockBasedOptions, Cache, ColumnFamilyDescriptor, DataBlockIndexType, Options,
    WriteBufferManager, DB,
};
use sysinfo::{MemoryRefreshKind, System};

use super::{
    cf_tuning::{apply_cf_settings, apply_db_settings, install_write_buffer_manager},
    keyspace::fixed_column_families,
    restore::apply_pending_rocksdb_restore,
};

/// Print the mapping that contains the open thread's stack pointer.
///
/// `pthread_attr_setstacksize` can succeed while glibc still installs the
/// default 8MB stack. The map size is the stack DB::Open actually gets.
#[cfg(target_os = "linux")]
fn log_open_thread_stack() {
    let here = 0u8;
    let sp = &here as *const u8 as usize;
    let Ok(maps) = std::fs::read_to_string("/proc/self/maps") else {
        eprintln!("startup: rocksdb stack map unavailable");
        return;
    };
    for line in maps.lines() {
        let Some((range, _)) = line.split_once(' ') else {
            continue;
        };
        let Some((start_hex, end_hex)) = range.split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) =
            (usize::from_str_radix(start_hex, 16), usize::from_str_radix(end_hex, 16))
        else {
            continue;
        };
        if (start..end).contains(&sp) {
            eprintln!("startup: rocksdb stack map {line} ({} bytes)", end.saturating_sub(start));
            return;
        }
    }
    eprintln!("startup: rocksdb stack map not found for {sp:#x}");
}

#[cfg(not(target_os = "linux"))]
fn log_open_thread_stack() {}

fn spawn_rocksdb_open(
    stack_size: usize,
    db_path: String,
    settings: RocksDbSettings,
) -> std::io::Result<
    std::thread::JoinHandle<Result<(Arc<DB>, Vec<String>, Cache, WriteBufferManager)>>,
> {
    std::thread::Builder::new()
        .name("kalamdb-rocksdb-open".to_string())
        .stack_size(stack_size)
        .spawn(move || {
            eprintln!("startup: rocksdb open thread entered (stack request {stack_size} bytes)");
            log_open_thread_stack();
            RocksDbInit::new(db_path, settings).open_with_cf_names_and_cache_here()
        })
}

/// RocksDB initializer for creating/opening a database with fixed physical CFs.
pub struct RocksDbInit {
    db_path: String,
    settings: RocksDbSettings,
}

impl RocksDbInit {
    /// Create a new initializer for the given path with custom settings.
    pub fn new(db_path: impl Into<String>, settings: RocksDbSettings) -> Self {
        Self {
            db_path: db_path.into(),
            settings: resolve_rocksdb_settings(settings),
        }
    }

    pub(crate) fn settings(&self) -> &RocksDbSettings {
        &self.settings
    }

    /// Create a new initializer with default settings.
    pub fn with_defaults(db_path: impl Into<String>) -> Self {
        Self::new(db_path, RocksDbSettings::default())
    }

    /// Open or create the RocksDB database and ensure fixed physical CFs exist.
    pub fn open(&self) -> Result<Arc<DB>> {
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        apply_db_settings(&mut db_opts, &self.settings);

        let cache = new_block_cache(self.settings.block_cache_size);
        let block_opts = create_block_options_with_cache(&cache);
        db_opts.set_block_based_table_factory(&block_opts);

        let (db, _, _) = self.open_internal(&mut db_opts, &cache)?;
        Ok(db)
    }

    /// Open and return both the DB and the list of column family names.
    pub fn open_with_cf_names(&self) -> Result<(Arc<DB>, Vec<String>)> {
        let (db, cf_names, _, _) = self.open_with_cf_names_and_cache()?;
        Ok((db, cf_names))
    }

    pub(crate) fn open_with_cf_names_and_cache(
        &self,
    ) -> Result<(Arc<DB>, Vec<String>, Cache, WriteBufferManager)> {
        // DB::Open on x86_64 uses more stack than the Linux main thread or the
        // 32MB process thread. The 128MB thread still hit its guard page on
        // GitHub's glibc, so the open gets a 512MB stack and records the
        // mapping actually installed.
        let db_path = self.db_path.clone();
        let settings = self.settings.clone();
        let handle = spawn_rocksdb_open(512 * 1024 * 1024, db_path.clone(), settings.clone())
            .or_else(|error| {
                eprintln!("startup: rocksdb 512MB open thread failed: {error}");
                spawn_rocksdb_open(128 * 1024 * 1024, db_path, settings)
            })
            .map_err(|error| anyhow::anyhow!("failed to start the rocksdb open thread: {error}"))?;
        match handle.join() {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn open_with_cf_names_and_cache_here(
        &self,
    ) -> Result<(Arc<DB>, Vec<String>, Cache, WriteBufferManager)> {
        eprintln!("startup: rocksdb options");
        let mut db_opts = Options::default();
        db_opts.create_if_missing(true);
        db_opts.create_missing_column_families(true);
        apply_db_settings(&mut db_opts, &self.settings);

        eprintln!("startup: rocksdb cache");
        let cache = new_block_cache(self.settings.block_cache_size);
        let block_opts = create_block_options_with_cache(&cache);
        db_opts.set_block_based_table_factory(&block_opts);

        let (db, cf_names, write_buffer_manager) = self.open_internal(&mut db_opts, &cache)?;
        eprintln!("startup: rocksdb open returned {} column families", cf_names.len());
        Ok((db, cf_names, cache, write_buffer_manager))
    }

    fn open_internal(
        &self,
        db_opts: &mut Options,
        cache: &Cache,
    ) -> Result<(Arc<DB>, Vec<String>, WriteBufferManager)> {
        let path = Path::new(&self.db_path);
        eprintln!("startup: rocksdb restore check");
        apply_pending_rocksdb_restore(path)?;
        eprintln!("startup: rocksdb listing column families");
        let cf_names = self.collect_column_family_names(db_opts, path);
        eprintln!("startup: rocksdb opening {} column families", cf_names.len());
        let write_buffer_manager =
            install_write_buffer_manager(db_opts, &self.settings, cf_names.len());

        eprintln!("startup: rocksdb building column family options");
        let cf_descriptors: Vec<_> = cf_names
            .iter()
            .map(|name| {
                let mut cf_opts = Options::default();
                apply_cf_settings(&mut cf_opts, &self.settings, name);
                cf_opts.set_block_based_table_factory(&create_block_options_with_cache(&cache));
                ColumnFamilyDescriptor::new(name, cf_opts)
            })
            .collect();

        eprintln!("startup: rocksdb calling DB::open");
        let db = DB::open_cf_descriptors(&db_opts, path, cf_descriptors)?;
        eprintln!("startup: rocksdb DB::open returned");
        let db = Arc::new(db);

        if self.settings.compact_on_startup {
            log::debug!("Running startup compaction for {} column families...", cf_names.len());
            let start = std::time::Instant::now();
            for cf_name in &cf_names {
                if let Some(cf) = db.cf_handle(cf_name) {
                    let sst_bytes = db
                        .property_int_value_cf(&cf, "rocksdb.total-sst-files-size")
                        .ok()
                        .flatten()
                        .unwrap_or(1);
                    let mem_entries = db
                        .property_int_value_cf(&cf, "rocksdb.num-entries-active-mem-table")
                        .ok()
                        .flatten()
                        .unwrap_or(1);
                    if sst_bytes > 0 || mem_entries > 0 {
                        db.compact_range_cf(&cf, None::<&[u8]>, None::<&[u8]>);
                    }
                }
            }
            log::info!("Startup compaction completed in {:?}", start.elapsed());
        }

        Ok((db, cf_names, write_buffer_manager))
    }

    fn collect_column_family_names(&self, db_opts: &Options, path: &Path) -> Vec<String> {
        let mut existing = match DB::list_cf(db_opts, path) {
            Ok(cfs) if !cfs.is_empty() => cfs,
            _ => vec!["default".to_string()],
        };

        for name in fixed_column_families() {
            if !existing.iter().any(|existing_name| existing_name == name) {
                existing.push(name.to_string());
            }
        }

        existing
    }
}

/// Shared block cache for all column families.
///
/// HyperClockCache is the RocksDB 11.8 recommended implementation for concurrent
/// point lookups. `estimated_entry_charge = 0` selects the dynamically sized variant.
pub(crate) fn new_block_cache(capacity: usize) -> Cache {
    Cache::new_hyper_clock_cache(capacity, 0)
}

pub(crate) fn create_block_options_with_cache(cache: &Cache) -> BlockBasedOptions {
    let mut block_opts = BlockBasedOptions::default();
    block_opts.set_block_cache(cache);
    block_opts.set_hybrid_ribbon_filter(10.0, 2);
    block_opts.set_optimize_filters_for_memory(true);
    block_opts.set_format_version(7);
    block_opts.set_data_block_index_type(DataBlockIndexType::BinaryAndHash);
    block_opts.set_data_block_hash_ratio(0.75);
    block_opts.set_cache_index_and_filter_blocks(true);
    block_opts.set_cache_index_and_filter_blocks_with_high_priority(true);
    block_opts.set_pin_l0_filter_and_index_blocks_in_cache(false);
    block_opts.set_pin_top_level_index_and_filter(true);
    block_opts.set_whole_key_filtering(true);
    block_opts
}

fn resolve_rocksdb_settings(settings: RocksDbSettings) -> RocksDbSettings {
    if !settings.memory_mode.scales_from_host_ram() {
        return settings;
    }
    let total_memory_bytes = host_total_memory_bytes();
    let resolved = settings.resolved_for_host_memory(total_memory_bytes);
    log::info!(
        "rocksdb memory_mode={} host_ram_bytes={:?} block_cache={} hot_data_write_buffer={}",
        resolved.memory_mode,
        total_memory_bytes,
        resolved.block_cache_size,
        resolved.cf_profiles.hot_data.write_buffer_size
    );
    resolved
}

fn host_total_memory_bytes() -> Option<u64> {
    let mut system = System::new();
    system.refresh_memory_specifics(MemoryRefreshKind::everything());
    let total = system.total_memory();
    (total > 0).then_some(total)
}
