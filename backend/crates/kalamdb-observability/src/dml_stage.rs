//! Aggregate DML stage timings.
//!
//! Disabled unless `KALAMDB_DML_STAGE_TRACE=1`. A disabled guard is one relaxed
//! atomic load. Nested stages stay nested: callers must not subtract one stage
//! from another. There are no per-row logs and no primary-key labels.

use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

const STAGE_COUNT: usize = 8;

static ENABLED: AtomicBool = AtomicBool::new(false);
static SCAN_FILTER_DEPTH: AtomicU64 = AtomicU64::new(0);
static NANOS: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];
static HITS: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];
static UNITS: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];

const COUNT_LEN: usize = 8;
static COUNTS: [AtomicU64; COUNT_LEN] = [const { AtomicU64::new(0) }; COUNT_LEN];

/// Storage counters. A cache's own bookkeeping is not one of these.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DmlCount {
    BackendLookup = 0,
    DecodedRows = 1,
    DecodedBytes = 2,
    IndexEntries = 3,
    SubmittedBytes = 4,
    TargetMiss = 5,
    FetchedBytes = 6,
    /// One full authoritative baseline resolution. Cache bookkeeping is not this.
    BaselineDecode = 7,
}

impl DmlCount {
    const fn name(self) -> &'static str {
        match self {
            Self::BackendLookup => "backend_lookups",
            Self::DecodedRows => "decoded_rows",
            Self::DecodedBytes => "decoded_bytes",
            Self::IndexEntries => "index_entries",
            Self::SubmittedBytes => "submitted_bytes",
            Self::TargetMiss => "target_misses",
            Self::FetchedBytes => "fetched_bytes",
            Self::BaselineDecode => "baseline_decodes",
        }
    }

    fn index(self) -> usize {
        self as u8 as usize
    }
}

/// Coarse stages for a profiling run. Timing suites leave this disabled.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DmlStage {
    Planning = 0,
    ScanFilter = 1,
    Overlay = 2,
    StagingFold = 3,
    TargetResolution = 4,
    RowEncode = 5,
    BackendSubmit = 6,
    PostCommit = 7,
}

impl DmlStage {
    const fn name(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::ScanFilter => "scan_filter",
            Self::Overlay => "overlay",
            Self::StagingFold => "staging_fold",
            Self::TargetResolution => "target_resolution",
            Self::RowEncode => "row_encode",
            Self::BackendSubmit => "backend_submit",
            Self::PostCommit => "post_commit",
        }
    }

    fn index(self) -> usize {
        self as u8 as usize
    }
}

/// Turn DML stage timing on when `KALAMDB_DML_STAGE_TRACE` is `1`, `true`, or `on`.
pub fn enable_from_env() {
    let enabled = std::env::var("KALAMDB_DML_STAGE_TRACE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false);
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        eprintln!("dml stage trace enabled");
    }
}

#[inline]
fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Whether DML stage tracing is on. Hot iterators should read this once.
#[inline]
pub fn tracing_enabled() -> bool {
    enabled()
}

/// Count one authoritative baseline resolution.
///
/// This is a hot or cold row decode used to prepare a mutation, including a
/// miss. A deleted-flag check that does not load the payload is not a decode.
/// Statement liveness rechecks after a primary-key claim wait are conflict
/// retries and are outside the preparation budget.
#[inline]
pub fn record_baseline_decode() {
    add_count(DmlCount::BaselineDecode, 1);
}

/// Adds `amount` to a storage counter when tracing is enabled.
#[inline]
pub fn add_count(kind: DmlCount, amount: u64) {
    if amount == 0 || !enabled() {
        return;
    }
    COUNTS[kind.index()].fetch_add(amount, Ordering::Relaxed);
}

/// Times the enclosing scope when DML stage tracing is enabled.
pub struct DmlStageGuard {
    stage: DmlStage,
    start: Option<Instant>,
}

#[inline]
pub fn stage_guard(stage: DmlStage) -> DmlStageGuard {
    let start = if enabled() {
        if stage == DmlStage::ScanFilter {
            SCAN_FILTER_DEPTH.fetch_add(1, Ordering::Relaxed);
        }
        Some(Instant::now())
    } else {
        None
    };
    DmlStageGuard { stage, start }
}

/// Adds one completed span. A storage scan does not add scan time while a
/// [`DmlStage::ScanFilter`] guard is already active, so the two spans are not summed.
pub fn record_elapsed(stage: DmlStage, elapsed: std::time::Duration) {
    if !enabled() {
        return;
    }
    if stage == DmlStage::ScanFilter && SCAN_FILTER_DEPTH.load(Ordering::Relaxed) > 0 {
        return;
    }
    let index = stage.index();
    NANOS[index].fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    HITS[index].fetch_add(1, Ordering::Relaxed);
}
#[inline]
pub fn add_units(stage: DmlStage, units: u64) {
    if units == 0 || !enabled() {
        return;
    }
    UNITS[stage.index()].fetch_add(units, Ordering::Relaxed);
}

