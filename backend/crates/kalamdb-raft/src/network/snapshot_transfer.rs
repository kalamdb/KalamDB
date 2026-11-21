//! One snapshot transfer: split a payload into bounded pieces, reassemble them,
//! and hand back a single body. The network only sends the pieces.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

use crate::storage::{KalamLogId, KalamStoredMembership, KalamVote};

/// One complete snapshot after its chunks have been reassembled.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FullSnapshotMessage {
    pub vote: KalamVote,
    pub last_log_id: Option<KalamLogId>,
    pub last_membership: KalamStoredMembership,
    pub data: Vec<u8>,
}

/// One bounded piece of a snapshot. The receiver installs only after every piece arrives.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotChunk {
    pub vote: KalamVote,
    pub last_log_id: Option<KalamLogId>,
    pub last_membership: KalamStoredMembership,
    pub transfer_id: u64,
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub data: Vec<u8>,
}

const SNAPSHOT_CHUNK_BYTES: usize = 1024 * 1024;

pub fn snapshot_chunk_count(len: usize) -> usize {
    if len == 0 {
        1
    } else {
        len.div_ceil(SNAPSHOT_CHUNK_BYTES)
    }
}

pub fn snapshot_chunk(data: &[u8], index: usize) -> &[u8] {
    if data.is_empty() {
        return &[];
    }
    let start = index.saturating_mul(SNAPSHOT_CHUNK_BYTES);
    let end = start.saturating_add(SNAPSHOT_CHUNK_BYTES).min(data.len());
    if start >= data.len() {
        &[]
    } else {
        &data[start..end]
    }
}

/// Materialize every chunk. Transfer sends [`snapshot_chunk`] from the stored
/// buffer instead, so a snapshot is not copied twice before the first byte leaves.
pub fn snapshot_payload_chunks(data: &[u8]) -> Vec<Vec<u8>> {
    let count = snapshot_chunk_count(data.len());
    let mut chunks = Vec::with_capacity(count);
    for index in 0..count {
        chunks.push(snapshot_chunk(data, index).to_vec());
    }
    chunks
}

struct PartialSnapshot {
    vote: KalamVote,
    last_log_id: Option<KalamLogId>,
    last_membership: KalamStoredMembership,
    parts: Vec<Option<Vec<u8>>>,
}

fn snapshot_parts() -> &'static Mutex<HashMap<(String, u64), PartialSnapshot>> {
    static PARTS: OnceLock<Mutex<HashMap<(String, u64), PartialSnapshot>>> = OnceLock::new();
    PARTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn next_snapshot_transfer_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Accept one snapshot chunk. `Ok(None)` means more chunks are required.
pub fn assemble_snapshot_chunk(
    group_id: &str,
    chunk: SnapshotChunk,
) -> Result<Option<FullSnapshotMessage>, String> {
    if chunk.chunk_count == 0 || chunk.chunk_index >= chunk.chunk_count {
        return Err(format!("invalid snapshot chunk {}/{}", chunk.chunk_index, chunk.chunk_count));
    }
    if chunk.chunk_count == 1 {
        return Ok(Some(FullSnapshotMessage {
            vote: chunk.vote,
            last_log_id: chunk.last_log_id,
            last_membership: chunk.last_membership,
            data: chunk.data,
        }));
    }

    let key = (group_id.to_string(), chunk.transfer_id);
    let mut guard = snapshot_parts().lock().unwrap_or_else(|err| err.into_inner());
    if guard.len() > 32 {
        guard.clear();
    }
    let partial = guard.entry(key.clone()).or_insert_with(|| PartialSnapshot {
        vote: chunk.vote.clone(),
        last_log_id: chunk.last_log_id,
        last_membership: chunk.last_membership.clone(),
        parts: vec![None; chunk.chunk_count as usize],
    });
    if partial.parts.len() != chunk.chunk_count as usize {
        return Err("snapshot chunk count changed".to_string());
    }
    partial.parts[chunk.chunk_index as usize] = Some(chunk.data);
    if partial.parts.iter().any(|part| part.is_none()) {
        return Ok(None);
    }
    let finished = guard.remove(&key).ok_or_else(|| "snapshot assembly lost".to_string())?;
    let mut data = Vec::new();
    for part in finished.parts.into_iter().flatten() {
        data.extend(part);
    }
    Ok(Some(FullSnapshotMessage {
        vote: finished.vote,
        last_log_id: finished.last_log_id,
        last_membership: finished.last_membership,
        data,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_chunks_reassemble_before_install() {
        let membership = KalamStoredMembership::default();
        let vote = KalamVote::new(1, 1);
        let transfer_id = next_snapshot_transfer_id();
        let pending = assemble_snapshot_chunk(
            "meta",
            SnapshotChunk {
                vote: vote.clone(),
                last_log_id: None,
                last_membership: membership.clone(),
                transfer_id,
                chunk_index: 0,
                chunk_count: 2,
                data: b"hello".to_vec(),
            },
        )
        .unwrap();
        assert!(pending.is_none());

        let complete = assemble_snapshot_chunk(
            "meta",
            SnapshotChunk {
                vote,
                last_log_id: None,
                last_membership: membership,
                transfer_id,
                chunk_index: 1,
                chunk_count: 2,
                data: b" world".to_vec(),
            },
        )
        .unwrap()
        .expect("second chunk completes the snapshot");
        assert_eq!(complete.data, b"hello world");
    }
}
