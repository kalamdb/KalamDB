//! Raft Storage Layer
//!
//! This module provides the storage implementation for openraft using native
//! `RaftLogStorage` + `RaftStateMachine`. Log IO and state-machine
//! apply can run concurrently; they are not serialized behind OpenRaft's Adaptor lock.
//!
//! ## Components
//!
//! - [`KalamRaftStorage`]: Combined storage implementing OpenRaft v2 log + state-machine traits
//! - [`KalamTypeConfig`]: OpenRaft type configuration for KalamDB
//! - [`KalamNode`]: Node information for cluster membership

mod command_bytes;
mod log_io;
mod raft_store;
mod types;

pub use command_bytes::RaftCommandBytes;
pub use raft_store::{
    decode_persisted_membership, decode_persisted_state, KalamLogReader, KalamRaftStorage,
    KalamSnapshotBuilder, PersistedRaftState, StoredSnapshot,
};
pub use types::{
    KalamCommittedLeaderId, KalamEntry, KalamEntryPayload, KalamLogId, KalamMembership, KalamNode,
    KalamSnapshot, KalamSnapshotData, KalamSnapshotMeta, KalamStoredMembership, KalamTypeConfig,
    KalamVote,
};
