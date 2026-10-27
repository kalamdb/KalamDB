//! Sharded workers for Raft log `append()` IO.
//!
//! OpenRaft 0.9 [`RaftLogStorage::append`] must return immediately and later
//! fire [`LogFlushed`]. Completing the RocksDB write on a worker lets the
//! Raft core task yield on the flush oneshot and overlap with the previous
//! apply. Consecutive appends still wait for `LogFlushed` (0.9 core limitation).
//!
//! Each group always uses the same worker, so that group's appends stay ordered.
//! A full shard queue blocks that caller instead of growing without a bound.

use std::{
    io,
    sync::{mpsc, OnceLock},
};

use kalamdb_store::RaftPartitionStore;
use openraft::storage::LogFlushed;

use crate::{storage::types::KalamTypeConfig, GroupId};

const LOG_IO_SHARDS: usize = 4;
const LOG_IO_QUEUE_DEPTH: usize = 64;

enum LogIoJob {
    Append {
        store: RaftPartitionStore,
        records: Vec<(u64, Vec<u8>)>,
        callback: LogFlushed<KalamTypeConfig>,
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
            let result = store.append_encoded(records).map_err(|e| io::Error::other(e.to_string()));
            callback.log_io_completed(result);
        },
    }
}

/// Persist packed log records off the Raft core task, then complete `callback`.
pub(super) fn flush_log_records(
    store: &RaftPartitionStore,
    records: Vec<(u64, Vec<u8>)>,
    callback: LogFlushed<KalamTypeConfig>,
) {
    if records.is_empty() {
        callback.log_io_completed(Ok(()));
        return;
    }

    let shard = shard_for_group(store.group_id());
    let job = LogIoJob::Append {
        store: store.clone(),
        records,
        callback,
    };
    if let Err(mpsc::SendError(job)) = log_io_senders()[shard].send(job) {
        run_log_io_job(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
