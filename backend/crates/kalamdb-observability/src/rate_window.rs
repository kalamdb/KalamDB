//! Fixed one-second admission window.
//!
//! A request loads and updates atomics. It does not take a mutex and it does
//! not wait for a background task. The next request observes the new count.
//! Spawning a task to record the hit would cost more than the atomic update.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::coarse_clock::coarse_unix_millis;

/// Lock-free counter that allows `limit` acquisitions per window.
pub struct AtomicRateWindow {
    limit: u32,
    window_ms: u64,
    window_started_ms: AtomicU64,
    used: AtomicU32,
}

impl AtomicRateWindow {
    /// `limit` acquisitions per second.
    #[inline]
    pub fn per_second(limit: u32) -> Self {
        Self {
            limit,
            window_ms: 1_000,
            window_started_ms: AtomicU64::new(coarse_unix_millis()),
            used: AtomicU32::new(0),
        }
    }

    /// Record one hit when the current window is under the limit.
    ///
    /// Returns `false` once this window has already reached `limit`. The caller
    /// does not block. A later call in the same window is the one that is refused.
    #[inline]
    pub fn try_acquire(&self) -> bool {
        self.roll_window();
        let mut used = self.used.load(Ordering::Relaxed);
        loop {
            if used >= self.limit {
                return false;
            }
            let Some(next) = used.checked_add(1) else {
                return false;
            };
            match self
                .used
                .compare_exchange_weak(used, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return true,
                Err(observed) => used = observed,
            }
        }
    }

    /// Remaining acquisitions in the current window.
    #[inline]
    pub fn available(&self) -> u32 {
        self.roll_window();
        self.limit.saturating_sub(self.used.load(Ordering::Relaxed))
    }

    fn roll_window(&self) {
        let now = coarse_unix_millis();
        let started = self.window_started_ms.load(Ordering::Relaxed);
        if now.saturating_sub(started) < self.window_ms {
            return;
        }
        if self
            .window_started_ms
            .compare_exchange(started, now, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            self.used.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_allows_the_limit_and_refuses_the_next() {
        let window = AtomicRateWindow::per_second(5);
        for _ in 0..5 {
            assert!(window.try_acquire());
        }
        assert!(!window.try_acquire());
        assert_eq!(window.available(), 0);
    }

    #[test]
    fn expired_window_accepts_again() {
        let window = AtomicRateWindow::per_second(1);
        assert!(window.try_acquire());
        assert!(!window.try_acquire());

        let now = coarse_unix_millis();
        window.window_started_ms.store(now.saturating_sub(1_001), Ordering::Relaxed);
        assert!(window.try_acquire());
    }
}
