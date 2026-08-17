use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};

const RATE_WINDOW_SECS: u64 = 60;
const RATE_BUCKET_COUNT: usize = RATE_WINDOW_SECS as usize;

static MANIFEST_CACHE_ROCKSDB_ENTRIES: AtomicU64 = AtomicU64::new(0);
static MANIFEST_CACHE_MEMORY_ENTRIES: AtomicU64 = AtomicU64::new(0);
static MANIFEST_READS_TOTAL: AtomicU64 = AtomicU64::new(0);
static MANIFEST_WRITES_TOTAL: AtomicU64 = AtomicU64::new(0);
static FLUSH_OPERATIONS_TOTAL: AtomicU64 = AtomicU64::new(0);
static PARQUET_FILES_WRITTEN_TOTAL: AtomicU64 = AtomicU64::new(0);
static PARQUET_FILES_READ_TOTAL: AtomicU64 = AtomicU64::new(0);
static PARQUET_ROWS_FLUSHED_TOTAL: AtomicU64 = AtomicU64::new(0);
static FIRST_STORAGE_EPOCH_SECOND: AtomicU64 = AtomicU64::new(0);

struct RateBuckets {
    epochs: [AtomicU64; RATE_BUCKET_COUNT],
    counts: [AtomicU64; RATE_BUCKET_COUNT],
}

