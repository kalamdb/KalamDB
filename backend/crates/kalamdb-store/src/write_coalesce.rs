//! Task-scoped write buffering for folding independent `put`/`batch` calls into
//! one [`StorageBackend::batch`] per backend.
//!
//! OpenRaft's state-machine worker is a `Send` Tokio task and may resume on
//! another thread after `.await`, so thread-locals are unsafe here. A `u64`
//! Tokio task-local carries an Arc to one apply session's pending operations.
//! Each backend's writes stay on that backend. [`CoalescedBatch::commit`]
//! writes those legs after the session ends.
//!
//! Reads (`get`/`scan`) do not see buffered writes. Callers must not rely on
//! read-your-writes inside the coalesced future. Two backends cannot share
//! one atomic batch: commit writes the session owner last, and an earlier
//! leg can already be durable if a later leg fails.

use std::{
    collections::HashMap,
    future::Future,
    ops::Range,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};

#[cfg(test)]
use std::cell::Cell;

use tokio::task_local;

use crate::storage_trait::{Operation, Partition, StorageBackend, StorageError};

task_local! {
    /// The current Tokio task's apply buffer survives thread migration and
    /// awaits, but is not shared with unrelated Raft apply tasks.
    static COALESCE_STATE: Arc<Mutex<ApplyState>>;
    /// 0 accepts every backend; a non-zero owner accepts its own backend and
    /// registered secondary commit targets.
    static COALESCE_OWNER: usize;
}

type PersistHook = Box<dyn FnOnce() + Send>;
type AsyncPersistHook = Pin<Box<dyn Future<Output = ()> + Send>>;

#[derive(Default)]
struct ApplyState {
    active: bool,
    pending: HashMap<usize, Vec<Operation>>,
    hooks: Vec<PersistHook>,
    async_hooks: Vec<AsyncPersistHook>,
}

fn lock_state(state: &Mutex<ApplyState>) -> std::sync::MutexGuard<'_, ApplyState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn commit_targets() -> &'static Mutex<HashMap<usize, Weak<dyn StorageBackend>>> {
    static MAP: OnceLock<Mutex<HashMap<usize, Weak<dyn StorageBackend>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Remember `backend` so [`CoalescedBatch::commit`] can write the operations
/// buffered under its [`StorageBackend::coalesce_token`].
pub fn bind_commit_target(backend: Arc<dyn StorageBackend>) {
    let token = backend.coalesce_token();
    commit_targets()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(token, Arc::downgrade(&backend));
}

fn commit_target(token: usize) -> Option<Arc<dyn StorageBackend>> {
    commit_targets()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&token)
        .and_then(Weak::upgrade)
}

/// One storage-owned apply batch. Every session owns its pending map, avoiding
/// a process-wide mutex and map lookup for every inserted/deleted row.
pub struct ApplyBatch {
    state: Arc<Mutex<ApplyState>>,
    owner: AtomicUsize,
}

impl ApplyBatch {
    pub fn begin() -> Self {
        Self {
            state: Arc::new(Mutex::new(ApplyState {
                active: true,
                ..ApplyState::default()
            })),
            owner: AtomicUsize::new(0),
        }
    }

    pub async fn enter<F, T>(&self, fut: F) -> T
    where
        F: Future<Output = T>,
    {
        self.enter_owned(0, fut).await
    }

    /// Backends not participating in this session write through as before.
    pub async fn enter_owned<F, T>(&self, owner: usize, fut: F) -> T
    where
        F: Future<Output = T>,
    {
        self.owner.store(owner, Ordering::Relaxed);
        COALESCE_STATE
            .scope(Arc::clone(&self.state), COALESCE_OWNER.scope(owner, fut))
            .await
    }

