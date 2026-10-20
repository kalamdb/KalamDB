use kalamdb_commons::system_tables::{classify_column_family_name, ColumnFamilyProfile};
use kalamdb_configs::{RocksDbCfProfileSettings, RocksDbSettings};
use rocksdb::{CompactionPri, DBCompressionType, Options, SliceTransform, WriteBufferManager};

use super::keyspace::{physical_key_prefix_in_domain, physical_key_prefix_transform};

const BYTES_PER_SYNC: u64 = 1024 * 1024;
const MEMTABLE_PREFIX_BLOOM_RATIO: f64 = 0.1;
const HOT_DATA_MEMPURGE_THRESHOLD: f64 = 1.0;
/// RocksDB `Arena::kMinBlockSize`. Automatic arena blocks are `write_buffer_size / 8`
/// and never smaller than this.
const MIN_ARENA_BLOCK: usize = 4 * 1024;
/// Active memtables flush when their total bytes exceed 7/8 of the write-buffer
/// budget. Two arena blocks of headroom keeps a freshly opened family under that line.
const MEMTABLE_BUDGET_HEADROOM: usize = 2;
const MIN_TARGET_FILE_SIZE: u64 = 256 * 1024;
const MAX_TARGET_FILE_SIZE: u64 = 64 * 1024 * 1024;
const MAX_LEVEL_BYTES: u64 = 256 * 1024 * 1024;
const MIN_TOTAL_WAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_WAL_BYTES: u64 = 512 * 1024 * 1024;
const INFO_LOG_MAX_BYTES: usize = 8 * 1024 * 1024;
const INFO_LOG_KEEP_FILES: usize = 4;

pub(crate) fn apply_db_settings(db_opts: &mut Options, settings: &RocksDbSettings) {
    let default_cf = &settings.cf_profiles.system_meta;
    db_opts.set_write_buffer_size(default_cf.write_buffer_size);
    db_opts.set_max_write_buffer_number(default_cf.max_write_buffers);
    db_opts.set_max_total_wal_size(max_total_wal_size(settings));
    db_opts.set_max_background_jobs(settings.max_background_jobs);
    db_opts.increase_parallelism(settings.max_background_jobs);
    db_opts.set_max_open_files(settings.max_open_files);
    let (max_subcompactions, max_file_opening_threads) =
        settings.memory_mode.rocksdb_io_parallelism();
    db_opts.set_max_subcompactions(max_subcompactions);
    db_opts.set_max_file_opening_threads(max_file_opening_threads);
    db_opts.set_enable_pipelined_write(true);
    db_opts.set_avoid_unnecessary_blocking_io(true);
    db_opts.set_bytes_per_sync(BYTES_PER_SYNC);
    db_opts.set_wal_bytes_per_sync(BYTES_PER_SYNC);
    // A flush storm writes tens of kilobytes of info log per second. Roll it.
    db_opts.set_max_log_file_size(INFO_LOG_MAX_BYTES);
    db_opts.set_keep_log_file_num(INFO_LOG_KEEP_FILES);
}

/// Install the shared memtable budget.
///
/// The profile sum alone is about 1MB. Each open column family still charges one
/// arena block (8–32KB). With a couple thousand families that minimum exceeds the
/// 1MB cap, `WriteBufferManager::ShouldFlush` stays true, and every background
/// write flushes some other family into a new tiny SST. Those files then sit in
/// L6 because the default 256MB level target never fills.
///
/// The returned manager is what RocksDB consults. Call [`WriteBufferManager::set_buffer_size`]
/// with [`db_write_buffer_budget`] when families are created or dropped.
pub(crate) fn install_write_buffer_manager(
    db_opts: &mut Options,
    settings: &RocksDbSettings,
    column_family_count: usize,
) -> WriteBufferManager {
    let budget = db_write_buffer_budget(settings, column_family_count);
    let manager = WriteBufferManager::new_write_buffer_manager(budget, false);
    db_opts.set_db_write_buffer_size(budget);
    db_opts.set_write_buffer_manager(&manager);
    manager
}

/// Shared memtable cap for `column_family_count` open families.
pub(crate) fn db_write_buffer_budget(
    settings: &RocksDbSettings,
    column_family_count: usize,
) -> usize {
    let families = column_family_count.max(1);
    let floor = families
        .saturating_mul(largest_arena_block(settings))
        .saturating_mul(MEMTABLE_BUDGET_HEADROOM);
    shared_profile_budget(settings).max(floor)
}

fn shared_profile_budget(settings: &RocksDbSettings) -> usize {
    let profiles = &settings.cf_profiles;
    [
        &profiles.system_meta,
        &profiles.system_index,
        &profiles.hot_data,
        &profiles.hot_index,
        &profiles.raft,
    ]
    .into_iter()
    .fold(0usize, |sum, profile| {
        sum.saturating_add(
            profile
                .write_buffer_size
                .saturating_mul(profile.max_write_buffers.max(1) as usize),
        )
    })
}

