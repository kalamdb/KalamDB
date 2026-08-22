//! Raft Storage Layer
//!
//! This module provides the storage implementation for openraft using native
//! `RaftLogStorage` + `RaftStateMachine` (storage-v2). Log IO and state-machine
//! apply can run concurrently; they are not serialized behind OpenRaft's Adaptor lock.
//!
//! ## Components
//!
//! - [`KalamRaftStorage`]: Combined storage implementing OpenRaft v2 log + state-machine traits
//! - [`KalamTypeConfig`]: OpenRaft type configuration for KalamDB
//! - [`KalamNode`]: Node information for cluster membership

mod raft_store;
mod types;

pub use raft_store::{KalamLogReader, KalamRaftStorage, KalamSnapshotBuilder, StoredSnapshot};
pub use types::{KalamNode, KalamTypeConfig};