    pub fn finish(self) -> CoalescedBatch {
        let (legs_map, after_persist, async_hooks) = {
            let mut state = lock_state(&self.state);
            state.active = false;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.hooks),
                std::mem::take(&mut state.async_hooks),
            )
        };

        let mut legs: Vec<(usize, Vec<Operation>)> = legs_map.into_iter().collect();
        legs.sort_by_key(|(token, _)| *token);
        let mut ops = Vec::new();
        let mut ranges = Vec::new();
        for (token, leg) in legs {
            let start = ops.len();
            ops.extend(leg);
            ranges.push((token, start..ops.len()));
        }
        CoalescedBatch {
            ops,
            legs: ranges,
            owner: self.owner.load(Ordering::Relaxed),
            after_persist,
            async_hooks,
        }
    }
}

impl Drop for ApplyBatch {
    fn drop(&mut self) {
        // Clearing here also makes a leaked task-local handle inert after the
        // owning apply scope has ended.
        let mut state = lock_state(&self.state);
        state.active = false;
        state.pending.clear();
        state.hooks.clear();
        state.async_hooks.clear();
    }
}

/// Buffered storage operations plus effects that must run only after they persist.
pub struct CoalescedBatch {
    /// Every buffered operation, in backend-token order.
    ///
    /// Callers that commit one backend themselves pass this to
    /// [`StorageBackend::batch`]. [`Self::commit`] splits it back into legs.
    pub ops: Vec<Operation>,
    legs: Vec<(usize, Range<usize>)>,
    owner: usize,
    pub after_persist: Vec<PersistHook>,
    async_hooks: Vec<AsyncPersistHook>,
}

/// Deferred sync and async hooks that must run only after WAL durability is confirmed.
pub struct PendingEffects {
    after_persist: Vec<PersistHook>,
    async_hooks: Vec<AsyncPersistHook>,
}

impl PendingEffects {
    /// Run synchronous hooks, then await deferred async hooks.
    pub async fn finish(self) {
        for hook in self.after_persist {
            hook();
        }
        for hook in self.async_hooks {
            hook.await;
        }
    }
}

impl CoalescedBatch {
    fn take_ordered_pieces(&mut self) -> Vec<(usize, Vec<Operation>)> {
        let ops = std::mem::take(&mut self.ops);
        let mut remaining = ops;
        let mut pieces = Vec::with_capacity(self.legs.len());
        for (token, range) in std::mem::take(&mut self.legs) {
            let len = range.end.saturating_sub(range.start);
            let tail = remaining.split_off(len.min(remaining.len()));
            pieces.push((token, remaining));
            remaining = tail;
        }
        let owner = self.owner;
        pieces.sort_by_key(|(token, _)| (*token == owner, *token));
        pieces
    }

    /// Write each backend's buffered operations, then run after-persist hooks.
    ///
    /// The session owner is written last. Hooks run only after every leg
    /// succeeds. An unbound token is an error and does not run the hooks.
    pub fn commit(mut self) -> Result<(), StorageError> {
        for (token, leg) in self.take_ordered_pieces() {
            if leg.is_empty() {
                continue;
            }
            let backend = commit_target(token).ok_or_else(|| {
                StorageError::Other(format!(
                    "coalesced writes for backend {token} have no commit target"
                ))
            })?;
            backend.batch(leg)?;
        }
        for hook in std::mem::take(&mut self.after_persist) {
            hook();
        }
        Ok(())
    }

    /// Write each backend's buffered operations via [`StorageBackend::batch_unsynced`]
    /// and return the written backends plus deferred hooks.
    ///
    /// Callers must call [`StorageBackend::sync_wal`] on every returned backend
    /// before calling [`PendingEffects::finish`] or acknowledging durability.
    pub fn commit_unsynced(
        mut self,
    ) -> Result<(Vec<Arc<dyn StorageBackend>>, PendingEffects), StorageError> {
        let pieces = self.take_ordered_pieces();
        let mut backends = Vec::with_capacity(pieces.len());
        for (token, leg) in pieces {
            if leg.is_empty() {
                continue;
            }
            let backend = commit_target(token).ok_or_else(|| {
                StorageError::Other(format!(
                    "coalesced writes for backend {token} have no commit target"
                ))
            })?;
            backend.batch_unsynced(leg)?;
            backends.push(backend);
        }
        let effects = PendingEffects {
            after_persist: std::mem::take(&mut self.after_persist),
            async_hooks: std::mem::take(&mut self.async_hooks),
        };
        Ok((backends, effects))
    }

