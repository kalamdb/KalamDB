//! Group durable WAL flushes across concurrent writers.
//!
//! Writes use `sync=false`. Waiters share one `flush_wal(true)` so fsync cost
//! is amortized. A ticket is assigned after the write returns so a waiter is
//! never marked durable by a flush that started before its WAL append.

use std::sync::{Condvar, Mutex};

use rocksdb::DB;

use crate::storage_trait::{Result, StorageError};

struct WalGroupState {
    next_ticket:      u64,
    durable_ticket:   u64,
    flushing_through: Option<u64>,
}

pub(super) struct WalGroupCommit {
    state: Mutex<WalGroupState>,
    cv:    Condvar,
}

impl WalGroupCommit {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(WalGroupState {
                next_ticket:      0,
                durable_ticket:   0,
                flushing_through: None,
            }),
            cv:    Condvar::new(),
        }
    }

    pub(super) fn wait_durable(&self, db: &DB) -> Result<()> {
        let ticket = {
            let mut state = lock_state(&self.state)?;
            state.next_ticket = state.next_ticket.saturating_add(1);
            state.next_ticket
        };

        loop {
            let mut state = lock_state(&self.state)?;
            if state.durable_ticket >= ticket {
                return Ok(());
            }
            if state.flushing_through.is_some() {
                drop(wait_group(&self.cv, state)?);
                continue;
            }

            let through = state.next_ticket;
            state.flushing_through = Some(through);
            drop(state);

            let flush_result =
                db.flush_wal(true).map_err(|err| StorageError::IoError(err.to_string()));

            let mut state = lock_state(&self.state)?;
            state.flushing_through = None;
            if flush_result.is_ok() {
                state.durable_ticket = state.durable_ticket.max(through);
            }
            self.cv.notify_all();
            drop(state);
            flush_result?;
        }
    }
}

fn lock_state(state: &Mutex<WalGroupState>) -> Result<std::sync::MutexGuard<'_, WalGroupState>> {
    state
        .lock()
        .map_err(|_| StorageError::Other("wal group lock poisoned".to_string()))
}

fn wait_group<'a>(
    cv: &Condvar,
    guard: std::sync::MutexGuard<'a, WalGroupState>,
) -> Result<std::sync::MutexGuard<'a, WalGroupState>> {
    cv.wait(guard)
        .map_err(|_| StorageError::Other("wal group lock poisoned".to_string()))
}