impl RateBuckets {
    fn new() -> Self {
        Self {
            epochs: std::array::from_fn(|_| AtomicU64::new(0)),
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

static MANIFEST_READ_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static MANIFEST_WRITE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static PARQUET_WRITE_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();
static PARQUET_READ_RATE_BUCKETS: OnceLock<RateBuckets> = OnceLock::new();

fn manifest_read_rate_buckets() -> &'static RateBuckets {
    MANIFEST_READ_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn manifest_write_rate_buckets() -> &'static RateBuckets {
    MANIFEST_WRITE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn parquet_write_rate_buckets() -> &'static RateBuckets {
    PARQUET_WRITE_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn parquet_read_rate_buckets() -> &'static RateBuckets {
    PARQUET_READ_RATE_BUCKETS.get_or_init(RateBuckets::new)
}

fn epoch_second() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn mark_first_storage_activity(now_second: u64) {
    let _ = FIRST_STORAGE_EPOCH_SECOND.compare_exchange(
        0,
        now_second,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn observe_rate_event(buckets: &RateBuckets, now_second: u64) {
    let index = (now_second % RATE_WINDOW_SECS) as usize;
    let bucket_epoch = &buckets.epochs[index];
    let bucket_count = &buckets.counts[index];
    let observed_epoch = bucket_epoch.load(Ordering::Acquire);
    if observed_epoch != now_second
        && bucket_epoch
            .compare_exchange(observed_epoch, now_second, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        bucket_count.store(0, Ordering::Release);
    }
    bucket_count.fetch_add(1, Ordering::Relaxed);
}

fn snapshot_rate(buckets: &RateBuckets, now_second: u64, elapsed_window: u64) -> f64 {
    let mut recent_count = 0u64;
    for index in 0..RATE_BUCKET_COUNT {
        let bucket_epoch = buckets.epochs[index].load(Ordering::Acquire);
        if bucket_epoch <= now_second && now_second.saturating_sub(bucket_epoch) < RATE_WINDOW_SECS
        {
            recent_count =
                recent_count.saturating_add(buckets.counts[index].load(Ordering::Relaxed));
        }
    }

    recent_count as f64 / elapsed_window as f64
}

fn storage_elapsed_window(now_second: u64) -> u64 {
    let first_second = FIRST_STORAGE_EPOCH_SECOND.load(Ordering::Acquire);
    if first_second == 0 {
        1
    } else {
        now_second
            .saturating_sub(first_second)
            .saturating_add(1)
            .min(RATE_WINDOW_SECS)
            .max(1)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StorageMetricsSnapshot {
    pub manifest_cache_rocksdb_entries: u64,
    pub manifest_cache_memory_entries: u64,
    pub manifest_reads_total: u64,
    pub manifest_reads_per_second: f64,
    pub manifest_writes_total: u64,
    pub manifest_writes_per_second: f64,
    pub flush_operations_total: u64,
    pub parquet_files_written_total: u64,
    pub parquet_files_written_per_second: f64,
    pub parquet_files_read_total: u64,
    pub parquet_files_read_per_second: f64,
    pub parquet_rows_flushed_total: u64,
}

impl StorageMetricsSnapshot {
    pub fn as_pairs(&self) -> Vec<(String, String)> {
        vec![
            (
                "manifest_cache_rocksdb_entries".to_string(),
                self.manifest_cache_rocksdb_entries.to_string(),
            ),
            (
                "manifest_cache_memory_entries".to_string(),
                self.manifest_cache_memory_entries.to_string(),
            ),
            ("manifest_reads_total".to_string(), self.manifest_reads_total.to_string()),
            (
                "manifest_reads_per_second".to_string(),
                format!("{:.2}", self.manifest_reads_per_second),
            ),
            ("manifest_writes_total".to_string(), self.manifest_writes_total.to_string()),
            (
                "manifest_writes_per_second".to_string(),
                format!("{:.2}", self.manifest_writes_per_second),
            ),
            ("flush_operations_total".to_string(), self.flush_operations_total.to_string()),
            (
                "parquet_files_written_total".to_string(),
                self.parquet_files_written_total.to_string(),
            ),
            (
                "parquet_files_written_per_second".to_string(),
                format!("{:.2}", self.parquet_files_written_per_second),
            ),
            (
                "parquet_files_read_total".to_string(),
                self.parquet_files_read_total.to_string(),
            ),
            (
                "parquet_files_read_per_second".to_string(),
                format!("{:.2}", self.parquet_files_read_per_second),
            ),
            (
                "parquet_rows_flushed_total".to_string(),
                self.parquet_rows_flushed_total.to_string(),
            ),
        ]
    }
}

pub fn initialize_manifest_cache_rocksdb_entries(entries: usize) {
    MANIFEST_CACHE_ROCKSDB_ENTRIES.store(entries as u64, Ordering::Release);
}

pub fn increment_manifest_cache_rocksdb_entries(delta: usize) {
    if delta > 0 {
        MANIFEST_CACHE_ROCKSDB_ENTRIES.fetch_add(delta as u64, Ordering::Relaxed);
    }
}

pub fn decrement_manifest_cache_rocksdb_entries(delta: usize) {
    if delta == 0 {
        return;
    }

    let decrement = delta as u64;
    let _ = MANIFEST_CACHE_ROCKSDB_ENTRIES.try_update(
        Ordering::AcqRel,
        Ordering::Acquire,
        |current| Some(current.saturating_sub(decrement)),
    );
}

pub fn set_manifest_cache_memory_entries(entries: usize) {
    MANIFEST_CACHE_MEMORY_ENTRIES.store(entries as u64, Ordering::Release);
}

pub fn record_manifest_read() {
    let now_second = epoch_second();
    mark_first_storage_activity(now_second);
    MANIFEST_READS_TOTAL.fetch_add(1, Ordering::Relaxed);
    observe_rate_event(manifest_read_rate_buckets(), now_second);
}

pub fn record_manifest_write() {
    let now_second = epoch_second();
    mark_first_storage_activity(now_second);
    MANIFEST_WRITES_TOTAL.fetch_add(1, Ordering::Relaxed);
    observe_rate_event(manifest_write_rate_buckets(), now_second);
}

pub fn record_parquet_files_written(file_count: usize, rows_flushed: usize) {
    if file_count == 0 && rows_flushed == 0 {
        return;
    }

    let now_second = epoch_second();
    mark_first_storage_activity(now_second);

    FLUSH_OPERATIONS_TOTAL.fetch_add(1, Ordering::Relaxed);
    if file_count > 0 {
        PARQUET_FILES_WRITTEN_TOTAL.fetch_add(file_count as u64, Ordering::Relaxed);
        for _ in 0..file_count {
            observe_rate_event(parquet_write_rate_buckets(), now_second);
        }
    }
    if rows_flushed > 0 {
        PARQUET_ROWS_FLUSHED_TOTAL.fetch_add(rows_flushed as u64, Ordering::Relaxed);
    }
}

pub fn record_parquet_file_read() {
    let now_second = epoch_second();
    mark_first_storage_activity(now_second);
    PARQUET_FILES_READ_TOTAL.fetch_add(1, Ordering::Relaxed);
    observe_rate_event(parquet_read_rate_buckets(), now_second);
}

pub fn storage_metrics_snapshot() -> StorageMetricsSnapshot {
    let now_second = epoch_second();
    let elapsed_window = storage_elapsed_window(now_second);

    StorageMetricsSnapshot {
        manifest_cache_rocksdb_entries: MANIFEST_CACHE_ROCKSDB_ENTRIES.load(Ordering::Relaxed),
        manifest_cache_memory_entries: MANIFEST_CACHE_MEMORY_ENTRIES.load(Ordering::Relaxed),
        manifest_reads_total: MANIFEST_READS_TOTAL.load(Ordering::Relaxed),
        manifest_reads_per_second: snapshot_rate(
            manifest_read_rate_buckets(),
            now_second,
            elapsed_window,
        ),
        manifest_writes_total: MANIFEST_WRITES_TOTAL.load(Ordering::Relaxed),
        manifest_writes_per_second: snapshot_rate(
            manifest_write_rate_buckets(),
            now_second,
            elapsed_window,
        ),
        flush_operations_total: FLUSH_OPERATIONS_TOTAL.load(Ordering::Relaxed),
        parquet_files_written_total: PARQUET_FILES_WRITTEN_TOTAL.load(Ordering::Relaxed),
        parquet_files_written_per_second: snapshot_rate(
            parquet_write_rate_buckets(),
            now_second,
            elapsed_window,
        ),
        parquet_files_read_total: PARQUET_FILES_READ_TOTAL.load(Ordering::Relaxed),
        parquet_files_read_per_second: snapshot_rate(
            parquet_read_rate_buckets(),
            now_second,
            elapsed_window,
        ),
        parquet_rows_flushed_total: PARQUET_ROWS_FLUSHED_TOTAL.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
fn reset_storage_metrics_for_tests() {
    MANIFEST_CACHE_ROCKSDB_ENTRIES.store(0, Ordering::Relaxed);
    MANIFEST_CACHE_MEMORY_ENTRIES.store(0, Ordering::Relaxed);
    MANIFEST_READS_TOTAL.store(0, Ordering::Relaxed);
    MANIFEST_WRITES_TOTAL.store(0, Ordering::Relaxed);
    FLUSH_OPERATIONS_TOTAL.store(0, Ordering::Relaxed);
    PARQUET_FILES_WRITTEN_TOTAL.store(0, Ordering::Relaxed);
    PARQUET_FILES_READ_TOTAL.store(0, Ordering::Relaxed);
    PARQUET_ROWS_FLUSHED_TOTAL.store(0, Ordering::Relaxed);
    FIRST_STORAGE_EPOCH_SECOND.store(0, Ordering::Relaxed);

    for buckets in [
        manifest_read_rate_buckets(),
        manifest_write_rate_buckets(),
        parquet_write_rate_buckets(),
        parquet_read_rate_buckets(),
    ] {
        for index in 0..RATE_BUCKET_COUNT {
            buckets.epochs[index].store(0, Ordering::Relaxed);
            buckets.counts[index].store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_metrics_snapshot_tracks_manifest_and_flush_counters() {
        reset_storage_metrics_for_tests();

        initialize_manifest_cache_rocksdb_entries(7);
        increment_manifest_cache_rocksdb_entries(2);
        decrement_manifest_cache_rocksdb_entries(3);
        set_manifest_cache_memory_entries(4);
        record_manifest_read();
        record_manifest_write();
        record_parquet_files_written(5, 42);
        record_parquet_file_read();

        let snapshot = storage_metrics_snapshot();
        assert_eq!(snapshot.manifest_cache_rocksdb_entries, 6);
        assert_eq!(snapshot.manifest_cache_memory_entries, 4);
        assert_eq!(snapshot.manifest_reads_total, 1);
        assert_eq!(snapshot.manifest_writes_total, 1);
        assert_eq!(snapshot.flush_operations_total, 1);
        assert_eq!(snapshot.parquet_files_written_total, 5);
        assert_eq!(snapshot.parquet_files_read_total, 1);
        assert_eq!(snapshot.parquet_rows_flushed_total, 42);
        assert!(snapshot.manifest_reads_per_second > 0.0);
        assert!(snapshot.manifest_writes_per_second > 0.0);
        assert!(snapshot.parquet_files_written_per_second > 0.0);
        assert!(snapshot.parquet_files_read_per_second > 0.0);
    }
}
