use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// Counts queued and running work until cleanup is acknowledged.
pub(crate) struct LaneCharge(pub Arc<AtomicUsize>);
impl Drop for LaneCharge {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
