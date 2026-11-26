//! Raft Manager
//!
//! This module provides the central orchestration for all Raft groups.
//!
//! ## Components
//!
//! - [`RaftManager`]: Manages all Raft group instances
//! - [`RaftGroup`]: A single Raft consensus group
//! - [`RaftManagerConfig`]: Configuration for the Raft manager
//! - [`PeerNode`]: Configuration for a peer node
//! - [`SnapshotInfo`]: Information about a snapshot operation
//! - [`SnapshotsSummary`]: Summary of all snapshots

mod config;
mod raft_group;
mod raft_manager;

pub use config::{
    PeerNode, RaftManagerConfig, RpcTlsConfig, DEFAULT_SHARED_DATA_SHARDS, DEFAULT_USER_DATA_SHARDS,
};
pub use raft_group::RaftGroup;
pub use raft_manager::{GroupProgressSnapshot, RaftManager, SnapshotInfo, SnapshotsSummary};
