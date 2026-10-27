use std::{
    future::Future,
    sync::{Condvar, Mutex, OnceLock},
};

use crate::error::{FilestoreError, Result};

const BLOCKING_SLOTS: usize = 8;

struct BlockingSlots {
    available: Mutex<usize>,
    ready: Condvar,
}

struct BlockingSlotGuard;

impl Drop for BlockingSlotGuard {
    fn drop(&mut self) {
        let slots = blocking_slots();
        let mut available = slots.available.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *available = available.saturating_add(1);
        slots.ready.notify_one();
    }
}

fn blocking_slots() -> &'static BlockingSlots {
    static SLOTS: OnceLock<BlockingSlots> = OnceLock::new();
    SLOTS.get_or_init(|| BlockingSlots {
        available: Mutex::new(BLOCKING_SLOTS),
        ready: Condvar::new(),
    })
}

fn acquire_blocking_slot() -> BlockingSlotGuard {
    let slots = blocking_slots();
    let mut available = slots.available.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    while *available == 0 {
        available = slots
            .ready
            .wait(available)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    *available -= 1;
    BlockingSlotGuard
}

/// Run an async operation in a synchronous context.
///
/// This function handles the tricky case of needing to call async code from sync context.
///
/// **Strategy**:
/// - If we're in a tokio multi-thread runtime context, use `block_in_place` which allows blocking
///   while letting other tasks run on other threads
/// - If we're in a tokio current-thread runtime (common in tests), spawn the work on a background
///   thread that uses a shared runtime to avoid nested block_on calls
/// - If no runtime exists, use the shared runtime's block_on
///
/// **Why this matters for object_store**:
/// Remote backends (S3, GCS, Azure) use the tokio I/O driver for networking.
/// Calling `handle.block_on()` from within the runtime thread will deadlock because
/// the network I/O futures need the runtime thread to poll them.
pub fn run_blocking<F, Fut, T>(make_future: F) -> Result<T>
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<T>> + Send,
    T: Send,
{
    fn shared_blocking_runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to create shared filestore runtime")
        })
    }

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // We're inside a tokio runtime. Check if we can use block_in_place.
            // block_in_place is only allowed in multi-thread runtime, not current_thread.
            match handle.runtime_flavor() {
                tokio::runtime::RuntimeFlavor::MultiThread => {
                    // Safe to use block_in_place - this lets us block while the runtime
                    // continues to service I/O on other threads
                    tokio::task::block_in_place(|| handle.block_on(make_future()))
                },
                _ => {
                    // A current-thread runtime cannot block_on its own I/O. The future borrows
                    // the caller, so it cannot move onto a 'static task. One scoped thread
                    // drives the shared multi-thread runtime, and the slot cap bounds how many
                    // of those bridges exist at once.
                    let _slot = acquire_blocking_slot();
                    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
                    std::thread::scope(|scope| {
                        scope.spawn(|| {
                            let result = shared_blocking_runtime().block_on(make_future());
                            let _ = sender.send(result);
                        });
                        receiver.recv().map_err(|_| {
                            FilestoreError::Other(
                                "filestore runtime dropped a blocking task before it returned"
                                    .to_string(),
                            )
                        })
                    })?
                },
            }
        },
        Err(_) => {
            // No tokio runtime in current thread - safe to use block_on
            shared_blocking_runtime().block_on(make_future())
        },
    }
}