fn largest_arena_block(settings: &RocksDbSettings) -> usize {
    let profiles = &settings.cf_profiles;
    [
        &profiles.system_meta,
        &profiles.system_index,
        &profiles.hot_data,
        &profiles.hot_index,
        &profiles.raft,
    ]
    .into_iter()
    .fold(MIN_ARENA_BLOCK, |largest, profile| {
        largest.max(arena_block_size(profile.write_buffer_size))
    })
}

fn arena_block_size(write_buffer_size: usize) -> usize {
    (write_buffer_size / 8).max(MIN_ARENA_BLOCK)
}

pub(crate) fn apply_cf_settings(cf_opts: &mut Options, settings: &RocksDbSettings, cf_name: &str) {
    let profile = classify_column_family_name(cf_name);
    let profile_settings = profile_settings(settings, profile);
    cf_opts.set_write_buffer_size(profile_settings.write_buffer_size);
    cf_opts.set_max_write_buffer_number(profile_settings.max_write_buffers);
    let target_file_size = target_file_size_base(profile_settings.write_buffer_size);
    cf_opts.set_target_file_size_base(target_file_size);
    cf_opts.set_max_bytes_for_level_base(max_bytes_for_level_base(target_file_size));
    cf_opts.set_compression_type(DBCompressionType::Zstd);
    cf_opts.set_bottommost_compression_type(DBCompressionType::Zstd);
    cf_opts.set_level_compaction_dynamic_level_bytes(true);
    cf_opts.set_compaction_pri(CompactionPri::MinOverlappingRatio);
    cf_opts.set_prefix_extractor(SliceTransform::create(
        "kalam-partition-prefix",
        physical_key_prefix_transform,
        Some(physical_key_prefix_in_domain),
    ));
    cf_opts.set_memtable_prefix_bloom_ratio(MEMTABLE_PREFIX_BLOOM_RATIO);
    cf_opts.set_memtable_whole_key_filtering(true);
    if profile == ColumnFamilyProfile::HotData {
        cf_opts.set_experimental_mempurge_threshold(HOT_DATA_MEMPURGE_THRESHOLD);
    }
}

/// SST target scaled from the memtable so a handful of flushes merge.
///
/// The RocksDB default is 64MB files and a 256MB L1 base. A per-table family
/// that appends a few hundred bytes per flush trivial-moves each file into the
/// bottom level and never compacts it. One idle dev database grew past 6,000
/// SST files in a single family that way.
fn target_file_size_base(write_buffer_size: usize) -> u64 {
    (write_buffer_size as u64)
        .saturating_mul(8)
        .clamp(MIN_TARGET_FILE_SIZE, MAX_TARGET_FILE_SIZE)
}

fn max_bytes_for_level_base(target_file_size: u64) -> u64 {
    target_file_size.saturating_mul(10).clamp(target_file_size, MAX_LEVEL_BYTES)
}

/// Cap total WAL size independently of the number of open column families.
///
/// When left at `0`, RocksDB derives the WAL limit from the sum of every open
/// family's write buffers (`sum_cfs(write_buffer_size * max_write_buffers) * 4`),
/// which grows to gigabytes with thousands of families and prevents RocksDB from
/// automatically flushing the oldest WAL-pinning family.
pub(crate) fn max_total_wal_size(settings: &RocksDbSettings) -> u64 {
    (shared_profile_budget(settings) as u64)
        .saturating_mul(4)
        .clamp(MIN_TOTAL_WAL_BYTES, MAX_TOTAL_WAL_BYTES)
}

fn profile_settings(
    settings: &RocksDbSettings,
    profile: ColumnFamilyProfile,
) -> &RocksDbCfProfileSettings {
    match profile {
        ColumnFamilyProfile::SystemMeta => &settings.cf_profiles.system_meta,
        ColumnFamilyProfile::SystemIndex => &settings.cf_profiles.system_index,
        ColumnFamilyProfile::HotData => &settings.cf_profiles.hot_data,
        ColumnFamilyProfile::HotIndex => &settings.cf_profiles.hot_index,
        ColumnFamilyProfile::Raft => &settings.cf_profiles.raft,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_buffer_budget_covers_one_memtable_per_family() {
        let settings = RocksDbSettings::default();
        let families = 2_000;
        let budget = db_write_buffer_budget(&settings, families);
        let one_block_each = families * largest_arena_block(&settings);
        // RocksDB flushes once active memtables exceed 7/8 of the budget.
        assert!(
            one_block_each * 8 < budget * 7,
            "budget {budget} cannot hold {one_block_each} bytes of memtables"
        );
        assert!(budget > shared_profile_budget(&settings));
    }

    #[test]
    fn small_profiles_compact_before_thousands_of_sst_files() {
        let settings = RocksDbSettings::default();
        let write_buffer = settings.cf_profiles.hot_data.write_buffer_size;
        let target = target_file_size_base(write_buffer);
        let level_base = max_bytes_for_level_base(target);
        // A ~2KB flush must not be allowed to accumulate until a 256MB level fills.
        assert!(level_base <= 16 * 1024 * 1024, "level base {level_base}");
        assert!(target <= level_base);
    }
}
