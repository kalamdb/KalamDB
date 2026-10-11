use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use kalamdb_commons::models::{TableId, TransactionId, TransactionOrigin, TransactionState};

use crate::{commit_sequence::SNAPSHOT_UNSET, ExecutionOwnerKey, TransactionRaftBinding};

/// Hot transaction metadata kept separate from the staged write buffer.
#[derive(Debug, Clone)]
pub struct TransactionHandle {
    pub transaction_id:      TransactionId,
    pub owner_key:           ExecutionOwnerKey,
    pub owner_id:            Arc<str>,
    pub origin:              TransactionOrigin,
    pub state:               TransactionState,
    pub raft_binding:        TransactionRaftBinding,
    /// Pinned group frontier (`SNAPSHOT_UNSET` until first data access).
    pub snapshot_log_index:  Arc<AtomicU64>,
    pub started_at:          Instant,
    pub last_activity_at:    Instant,
    pub write_count:         usize,
    pub write_bytes:         usize,
    pub touched_tables:      HashSet<TableId>,
    pub has_write_set:       bool,
}

impl TransactionHandle {
    pub fn new(
        transaction_id: TransactionId,
        owner_key: ExecutionOwnerKey,
        owner_id: Arc<str>,
        origin: TransactionOrigin,
        raft_binding: TransactionRaftBinding,
        now: Instant,
    ) -> Self {
        Self {
            transaction_id,
            owner_key,
            owner_id,
            origin,
            state: TransactionState::OpenRead,
            raft_binding,
            snapshot_log_index: Arc::new(AtomicU64::new(SNAPSHOT_UNSET)),
            started_at: now,
            last_activity_at: now,
            write_count: 0,
            write_bytes: 0,
            touched_tables: HashSet::new(),
            has_write_set: false,
        }
    }

    /// Snapshot frontier pinned at first data access, or `None` if still unset.
    #[inline]
    pub fn snapshot_commit_seq(&self) -> Option<u64> {
        let value = self.snapshot_log_index.load(Ordering::Acquire);
        if value == SNAPSHOT_UNSET {
            None
        } else {
            Some(value)
        }
    }

    #[inline]
    pub fn touch(&mut self) {
        self.last_activity_at = Instant::now();
    }

    pub fn record_staged_write(
        &mut self,
        table_id: TableId,
        write_count: usize,
        write_bytes: usize,
    ) {
        self.last_activity_at = Instant::now();
        self.state = TransactionState::OpenWrite;
        self.write_count = write_count;
        self.write_bytes = write_bytes;
        self.has_write_set = true;
        self.touched_tables.insert(table_id);
    }

    pub fn record_staged_write_batch<I>(
        &mut self,
        table_ids: I,
        write_count: usize,
        write_bytes: usize,
    ) where
        I: IntoIterator<Item = TableId>,
    {
        self.last_activity_at = Instant::now();
        self.state = TransactionState::OpenWrite;
        self.write_count = write_count;
        self.write_bytes = write_bytes;
        self.has_write_set = true;
        for table_id in table_ids {
            self.touched_tables.insert(table_id);
        }
    }

    pub fn mark_state(&mut self, state: TransactionState) {
        self.state = state;
        self.last_activity_at = Instant::now();
    }
}
