//! Per-request stage timings for the SQL point-read path.
//!
//! Disabled unless `KALAMDB_SQL_STAGE_TRACE=1`. A disabled guard is one relaxed
//! atomic load and does not take a timestamp.

use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

const STAGE_COUNT: usize = 16;

static ENABLED: AtomicBool = AtomicBool::new(false);
static NANOS: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];
static HITS: [AtomicU64; STAGE_COUNT] = [const { AtomicU64::new(0) }; STAGE_COUNT];

/// Stages on the HTTP SQL point-read path, in source order.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SqlStage {
    Handler = 0,
    Parse = 1,
    Prepare = 2,
    Forward = 3,
    PointGet = 4,
    Bind = 5,
    Session = 6,
    ScanPlan = 7,
    Scalar = 8,
    Arrow = 9,
    Json = 10,
    Leader = 11,
    HotGet = 12,
    Materialize = 13,
    PreAuthorize = 14,
    JsonRows = 15,
}

impl SqlStage {
    const fn name(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Parse => "parse",
            Self::Prepare => "prepare",
            Self::Forward => "forward",
            Self::PointGet => "point_get",
            Self::Bind => "bind",
            Self::Session => "session",
            Self::ScanPlan => "scan_plan",
            Self::Scalar => "scalar",
            Self::Arrow => "arrow",
            Self::Json => "json",
            Self::Leader => "leader",
            Self::HotGet => "hot_get",
            Self::Materialize => "materialize",
            Self::PreAuthorize => "pre_auth",
            Self::JsonRows => "json_rows",
        }
    }

    fn index(self) -> usize {
        self as u8 as usize
    }
}

/// Turn stage timing on when `KALAMDB_SQL_STAGE_TRACE` is `1`, `true`, or `on`.
pub fn enable_from_env() {
    let enabled = std::env::var("KALAMDB_SQL_STAGE_TRACE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "on" | "yes"))
        .unwrap_or(false);
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        eprintln!("sql stage trace enabled");
    }
}

#[inline]
fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Times the enclosing scope when stage tracing is enabled.
pub struct StageGuard {
    stage: SqlStage,
    start: Option<Instant>,
}

#[inline]
pub fn stage_guard(stage: SqlStage) -> StageGuard {
    StageGuard {
        stage,
        start: enabled().then(Instant::now),
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        let Some(start) = self.start else {
            return;
        };
        let index = self.stage.index();
        NANOS[index].fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        let hits = HITS[index].fetch_add(1, Ordering::Relaxed) + 1;
        if self.stage == SqlStage::Handler && hits.is_multiple_of(100_000) {
            eprint_snapshot(hits);
        }
    }
}

fn eprint_snapshot(handler_hits: u64) {
    let mut line = format!("sql-stage handler_n={handler_hits}");
    for index in 0..STAGE_COUNT {
        let hits = HITS[index].load(Ordering::Relaxed);
        if hits == 0 {
            continue;
        }
        let nanos = NANOS[index].load(Ordering::Relaxed);
        let avg_us = nanos / hits / 1_000;
        let name = match index {
            0 => SqlStage::Handler.name(),
            1 => SqlStage::Parse.name(),
            2 => SqlStage::Prepare.name(),
            3 => SqlStage::Forward.name(),
            4 => SqlStage::PointGet.name(),
            5 => SqlStage::Bind.name(),
            6 => SqlStage::Session.name(),
            7 => SqlStage::ScanPlan.name(),
            8 => SqlStage::Scalar.name(),
            9 => SqlStage::Arrow.name(),
            10 => SqlStage::Json.name(),
            11 => SqlStage::Leader.name(),
            12 => SqlStage::HotGet.name(),
            13 => SqlStage::Materialize.name(),
            14 => SqlStage::PreAuthorize.name(),
            _ => SqlStage::JsonRows.name(),
        };
        line.push_str(&format!(" {name}={avg_us}us/n={hits}"));
    }
    eprintln!("{line}");
}
