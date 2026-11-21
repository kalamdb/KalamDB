//! Sharded workers for Raft log `append()` IO.
//!
//! [`RaftLogStorage::append`] returns before the flush callback. Completing
//! the write on a worker lets the Raft core yield on [`IOFlushed`] and overlap
//! with the previous apply.
//!
//! Each group always uses the same worker, so that group's appends stay ordered.
//! A full shard queue is bounded. The caller uses `try_send` first. Only a full
//! queue waits, and on a Tokio worker that wait is `block_in_place` so the
//! runtime can move other tasks off this thread.

use std::{
    io,
    sync::{mpsc, OnceLock},
};

use kalamdb_store::RaftPartitionStore;
use openraft::storage::IOFlushed;

use crate::{storage::types::KalamTypeConfig, GroupId};

const LOG_IO_SHARDS: usize = 4;
const LOG_IO_QUEUE_DEPTH: usize = 64;

enum LogIoJob {
    Append {
        store: RaftPartitionStore,
        records: Vec<(u64, std::sync::Arc<[u8]>)>,
        callback: IOFlushed<KalamTypeConfig>,
    },
}

fn log_io_senders() -> &'static [mpsc::SyncSender<LogIoJob>; LOG_IO_SHARDS] {
    static SENDERS: OnceLock<[mpsc::SyncSender<LogIoJob>; LOG_IO_SHARDS]> = OnceLock::new();
    SENDERS.get_or_init(|| {
        std::array::from_fn(|shard| {
            let (sender, receiver) = mpsc::sync_channel(LOG_IO_QUEUE_DEPTH);
            std::thread::Builder::new()
                .name(format!("kalamdb-raft-log-io-{shard}"))
                .spawn(move || {
                    while let Ok(job) = receiver.recv() {
                        run_log_io_job(job);
                    }
                })
                .expect("failed to spawn kalamdb-raft-log-io thread");
            sender
        })
    })
}

fn shard_for_group(group_id: GroupId) -> usize {
    let mixed = group_id.as_u64().wrapping_mul(0x9E37_79B1);
    (mixed as usize) % LOG_IO_SHARDS
}

fn run_log_io_job(job: LogIoJob) {
    match job {
        LogIoJob::Append {
            store,
            records,
            callback,
        } => {
            let records =
                records.into_iter().map(|(index, record)| (index, record.to_vec())).collect();
            let result = store.append_encoded(records).map_err(|e| io::Error::other(e.to_string()));
            callback.io_completed(result);
        },
    }
}

/// Persist packed log records off the Raft core task, then complete `callback`.
pub(super) fn flush_log_records(
    store: &RaftPartitionStore,
    records: Vec<(u64, std::sync::Arc<[u8]>)>,
    callback: IOFlushed<KalamTypeConfig>,
) {
    if records.is_empty() {
        callback.io_completed(Ok(()));
        return;
    }

    let shard = shard_for_group(store.group_id());
    let job = LogIoJob::Append {
        store: store.clone(),
        records,
        callback,
    };
    enqueue(&log_io_senders()[shard], job);
}

/// Queue `job` without blocking a Tokio worker.
///
/// `try_send` returns immediately. A full queue waits on `block_in_place`
/// when a runtime is present, so other tasks can move to another worker.
/// The send completes before this function returns, which keeps a group's
/// appends in order on its shard. A disconnected worker runs the job inline.
fn enqueue(sender: &mpsc::SyncSender<LogIoJob>, job: LogIoJob) {
    match sender.try_send(job) {
        Ok(()) => {},
        Err(mpsc::TrySendError::Disconnected(job)) => run_log_io_job(job),
        Err(mpsc::TrySendError::Full(job)) => {
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::task::block_in_place(|| send_or_run(sender, job));
            } else {
                send_or_run(sender, job);
            }
        },
    }
}

fn send_or_run(sender: &mpsc::SyncSender<LogIoJob>, job: LogIoJob) {
    if let Err(mpsc::SendError(job)) = sender.send(job) {
        run_log_io_job(job);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use kalamdb_store::{test_utils::InMemoryBackend, RaftPartitionStore};
    use openraft::storage::IOFlushed;

    use super::*;

    fn append_job(store: &RaftPartitionStore, index: u64) -> LogIoJob {
        LogIoJob::Append {
            store: store.clone(),
            records: vec![(index, Arc::from([index as u8].as_slice()))],
            callback: IOFlushed::noop(),
        }
    }

    fn append_index(job: LogIoJob) -> u64 {
        match job {
            LogIoJob::Append { records, .. } => records[0].0,
        }
    }

    #[test]
    fn four_raft_groups_do_not_share_one_log_io_shard() {
        let shards = [
            shard_for_group(GroupId::Meta),
            shard_for_group(GroupId::DataUserShard(0)),
            shard_for_group(GroupId::DataSharedShard(0)),
            shard_for_group(GroupId::DataSharedShard(1)),
        ];
        let mut unique = shards.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert!(
            unique.len() > 1,
            "expected the four groups to cover more than one log worker, got {shards:?}"
        );
        assert_eq!(
            shard_for_group(GroupId::DataSharedShard(1)),
            shard_for_group(GroupId::DataSharedShard(1))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn full_log_io_queue_lets_another_task_run() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let store = RaftPartitionStore::new(Arc::new(InMemoryBackend::new()), GroupId::Meta);
        sender.try_send(append_job(&store, 1)).expect("prime the queue");

        let queued = tokio::spawn(async move {
            enqueue(&sender, append_job(&store, 2));
        });
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        tokio::spawn(async move {
            flag.store(true, Ordering::SeqCst);
        })
        .await
        .expect("companion task");
        assert!(ran.load(Ordering::SeqCst), "a full log queue stalled the runtime");

        assert_eq!(append_index(receiver.recv().expect("first")), 1);
        assert_eq!(
            append_index(receiver.recv_timeout(std::time::Duration::from_secs(1)).expect("second")),
            2
        );
        queued.await.expect("enqueue");
    }
}
