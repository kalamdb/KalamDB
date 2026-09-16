//! Isolate and deployment lifecycle events. The engine emits; `kalamdb-core` persists them.

use std::sync::Arc;

/// What happened to a resident isolate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionLifecycleKind {
    Created,
    Reused,
    Idle,
    Dropped,
}

impl FunctionLifecycleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Reused => "reused",
            Self::Idle => "idle",
            Self::Dropped => "dropped",
        }
    }

    /// Warm reuse and idle parking are per-call; keep them quieter than create/drop.
    pub fn log_level(self) -> &'static str {
        match self {
            Self::Reused | Self::Idle => "debug",
            Self::Created | Self::Dropped => "info",
        }
    }
}

/// One isolate state change. Isolates are per module revision, not per procedure.
#[derive(Debug, Clone)]
pub struct FunctionLifecycleEvent {
    pub kind:            FunctionLifecycleKind,
    pub reason:          &'static str,
    pub instance_id:     u64,
    pub worker:          usize,
    pub module_id:       String,
    pub revision_id:     String,
    pub procedure_id:    Option<String>,
    pub reserved_bytes:  u64,
    pub used_heap_bytes: u64,
    /// Calls served by this isolate so far; lets a single `dropped` record summarise its life.
    pub invocations:     u64,
}

/// Receives isolate lifecycle events. Implementations must not block the V8 worker for long.
///
/// `Created`/`Dropped` fire once per isolate. `Reused`/`Idle` are rate-limited by the engine to
/// one per isolate per minute, so the observer's cost is not on the per-call path.
pub trait FunctionLifecycleObserver: Send + Sync {
    fn on_lifecycle(&self, event: FunctionLifecycleEvent);
}

#[derive(Clone)]
pub(crate) struct LifecycleHandle(pub Arc<dyn FunctionLifecycleObserver>);

pub(crate) type LifecycleSlot = Arc<arc_swap::ArcSwapOption<LifecycleHandle>>;

pub(crate) fn store_observer(slot: &LifecycleSlot, observer: Arc<dyn FunctionLifecycleObserver>) {
    slot.store(Some(Arc::new(LifecycleHandle(observer))));
}