impl Drop for DmlStageGuard {
    fn drop(&mut self) {
        let Some(start) = self.start else {
            return;
        };
        if self.stage == DmlStage::ScanFilter {
            SCAN_FILTER_DEPTH.fetch_sub(1, Ordering::Relaxed);
        }
        let index = self.stage.index();
        NANOS[index].fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        HITS[index].fetch_add(1, Ordering::Relaxed);
    }
}

/// One line of aggregate stage totals. Empty when every stage is still zero.
pub fn snapshot_line() -> Option<String> {
    let mut line = String::from("dml-stage");
    let mut any = false;
    for index in 0..STAGE_COUNT {
        let hits = HITS[index].load(Ordering::Relaxed);
        if hits == 0 {
            continue;
        }
        any = true;
        let nanos = NANOS[index].load(Ordering::Relaxed);
        let units = UNITS[index].load(Ordering::Relaxed);
        let name = match index {
            0 => DmlStage::Planning.name(),
            1 => DmlStage::ScanFilter.name(),
            2 => DmlStage::Overlay.name(),
            3 => DmlStage::StagingFold.name(),
            4 => DmlStage::TargetResolution.name(),
            5 => DmlStage::RowEncode.name(),
            6 => DmlStage::BackendSubmit.name(),
            _ => DmlStage::PostCommit.name(),
        };
        let millis = nanos / 1_000_000;
        line.push_str(&format!(" {name}={millis}ms/n={hits}/units={units}"));
    }
    for index in 0..COUNT_LEN {
        let value = COUNTS[index].load(Ordering::Relaxed);
        if value == 0 {
            continue;
        }
        any = true;
        let name = match index {
            0 => DmlCount::BackendLookup.name(),
            1 => DmlCount::DecodedRows.name(),
            2 => DmlCount::DecodedBytes.name(),
            3 => DmlCount::IndexEntries.name(),
            4 => DmlCount::SubmittedBytes.name(),
            5 => DmlCount::TargetMiss.name(),
            6 => DmlCount::FetchedBytes.name(),
            _ => DmlCount::BaselineDecode.name(),
        };
        line.push_str(&format!(" {name}={value}"));
    }
    any.then_some(line)
}

/// Prints the current aggregate line and clears it so the next query starts at zero.
pub fn eprint_and_reset(label: &str) {
    if !enabled() {
        return;
    }
    match snapshot_line() {
        Some(line) => eprintln!("{line} sql={label}"),
        None => eprintln!("dml-stage sql={label}"),
    }
    for slot in &NANOS {
        slot.store(0, Ordering::Relaxed);
    }
    for slot in &HITS {
        slot.store(0, Ordering::Relaxed);
    }
    for slot in &UNITS {
        slot.store(0, Ordering::Relaxed);
    }
    for slot in &COUNTS {
        slot.store(0, Ordering::Relaxed);
    }
}

/// Prints [`snapshot_line`] when tracing recorded anything.
pub fn eprint_snapshot() {
    if let Some(line) = snapshot_line() {
        eprintln!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_records_only_while_enabled() {
        ENABLED.store(false, Ordering::Relaxed);
        {
            let _guard = stage_guard(DmlStage::Planning);
            add_units(DmlStage::Planning, 4);
        }
        assert!(snapshot_line().is_none());

        ENABLED.store(true, Ordering::Relaxed);
        NANOS[DmlStage::BackendSubmit.index()].store(0, Ordering::Relaxed);
        HITS[DmlStage::BackendSubmit.index()].store(0, Ordering::Relaxed);
        UNITS[DmlStage::BackendSubmit.index()].store(0, Ordering::Relaxed);
        {
            let _guard = stage_guard(DmlStage::BackendSubmit);
            add_units(DmlStage::BackendSubmit, 3);
        }
        let line = snapshot_line().expect("snapshot");
        assert!(line.contains("backend_submit="));
        assert!(line.contains("n=1"));
        assert!(line.contains("units=3"));
        add_count(DmlCount::TargetMiss, 2);
        let line = snapshot_line().expect("snapshot");
        assert!(line.contains("target_misses=2"));
        ENABLED.store(false, Ordering::Relaxed);
        add_count(DmlCount::TargetMiss, 9);
        HITS[DmlStage::BackendSubmit.index()].store(0, Ordering::Relaxed);
        COUNTS[DmlCount::TargetMiss.index()].store(0, Ordering::Relaxed);
    }
}
