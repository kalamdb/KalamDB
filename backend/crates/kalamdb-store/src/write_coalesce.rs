//! Task-scoped write buffering for folding independent `put`/`batch` calls into
//! one [`StorageBackend::batch`].
//!
//! OpenRaft's state-machine worker is a `Send` Tokio task and may resume on
//! another thread after `.await`, so thread-locals are unsafe here. A `u64`
//! Tokio task-local (Send) keys a process-wide map of pending operations.
//!
//! Reads (`get`/`scan`) do not see buffered writes. Callers must not rely on
//! read-your-writes inside the coalesced future.

use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard, OnceLock,
    },
};

use tokio::task_local;

use crate::storage_trait::{Operation, Partition};

task_local! {
    static COALESCE_ID: u64;
}

fn pending_map() -> &'static Mutex<HashMap<u64, Vec<Operation>>> {
    static MAP: OnceLock<Mutex<HashMap<u64, Vec<Operation>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_pending() -> MutexGuard<'static, HashMap<u64, Vec<Operation>>> {
    pending_map().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct CoalesceGuard(u64);

impl Drop for CoalesceGuard {
    fn drop(&mut self) {
        lock_pending().remove(&self.0);
    }
}

/// Run `fut` while `put`/`batch` on participating backends append to one buffer.
///
/// Returns the future's output and the buffered operations. The caller must
/// commit with [`StorageBackend::batch`] (or discard them).
pub async fn with_write_coalesce<F, T>(fut: F) -> (T, Vec<Operation>)
where
    F: Future<Output = T>,
{
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    lock_pending().insert(id, Vec::with_capacity(8));
    let guard = CoalesceGuard(id);

    let value = COALESCE_ID.scope(id, fut).await;
    let ops = lock_pending().remove(&id).unwrap_or_default();
    drop(guard);
    (value, ops)
}

/// Buffer a put if a coalesce session is active on this task.
pub fn buffer_put(partition: &Partition, key: &[u8], value: &[u8]) -> bool {
    buffer_operation(Operation::Put {
        partition: partition.clone(),
        key:       key.to_vec(),
        value:     value.to_vec(),
    })
}

/// Buffer a delete if a coalesce session is active on this task.
pub fn buffer_delete(partition: &Partition, key: &[u8]) -> bool {
    buffer_operation(Operation::Delete {
        partition: partition.clone(),
        key:       key.to_vec(),
    })
}

/// Absorb `operations` into the active session.
///
/// Returns `None` when they were buffered, or `Some(operations)` when this task
/// is not coalescing and the caller should write them immediately.
pub fn buffer_operations(mut operations: Vec<Operation>) -> Option<Vec<Operation>> {
    let Ok(buffered) = COALESCE_ID.try_with(|id| {
        if let Some(pending) = lock_pending().get_mut(id) {
            pending.append(&mut operations);
            true
        } else {
            false
        }
    }) else {
        return Some(operations);
    };

    if buffered {
        None
    } else {
        Some(operations)
    }
}

fn buffer_operation(op: Operation) -> bool {
    COALESCE_ID
        .try_with(|id| {
            if let Some(pending) = lock_pending().get_mut(id) {
                pending.push(op);
                true
            } else {
                false
            }
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
        let (seen, ops) = with_write_coalesce(async move {
            writer.put(&partition_for_write, b"k", b"v").unwrap();
            writer.get(&partition_for_write, b"k").unwrap()
        })
        .await;

        assert_eq!(seen, None);
        assert_eq!(ops.len(), 1);
        assert_eq!(backend.get(&partition, b"k").unwrap(), None);

        backend.batch(ops).unwrap();
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
        let (_ok, ops) = with_write_coalesce(async move {
            writer
                .batch(vec![Operation::Put {
                    partition: hot_write.clone(),
                    key:       b"row".to_vec(),
                    value:     b"table".to_vec(),
                }])
                .unwrap();
            writer.put(&raft_write, b"log:1", b"entry").unwrap();
            Ok::<(), ()>(())
        })
        .await;

        assert_eq!(ops.len(), 2);
        assert_eq!(backend.get(&hot, b"row").unwrap(), None);
        assert_eq!(backend.get(&raft, b"log:1").unwrap(), None);

        backend.batch(ops).unwrap();
        assert_eq!(backend.get(&hot, b"row").unwrap(), Some(b"table".to_vec()));
        assert_eq!(backend.get(&raft, b"log:1").unwrap(), Some(b"entry".to_vec()));
    }

    #[test]
    fn puts_write_through_outside_coalesce_scope() {
        let backend = InMemoryBackend::new();
        let partition = Partition::new("direct");
        backend.create_partition(&partition).unwrap();
        backend.put(&partition, b"k", b"v").unwrap();
        assert_eq!(backend.get(&partition, b"k").unwrap(), Some(b"v".to_vec()));
    }
}