    /// Persist buffered writes, run synchronous hooks, then await deferred async work.
    ///
    /// Async work runs only after every leg is durable, and still before the
    /// caller observes success. A failed commit drops that work.
    pub async fn commit_await(mut self) -> Result<(), StorageError> {
        let async_hooks = std::mem::take(&mut self.async_hooks);
        self.commit()?;
        for hook in async_hooks {
            hook.await;
        }
        Ok(())
    }
}

/// Run `fut` while `put`/`batch` on participating backends append to one buffer.
///
/// Returns the future's output and the buffered operations. The caller commits
/// with [`StorageBackend::batch`] only when the future succeeded, then runs
/// `after_persist`. On failure both are discarded.
pub async fn with_write_coalesce<F, T>(fut: F) -> (T, CoalescedBatch)
where
    F: Future<Output = T>,
{
    let batch = ApplyBatch::begin();
    let value = batch.enter(fut).await;
    (value, batch.finish())
}

/// Whether this task is inside [`with_write_coalesce`].
///
/// `StorageBackend::batch` calls from that scope are absorbed and are not a
/// durable submission. The caller writes the buffered operations once.
pub fn coalesce_session_active() -> bool {
    COALESCE_STATE.try_with(|state| lock_state(state).active).unwrap_or(false)
}

/// Defer a hook until *every* participating backend has durably committed.
pub fn defer_after_persist(hook: impl FnOnce() + Send + 'static) {
    let mut hook = Some(hook);
    let deferred = COALESCE_STATE
        .try_with(|state| {
            let mut state = lock_state(state);
            if state.active {
                if let Some(hook) = hook.take() {
                    state.hooks.push(Box::new(hook));
                }
                true
            } else {
                false
            }
        })
        .unwrap_or(false);
    if !deferred {
        if let Some(hook) = hook.take() {
            hook();
        }
    }
}

/// Defer async work until [`CoalescedBatch::commit_await`] has persisted every leg.
///
/// Outside a coalesce session the future is not stored. Callers must await the
/// work themselves in that case.
pub fn defer_after_persist_async(hook: impl Future<Output = ()> + Send + 'static) -> bool {
    let mut hook = Some(Box::pin(hook));
    COALESCE_STATE
        .try_with(|state| {
            let mut state = lock_state(state);
            if state.active {
                if let Some(hook) = hook.take() {
                    state.async_hooks.push(hook);
                }
                true
            } else {
                false
            }
        })
        .unwrap_or(false)
}

// Copies of a key or value made while building a buffered operation.
// Counting is per thread and stays off unless a test enables it, so parallel
// coalesce tests do not share one total.
#[cfg(test)]
thread_local! {
    static COUNT_PAYLOAD_COPIES: Cell<bool> = const { Cell::new(false) };
    static BUFFER_PAYLOAD_COPIES: Cell<u64> = const { Cell::new(0) };
}

#[cfg(test)]
fn record_payload_copies(count: u64) {
    if COUNT_PAYLOAD_COPIES.with(Cell::get) {
        BUFFER_PAYLOAD_COPIES.with(|copies| copies.set(copies.get() + count));
    }
}

#[cfg(test)]
fn payload_copy_count() -> u64 {
    BUFFER_PAYLOAD_COPIES.with(Cell::get)
}

#[cfg(test)]
fn reset_payload_copy_count() {
    COUNT_PAYLOAD_COPIES.with(|flag| flag.set(true));
    BUFFER_PAYLOAD_COPIES.with(|copies| copies.set(0));
}

