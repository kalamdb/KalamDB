use kalamdb_commons::system_tables::{classify_column_family_name, ColumnFamilyProfile};
use kalamdb_configs::{RocksDbCfProfileSettings, RocksDbMemoryMode, RocksDbSettings};
use rocksdb::{CompactionPri, DBCompressionType, Options, SliceTransform};

use super::keyspace::{physical_key_prefix_in_domain, physical_key_prefix_transform};

const BYTES_PER_SYNC: u64 = 1024 * 1024;
const MEMTABLE_PREFIX_BLOOM_RATIO: f64 = 0.1;
const HOT_DATA_MEMPURGE_THRESHOLD: f64 = 1.0;

pub(crate) fn apply_db_settings(db_opts: &mut Options, settings: &RocksDbSettings) {
    let default_cf = &settings.cf_profiles.system_meta;
    db_opts.set_write_buffer_size(default_cf.write_buffer_size);
    db_opts.set_max_write_buffer_number(default_cf.max_write_buffers);
    db_opts.set_max_background_jobs(settings.max_background_jobs);
    db_opts.increase_parallelism(settings.max_background_jobs);
    db_opts.set_max_open_files(settings.max_open_files);
    match settings.memory_mode {
        RocksDbMemoryMode::Compact => {
            db_opts.set_max_subcompactions(1);
            db_opts.set_max_file_opening_threads(2);
        },
        RocksDbMemoryMode::Auto => {
            db_opts.set_max_subcompactions(2);
            db_opts.set_max_file_opening_threads(8);
        },
    }
    db_opts.set_enable_pipelined_write(true);
    db_opts.set_avoid_unnecessary_blocking_io(true);
    db_opts.set_bytes_per_sync(BYTES_PER_SYNC);
    db_opts.set_wal_bytes_per_sync(BYTES_PER_SYNC);
}

pub(crate) fn apply_cf_settings(cf_opts: &mut Options, settings: &RocksDbSettings, cf_name: &str) {
    let profile = classify_column_family_name(cf_name);
    let profile_settings = profile_settings(settings, profile);
    cf_opts.set_write_buffer_size(profile_settings.write_buffer_size);
    cf_opts.set_max_write_buffer_number(profile_settings.max_write_buffers);
    cf_opts.set_compression_type(DBCompressionType::Lz4);
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
