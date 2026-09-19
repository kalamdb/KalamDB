//! Dedicated thread for Raft log `append()` IO.
//!
//! OpenRaft 0.9 [`RaftLogStorage::append`] must return immediately and later
//! fire [`LogFlushed`]. Completing the RocksDB write on this thread lets the
//! Raft core task yield on the flush oneshot and overlap with the previous
//! apply. Consecutive appends still wait for `LogFlushed` (0.9 core limitation).

use std::{
    io,
    sync::{mpsc, OnceLock},
};

use kalamdb_store::RaftPartitionStore;
use openraft::storage::LogFlushed;

use crate::storage::types::KalamTypeConfig;

enum LogIoJob {
    Append {
        store:    RaftPartitionStore,
        records:  Vec<(u64, Vec<u8>)>,
        callback: LogFlushed<KalamTypeConfig>,
    },
}

fn log_io_sender() -> &'static mpsc::Sender<LogIoJob> {
    static SENDER: OnceLock<mpsc::Sender<LogIoJob>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("kalamdb-raft-log-io".to_string())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    run_log_io_job(job);
                }
            })
            .expect("failed to spawn kalamdb-raft-log-io thread");
        tx
    })
}

fn run_log_io_job(job: LogIoJob) {
    match job {
        LogIoJob::Append {
            store,
            records,
            callback,
        } => {
            let result = store
                .append_encoded(records)
                .map_err(|e| io::Error::other(e.to_string()));
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

    let job = LogIoJob::Append {
        store: store.clone(),
        records,
        callback,
    };
    if let Err(mpsc::SendError(job)) = log_io_sender().send(job) {
        run_log_io_job(job);
    }
}