fn should_buffer(backend_ptr: usize) -> bool {
    // Avoid both a task-local mutex and a backend-registry lookup outside
    // coalesced Raft apply; the normal write path must remain near zero cost.
    if !coalesce_session_active() {
        return false;
    }
    let owner = COALESCE_OWNER.try_with(|owner| *owner).unwrap_or(0);
    owner == 0 || owner == backend_ptr || commit_target(backend_ptr).is_some()
}

pub fn buffer_put(backend_ptr: usize, partition: &Partition, key: &[u8], value: &[u8]) -> bool {
    if !should_buffer(backend_ptr) {
        return false;
    }
    #[cfg(test)]
    record_payload_copies(2);
    buffer_operation(
        backend_ptr,
        Operation::Put {
            partition: partition.clone(),
            key: key.to_vec(),
            value: value.to_vec(),
        },
    )
}

pub fn buffer_delete(backend_ptr: usize, partition: &Partition, key: &[u8]) -> bool {
    if !should_buffer(backend_ptr) {
        return false;
    }
    #[cfg(test)]
    record_payload_copies(1);
    buffer_operation(
        backend_ptr,
        Operation::Delete {
            partition: partition.clone(),
            key: key.to_vec(),
        },
    )
}

pub fn buffer_operations(
    backend_ptr: usize,
    mut operations: Vec<Operation>,
) -> Option<Vec<Operation>> {
    if !should_buffer(backend_ptr) {
        return Some(operations);
    }
    let buffered = COALESCE_STATE
        .try_with(|state| {
            let mut state = lock_state(state);
            if !state.active {
                return false;
            }
            state.pending.entry(backend_ptr).or_default().append(&mut operations);
            true
        })
        .unwrap_or(false);
    if buffered {
        None
    } else {
        Some(operations)
    }
}

