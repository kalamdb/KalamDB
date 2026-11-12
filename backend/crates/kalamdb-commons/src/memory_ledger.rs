//! Process-wide admission for memory this process retains.
//!
//! Tracked bytes are logical reservations. They are not resident-set size.
//! Allocator caches can keep RSS after [`MemoryLedger::release`]. RocksDB
//! memtables and block cache are not in this ledger; the store owns those.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// How many full per-transaction buffers the process admits at once.
///
/// The per-transaction cap stays `max_transaction_buffer_bytes`. This multiplier
/// is the process cap for staged payload and for the overlay copy of those
/// payloads. It is not a separate configuration key.
pub const PROCESS_TRANSACTION_SLOTS: usize = 8;

/// Bytes charged for one overlay map entry beyond the cloned payload.
pub const OVERLAY_ENTRY_BYTES: usize = 64;

/// One kind of retained bytes. Each kind has its own process counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryClass {
    /// Staged mutation payloads and the history that still owns them.
    StagedPayload,
    /// Overlay and map capacity, including the cloned payload.
    Overlay,
    /// Scan and decoded Arrow batches.
    ScanBatch,
    /// Commit target-resolution cache.
    TargetCache,
    /// Encoded storage operations for one commit batch.
    EncodedBatch,
    /// Notifications, file cleanup, and other post-commit payloads.
    SideEffects,
}

impl MemoryClass {
    fn index(self) -> usize {
        match self {
            Self::StagedPayload => 0,
            Self::Overlay => 1,
            Self::ScanBatch => 2,
            Self::TargetCache => 3,
            Self::EncodedBatch => 4,
            Self::SideEffects => 5,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::StagedPayload => "staged_payload",
            Self::Overlay => "overlay",
            Self::ScanBatch => "scan_batch",
            Self::TargetCache => "target_cache",
            Self::EncodedBatch => "encoded_batch",
            Self::SideEffects => "side_effects",
        }
    }
}

const CLASS_COUNT: usize = 6;

/// Process caps, in bytes. A zero cap admits nothing above zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryBudgets {
    pub staged_payload: usize,
    pub overlay: usize,
    pub scan_batch: usize,
    pub target_cache: usize,
    pub encoded_batch: usize,
    pub side_effects: usize,
}

impl MemoryBudgets {
    /// Caps from the existing per-transaction buffer and the DataFusion limit.
    ///
    /// Staged payload, overlay, target cache, encoded batches, and side effects
    /// share the process slot multiplier. Scan batches use the DataFusion limit
    /// so a query workspace cannot grow by raising the transaction buffer.
    pub fn from_existing(
        max_transaction_buffer_bytes: usize,
        datafusion_memory_limit: usize,
    ) -> Self {
        let process_bytes = max_transaction_buffer_bytes.saturating_mul(PROCESS_TRANSACTION_SLOTS);
        Self {
            staged_payload: process_bytes,
            overlay: process_bytes,
            scan_batch: datafusion_memory_limit,
            target_cache: process_bytes,
            encoded_batch: process_bytes,
            side_effects: process_bytes,
        }
    }

    fn for_class(self, class: MemoryClass) -> usize {
        match class {
            MemoryClass::StagedPayload => self.staged_payload,
            MemoryClass::Overlay => self.overlay,
            MemoryClass::ScanBatch => self.scan_batch,
            MemoryClass::TargetCache => self.target_cache,
            MemoryClass::EncodedBatch => self.encoded_batch,
            MemoryClass::SideEffects => self.side_effects,
        }
    }
}

/// Refusal from [`MemoryLedger::try_reserve`]. The caller has not retained the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBudgetError {
    pub class: MemoryClass,
    pub requested: usize,
    pub held: usize,
    pub limit: usize,
}

impl std::fmt::Display for MemoryBudgetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "process {} budget exceeded: {} bytes would raise {} above {}",
            self.class.name(),
            self.requested,
            self.held,
            self.limit
        )
    }
}

impl std::error::Error for MemoryBudgetError {}

/// Process-wide reservation counters.
///
/// [`Self::try_reserve`] and [`Self::release`] use atomics. They do not take a
/// lock, so a caller can hold a map reference across the call and must not
/// hold any lock across `.await` merely to keep a reservation.
#[derive(Debug)]
pub struct MemoryLedger {
    held: [AtomicUsize; CLASS_COUNT],
    limits: [usize; CLASS_COUNT],
}

