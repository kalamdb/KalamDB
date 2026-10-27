//! Shared clocks. Session leases, wire startup, and the backend registry use one
//! millisecond clock so expiry checks agree.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch. A clock before the epoch returns `0`.
#[inline]
pub fn unix_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
