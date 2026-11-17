//! Watermark protocol shared by the user and shared data groups.
//!
//! A data command waits until the meta group has applied `required_meta_index`,
//! then the group applies it. Shard ownership stays in the shared group.

use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
};

use kalamdb_commons::ids::VersionId;

use super::{get_coordinator, ApplyResult, PendingBuffer, PendingCommand};
use crate::{codec::command_codec::encode_data_response, DataResponse, RaftError};

/// Applied index, its notifier, and the commands waiting on the meta watermark.
pub struct DataWatermark<'a> {
    pub last_applied_index: &'a AtomicU64,
    pub last_applied_term: &'a AtomicU64,
    pub notify: &'a tokio::sync::watch::Sender<u64>,
    pub pending: &'a PendingBuffer,
}

impl DataWatermark<'_> {
    pub fn mark(&self, index: u64, term: u64) {
        let last_applied = self.last_applied_index.load(Ordering::Acquire);
        if index <= last_applied {
            return;
        }
        self.store(index, term);
    }

    pub fn rewind(&self, index: u64, term: u64) {
        self.store(index, term);
    }

    fn store(&self, index: u64, term: u64) {
        self.last_applied_index.store(index, Ordering::Release);
        self.last_applied_term.store(term, Ordering::Release);
        self.notify.send_replace(index);
    }
}

/// Apply one data command, or buffer its bytes while the meta watermark is behind.
pub async fn apply_data_entry<C, Decode, Meta, Slots, Apply, Fut>(
    label: &'static str,
    shard: u32,
    watermark: DataWatermark<'_>,
    index: u64,
    term: u64,
    command: &[u8],
    decode: Decode,
    required_meta: Meta,
    row_slots: Slots,
    apply_cmd: Apply,
) -> Result<ApplyResult, RaftError>
where
    Decode: Fn(&[u8]) -> Result<C, RaftError>,
    Meta: Fn(&C) -> u64,
    Slots: Fn(&C) -> usize,
    Apply: Fn(C, Vec<VersionId>) -> Fut,
    Fut: Future<Output = Result<DataResponse, RaftError>> + Send,
{
    let last_applied = watermark.last_applied_index.load(Ordering::Acquire);
    if index <= last_applied {
        log::debug!(
            "{label}[{shard}]: Skipping already applied entry {index} (last_applied={last_applied})"
        );
        return Ok(ApplyResult::NoOp);
    }

    let cmd = decode(command)?;
    let required = required_meta(&cmd);
    if required > 0 {
        let current_meta = get_coordinator().current_index();
        if required > current_meta {
            log::debug!(
                "{label}[{shard}]: Buffering command (required_meta={required} > current_meta={current_meta})"
            );
            watermark.pending.add(PendingCommand {
                log_index: index,
                log_term: term,
                required_meta_index: required,
                command_bytes: command.to_vec(),
            });
            watermark.store(index, term);
            return Ok(ApplyResult::NoOp);
        }
    }

    if !watermark.pending.is_empty() {
        drain_pending(label, shard, watermark.pending, &decode, &row_slots, &apply_cmd).await?;
    }

    let slots = row_slots(&cmd);
    let versions = match crate::assign_entry_versions(index, slots) {
        Ok(versions) => versions,
        Err(error) => {
            return Ok(ApplyResult::ok_with_data(encode_data_response(&DataResponse::error(
                error.to_string(),
            ))?));
        },
    };
    let response = apply_cmd(cmd, versions).await?;
    watermark.store(index, term);
    Ok(ApplyResult::ok_with_data(encode_data_response(&response)?))
}

async fn drain_pending<C, Decode, Slots, Apply, Fut>(
    label: &'static str,
    shard: u32,
    pending: &PendingBuffer,
    decode: &Decode,
    row_slots: &Slots,
    apply_cmd: &Apply,
) -> Result<(), RaftError>
where
    Decode: Fn(&[u8]) -> Result<C, RaftError>,
    Slots: Fn(&C) -> usize,
    Apply: Fn(C, Vec<VersionId>) -> Fut,
    Fut: Future<Output = Result<DataResponse, RaftError>> + Send,
{
    let current_meta = get_coordinator().current_index();
    let drained = pending.drain_satisfied(current_meta);
    if !drained.is_empty() {
        log::info!(
            "{label}[{shard}]: Draining {} buffered commands (meta_index={current_meta})",
            drained.len()
        );
    }

    for command in drained {
        let cmd = decode(&command.command_bytes)?;
        let versions = match crate::assign_entry_versions(command.log_index, row_slots(&cmd)) {
            Ok(versions) => versions,
            Err(error) => {
                log::warn!("{label}[{shard}]: rejecting buffered entry: {error}");
                continue;
            },
        };
        let _ = apply_cmd(cmd, versions).await?;
        log::debug!("{label}[{shard}]: Applied buffered command log_index={}", command.log_index);
    }
    Ok(())
}