fn buffer_operation(backend_ptr: usize, op: Operation) -> bool {
    COALESCE_STATE
        .try_with(|state| {
            let mut state = lock_state(state);
            if !state.active {
                return false;
            }
            state.pending.entry(backend_ptr).or_default().push(op);
            true
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{test_utils::InMemoryBackend, StorageBackend};

    #[tokio::test]
    async fn coalesced_puts_are_invisible_until_commit() {
        let backend = Arc::new(InMemoryBackend::new());
        let partition = Partition::new("coalesce");
        backend.create_partition(&partition).unwrap();

        let writer = Arc::clone(&backend);
        let partition_for_write = partition.clone();
        let (seen, batch) = with_write_coalesce(async move {
            writer.put(&partition_for_write, b"k", b"v").unwrap();
            writer.get(&partition_for_write, b"k").unwrap()
        })
        .await;

        assert_eq!(seen, None);
        assert_eq!(batch.ops.len(), 1);
        assert_eq!(backend.get(&partition, b"k").unwrap(), None);

        backend.batch(batch.ops).unwrap();
        assert_eq!(backend.get(&partition, b"k").unwrap(), Some(b"v".to_vec()));
    }

    #[tokio::test]
    async fn batch_and_put_fold_into_one_commit() {
        let backend = Arc::new(InMemoryBackend::new());
        let hot = Partition::new("hot_data");
        let raft = Partition::new("raft_data");
        backend.create_partition(&hot).unwrap();
        backend.create_partition(&raft).unwrap();

        let writer = Arc::clone(&backend);
        let hot_write = hot.clone();
        let raft_write = raft.clone();
        let (_ok, batch) = with_write_coalesce(async move {
            writer
                .batch(vec![Operation::Put {
                    partition: hot_write.clone(),
                    key: b"row".to_vec(),
                    value: b"table".to_vec(),
                }])
                .unwrap();
            writer.put(&raft_write, b"log:1", b"entry").unwrap();
            Ok::<(), ()>(())
        })
        .await;

        assert_eq!(batch.ops.len(), 2);
        assert_eq!(backend.get(&hot, b"row").unwrap(), None);
        assert_eq!(backend.get(&raft, b"log:1").unwrap(), None);

        backend.batch(batch.ops).unwrap();
        assert_eq!(backend.get(&hot, b"row").unwrap(), Some(b"table".to_vec()));
        assert_eq!(backend.get(&raft, b"log:1").unwrap(), Some(b"entry".to_vec()));
    }

    #[tokio::test]
    async fn deferred_hook_is_not_part_of_the_buffered_batch() {
        let mut ran = false;
        let ((), batch) = with_write_coalesce(async {
            defer_after_persist(|| {});
        })
        .await;
        assert!(batch.ops.is_empty());
        assert_eq!(batch.after_persist.len(), 1);
        for hook in batch.after_persist {
            hook();
            ran = true;
        }
        assert!(ran);
    }

    #[test]
    fn puts_write_through_outside_coalesce_scope() {
        let backend = InMemoryBackend::new();
        let partition = Partition::new("direct");
        backend.create_partition(&partition).unwrap();
        backend.put(&partition, b"k", b"v").unwrap();
        assert_eq!(backend.get(&partition, b"k").unwrap(), Some(b"v".to_vec()));
    }

    #[test]
    fn inactive_buffer_helpers_return_before_copying_payloads() {
        let partition = Partition::new("direct");
        let key = vec![1_u8; 64];
        let value = vec![2_u8; 256];
        reset_payload_copy_count();
        for _ in 0..10_000 {
            assert!(!buffer_put(0, &partition, &key, &value));
            assert!(!buffer_delete(0, &partition, &key));
        }
        assert_eq!(payload_copy_count(), 0);
        let returned = buffer_operations(
            0,
            vec![Operation::Put {
                partition: partition.clone(),
                key: key.clone(),
                value: value.clone(),
            }],
        )
        .expect("inactive batch stays a direct write");
        assert_eq!(returned.len(), 1);
    }

    #[tokio::test]
    async fn active_buffer_put_still_copies_into_the_session() {
        let partition = Partition::new("coalesce");
        let key = vec![1_u8; 64];
        let value = vec![2_u8; 256];
        let (copies, batch) = with_write_coalesce(async {
            reset_payload_copy_count();
            for _ in 0..10_000 {
                assert!(buffer_put(0, &partition, &key, &value));
            }
            payload_copy_count()
        })
        .await;
        assert_eq!(batch.ops.len(), 10_000);
        assert_eq!(copies, 20_000);
    }

    #[tokio::test]
    async fn each_backend_commits_its_own_buffered_writes() {
        let tables = Arc::new(InMemoryBackend::new());
        let raft = Arc::new(InMemoryBackend::new());
        let tables_dyn: Arc<dyn StorageBackend> = Arc::clone(&tables) as _;
        let raft_dyn: Arc<dyn StorageBackend> = Arc::clone(&raft) as _;
        assert_eq!(tables.coalesce_token(), tables_dyn.coalesce_token());
        assert_eq!(raft.coalesce_token(), raft_dyn.coalesce_token());
        bind_commit_target(Arc::clone(&tables_dyn));
        bind_commit_target(Arc::clone(&raft_dyn));

        let rows = Partition::new("rows");
        let log = Partition::new("log");
        tables.create_partition(&rows).unwrap();
        raft.create_partition(&log).unwrap();

        let batch = ApplyBatch::begin();
        let tables_write = Arc::clone(&tables);
        let raft_write = Arc::clone(&raft);
        let rows_write = rows.clone();
        let log_write = log.clone();
        batch
            .enter_owned(raft.coalesce_token(), async move {
                tables_write.put(&rows_write, b"row", b"v").unwrap();
                raft_write.put(&log_write, b"entry", b"e").unwrap();
            })
            .await;
        let finished = batch.finish();
        assert_eq!(finished.ops.len(), 2);
        assert_eq!(tables.get(&rows, b"row").unwrap(), None);
        assert_eq!(raft.get(&log, b"entry").unwrap(), None);

        finished.commit().unwrap();
        assert_eq!(tables.get(&rows, b"row").unwrap(), Some(b"v".to_vec()));
        assert_eq!(raft.get(&log, b"entry").unwrap(), Some(b"e".to_vec()));
    }
}
