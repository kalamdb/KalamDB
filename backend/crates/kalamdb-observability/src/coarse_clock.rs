//! Background clock for request paths that only need about 10ms resolution.
//!
//! Session expiry, rate-limit windows, and idle tracking do not need a fresh
//! `SystemTime` read on every request. One thread publishes the time. Callers
//! load an atomic.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Once,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static COARSE_UNIX_MS: AtomicU64 = AtomicU64::new(0);
static COARSE_UNIX_SECS: AtomicU64 = AtomicU64::new(0);
static CLOCK_RUNNING: AtomicBool = AtomicBool::new(false);

fn read_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn publish(millis: u64) {
    COARSE_UNIX_MS.store(millis, Ordering::Relaxed);
    COARSE_UNIX_SECS.store(millis / 1000, Ordering::Relaxed);
}

/// Start the process-wide 10ms clock. Safe to call more than once.
pub fn start_coarse_clock() {
    static START: Once = Once::new();
    START.call_once(|| {
        publish(read_unix_millis());
        if thread::Builder::new()
            .name("kalamdb-clock".to_string())
            .spawn(|| loop {
                thread::sleep(Duration::from_millis(10));
                publish(read_unix_millis());
            })
            .is_ok()
        {
            CLOCK_RUNNING.store(true, Ordering::Relaxed);
        }
    });
}

/// Unix milliseconds from the background clock.
///
/// Falls back to a direct read when the clock thread did not start.
#[inline]
pub fn coarse_unix_millis() -> u64 {
    start_coarse_clock();
    if CLOCK_RUNNING.load(Ordering::Relaxed) {
        COARSE_UNIX_MS.load(Ordering::Relaxed)
    } else {
        read_unix_millis()
    }
}

/// Unix seconds from the background clock.
#[inline]
pub fn coarse_unix_secs() -> u64 {
    start_coarse_clock();
    if CLOCK_RUNNING.load(Ordering::Relaxed) {
        COARSE_UNIX_SECS.load(Ordering::Relaxed)
    } else {
        read_unix_millis() / 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_clock_matches_the_wall_clock() {
        let coarse = coarse_unix_millis();
        let wall = read_unix_millis();
        assert!(coarse.abs_diff(wall) < 1_000);
        assert!(coarse_unix_secs().abs_diff(wall / 1000) <= 1);
    }
}