impl MemoryLedger {
    pub fn new(budgets: MemoryBudgets) -> Self {
        Self {
            held: std::array::from_fn(|_| AtomicUsize::new(0)),
            limits: std::array::from_fn(|index| {
                let class = match index {
                    0 => MemoryClass::StagedPayload,
                    1 => MemoryClass::Overlay,
                    2 => MemoryClass::ScanBatch,
                    3 => MemoryClass::TargetCache,
                    4 => MemoryClass::EncodedBatch,
                    _ => MemoryClass::SideEffects,
                };
                budgets.for_class(class)
            }),
        }
    }

    pub fn held(&self, class: MemoryClass) -> usize {
        self.held[class.index()].load(Ordering::Relaxed)
    }

    pub fn limit(&self, class: MemoryClass) -> usize {
        self.limits[class.index()]
    }

    /// Reserve `bytes` before retaining them. Zero always succeeds.
    pub fn try_reserve(&self, class: MemoryClass, bytes: usize) -> Result<(), MemoryBudgetError> {
        if bytes == 0 {
            return Ok(());
        }
        let counter = &self.held[class.index()];
        let limit = self.limits[class.index()];
        let mut current = counter.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_add(bytes) else {
                return Err(MemoryBudgetError {
                    class,
                    requested: bytes,
                    held: current,
                    limit,
                });
            };
            if next > limit {
                return Err(MemoryBudgetError {
                    class,
                    requested: bytes,
                    held: current,
                    limit,
                });
            }
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }

    /// Return a reservation. Extra releases stop at zero.
    pub fn release(&self, class: MemoryClass, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let counter = &self.held[class.index()];
        let mut current = counter.load(Ordering::Relaxed);
        loop {
            let next = current.saturating_sub(bytes);
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Relaxed)
            {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    /// Charge a staged payload and the overlay copy, or charge neither.
    pub fn try_reserve_staged(
        &self,
        payload_bytes: usize,
        mutations: usize,
    ) -> Result<(), MemoryBudgetError> {
        self.try_reserve(MemoryClass::StagedPayload, payload_bytes)?;
        let overlay = payload_bytes.saturating_add(mutations.saturating_mul(OVERLAY_ENTRY_BYTES));
        if let Err(error) = self.try_reserve(MemoryClass::Overlay, overlay) {
            self.release(MemoryClass::StagedPayload, payload_bytes);
            return Err(error);
        }
        Ok(())
    }

    /// Drop the charge from [`Self::try_reserve_staged`].
    pub fn release_staged(&self, payload_bytes: usize, mutations: usize) {
        self.release(MemoryClass::StagedPayload, payload_bytes);
        let overlay = payload_bytes.saturating_add(mutations.saturating_mul(OVERLAY_ENTRY_BYTES));
        self.release(MemoryClass::Overlay, overlay);
    }
}

/// Releases its bytes when dropped, including on error and cancellation paths
/// that leave the scope.
#[derive(Debug)]
pub struct Reservation {
    ledger: Arc<MemoryLedger>,
    class: MemoryClass,
    bytes: usize,
}

impl Reservation {
    pub fn reserve(
        ledger: &Arc<MemoryLedger>,
        class: MemoryClass,
        bytes: usize,
    ) -> Result<Self, MemoryBudgetError> {
        ledger.try_reserve(class, bytes)?;
        Ok(Self {
            ledger: Arc::clone(ledger),
            class,
            bytes,
        })
    }

    /// Own bytes that were already charged with [`MemoryLedger::try_reserve`].
    pub fn adopt(ledger: &Arc<MemoryLedger>, class: MemoryClass, bytes: usize) -> Self {
        Self {
            ledger: Arc::clone(ledger),
            class,
            bytes,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.ledger.release(self.class, self.bytes);
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger(limit: usize) -> MemoryLedger {
        MemoryLedger::new(MemoryBudgets {
            staged_payload: limit,
            overlay: limit,
            scan_batch: limit / 2,
            target_cache: limit,
            encoded_batch: limit,
            side_effects: limit,
        })
    }

    #[test]
    fn reserve_stops_at_the_class_limit_and_release_reopens_it() {
        let ledger = ledger(100);
        ledger.try_reserve(MemoryClass::StagedPayload, 100).expect("at the limit");
        let error = ledger.try_reserve(MemoryClass::StagedPayload, 1).expect_err("over");
        assert_eq!(error.class, MemoryClass::StagedPayload);
        assert_eq!(error.limit, 100);
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 100);

        ledger.try_reserve(MemoryClass::ScanBatch, 50).expect("scan has its own cap");
        assert!(ledger.try_reserve(MemoryClass::ScanBatch, 1).is_err());
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 100);

        ledger.release(MemoryClass::StagedPayload, 40);
        ledger.try_reserve(MemoryClass::StagedPayload, 40).expect("reopened");
        ledger.release(MemoryClass::StagedPayload, 10_000);
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 0);
    }

    #[test]
    fn staged_charge_covers_the_overlay_copy_or_neither() {
        let ledger = ledger(200);
        let error = ledger.try_reserve_staged(150, 1).expect_err("overlay copy does not fit");
        assert_eq!(error.class, MemoryClass::Overlay);
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 0);
        assert_eq!(ledger.held(MemoryClass::Overlay), 0);

        ledger.try_reserve_staged(20, 2).expect("payload and overlay");
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 20);
        assert_eq!(ledger.held(MemoryClass::Overlay), 20 + 2 * OVERLAY_ENTRY_BYTES);
        ledger.release_staged(20, 2);
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 0);
        assert_eq!(ledger.held(MemoryClass::Overlay), 0);
    }

    #[test]
    fn reservation_drop_releases_the_bytes() {
        let ledger = Arc::new(ledger(64));
        {
            let _held = Reservation::reserve(&ledger, MemoryClass::EncodedBatch, 64).expect("hold");
            assert!(Reservation::reserve(&ledger, MemoryClass::EncodedBatch, 1).is_err());
        }
        assert_eq!(ledger.held(MemoryClass::EncodedBatch), 0);
        Reservation::reserve(&ledger, MemoryClass::EncodedBatch, 64).expect("after drop");
    }

    #[test]
    fn existing_configs_set_scan_and_process_caps() {
        let budgets = MemoryBudgets::from_existing(1_000, 32);
        assert_eq!(budgets.staged_payload, 1_000 * PROCESS_TRANSACTION_SLOTS);
        assert_eq!(budgets.encoded_batch, budgets.staged_payload);
        assert_eq!(budgets.scan_batch, 32);
    }

    #[test]
    fn concurrent_transactions_share_the_process_cap() {
        // Each staged(20,1) needs overlay 20 + OVERLAY_ENTRY_BYTES, so a 100-byte
        // dual cap admits at most one concurrent transaction.
        let ledger = Arc::new(ledger(100));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let ledger = Arc::clone(&ledger);
            handles.push(std::thread::spawn(move || {
                ledger.try_reserve_staged(20, 1).is_ok()
            }));
        }
        let admitted: usize = handles
            .into_iter()
            .map(|handle| usize::from(handle.join().expect("join")))
            .sum();
        assert_eq!(admitted, 1);
        assert!(ledger.held(MemoryClass::StagedPayload) <= 100);
        assert!(ledger.held(MemoryClass::Overlay) <= 100);
    }

    #[test]
    fn many_small_rows_and_one_wide_row_hit_predictable_bounds() {
        // Overlay charge is payload + mutations * OVERLAY_ENTRY_BYTES, so keep
        // the dual cap high enough for many small rows but not the wide add-on.
        let ledger = Arc::new(ledger(2_000));
        for _ in 0..20 {
            ledger.try_reserve_staged(10, 1).expect("small rows fit");
        }
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 200);
        let wide = ledger
            .try_reserve_staged(1_900, 1)
            .expect_err("wide row exceeds remaining staged budget");
        assert_eq!(wide.class, MemoryClass::StagedPayload);
        ledger.release_staged(200, 20);
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 0);
        ledger.try_reserve_staged(1_900, 1).expect("wide row after cleanup");
        ledger.release_staged(1_900, 1);
        assert_eq!(ledger.held(MemoryClass::StagedPayload), 0);
        assert_eq!(ledger.held(MemoryClass::Overlay), 0);
    }
}
