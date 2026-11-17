//! OpenRaft Type Configuration
//!
//! This module defines the type configuration for KalamDB's Raft implementation.
//! These types are used throughout the Raft layer for node identification,
//! log entries, and snapshot data.

use kalamdb_observability::SERVER_VERSION;
use openraft::{vote::RaftLeaderId, OptionalSend, RaftTypeConfig};
use serde::{Deserialize, Serialize};

/// Type configuration for KalamDB Raft.
///
/// Leader ids use standard Raft: a committed leader id is the term, so a log
/// id read back from disk compares equal to the one that was appended.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct KalamTypeConfig;

impl RaftTypeConfig for KalamTypeConfig {
    type D = super::command_bytes::RaftCommandBytes;
    type R = Vec<u8>;
    type NodeId = u64;
    type Node = KalamNode;
    type Term = u64;
    type LeaderId = openraft::impls::leader_id_std::LeaderId<u64, u64>;
    type Vote = openraft::impls::Vote<Self::LeaderId>;
    type Membership = openraft::impls::Membership<u64, KalamNode>;
    type MembershipEdit = openraft::impls::ChangeMembers<Self::NodeId, Self::Node>;
    type Payload = openraft::impls::EntryPayload<Self::D, Self::Membership>;
    type Entry = openraft::impls::Entry<<Self::LeaderId as RaftLeaderId>::Committed, Self::Payload>;
    type AsyncRuntime = openraft::TokioRuntime;
    type Responder<T>
        = openraft::impls::OneshotResponder<Self, T>
    where
        T: OptionalSend + 'static;
    type Batch<T>
        = openraft::impls::InlineBatch<T>
    where
        T: OptionalSend + 'static;
    type ErrorSource = openraft::AnyError;
}

/// Committed leader id for standard Raft: the term alone.
pub type KalamCommittedLeaderId = openraft::vote::leader_id_std::CommittedLeaderId<u64>;

/// Log id stored by OpenRaft. The committed leader id is the term.
pub type KalamLogId = openraft::LogId<KalamCommittedLeaderId>;

/// Vote type for [`KalamTypeConfig`].
pub type KalamVote = <KalamTypeConfig as RaftTypeConfig>::Vote;

/// Membership type for [`KalamTypeConfig`].
pub type KalamMembership = <KalamTypeConfig as RaftTypeConfig>::Membership;

/// Membership record carried in the state machine and snapshots.
pub type KalamStoredMembership =
    openraft::StoredMembership<KalamCommittedLeaderId, KalamMembership>;

/// Snapshot metadata carried by OpenRaft. The file id stays on KalamDB's own record.
pub type KalamSnapshotMeta = openraft::SnapshotMeta<KalamCommittedLeaderId, KalamMembership>;

/// Log entry type for [`KalamTypeConfig`].
pub type KalamEntry = <KalamTypeConfig as RaftTypeConfig>::Entry;

/// Log payload type for [`KalamTypeConfig`].
pub type KalamEntryPayload = <KalamTypeConfig as RaftTypeConfig>::Payload;

/// Snapshot bytes shared by build, the current snapshot, and transfer.
pub type KalamSnapshotData = std::io::Cursor<std::sync::Arc<[u8]>>;

/// Snapshot handle passed through OpenRaft.
pub type KalamSnapshot =
    openraft::Snapshot<KalamCommittedLeaderId, KalamMembership, KalamSnapshotData>;

/// Node information for cluster membership
///
/// Contains the network addresses and system metadata for a node:
/// - `rpc_addr`: gRPC address for Raft consensus messages
/// - `api_addr`: HTTP address for client requests
/// - `hostname`: Machine hostname (optional, for display purposes)
/// - `version`: KalamDB version (optional)
/// - `memory_mb`: Total system memory in MB (optional)
/// - `os`: Operating system (optional)
/// - `arch`: CPU architecture (optional)
///
/// **Important**: Do NOT use `skip_serializing_if` on any fields!
/// Bincode is a non-self-describing format that requires all fields to be present.
/// Using `skip_serializing_if` causes deserialization to fail with `UnexpectedEnd`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KalamNode {
    /// gRPC address for Raft communication (e.g., "127.0.0.1:2910")
    pub rpc_addr: String,
    /// HTTP address for client requests (e.g., "127.0.0.1:2900")
    pub api_addr: String,
    /// Machine hostname (e.g., "node-1.kalamdb.local")
    #[serde(default)]
    pub hostname: Option<String>,
    /// KalamDB version (e.g., "0.1.0")
    #[serde(default)]
    pub version: Option<String>,
    /// Total system memory in megabytes
    #[serde(default)]
    pub memory_mb: Option<u64>,
    /// Operating system (e.g., "linux", "macos", "windows")
    #[serde(default)]
    pub os: Option<String>,
    /// CPU architecture (e.g., "x86_64", "aarch64")
    #[serde(default)]
    pub arch: Option<String>,
}

impl KalamNode {
    /// Create a new KalamNode with the given addresses
    pub fn new(rpc_addr: impl Into<String>, api_addr: impl Into<String>) -> Self {
        Self {
            rpc_addr: rpc_addr.into(),
            api_addr: api_addr.into(),
            hostname: None,
            version: None,
            memory_mb: None,
            os: None,
            arch: None,
        }
    }

    /// Create a new KalamNode with full system metadata
    pub fn with_metadata(
        rpc_addr: impl Into<String>,
        api_addr: impl Into<String>,
        hostname: Option<String>,
        version: Option<String>,
        memory_mb: Option<u64>,
        os: Option<String>,
        arch: Option<String>,
    ) -> Self {
        Self {
            rpc_addr: rpc_addr.into(),
            api_addr: api_addr.into(),
            hostname,
            version,
            memory_mb,
            os,
            arch,
        }
    }

    /// Create a new KalamNode with auto-detected system metadata
    pub fn with_auto_metadata(rpc_addr: impl Into<String>, api_addr: impl Into<String>) -> Self {
        Self {
            rpc_addr: rpc_addr.into(),
            api_addr: api_addr.into(),
            hostname: Self::detect_hostname(),
            version: Some(SERVER_VERSION.to_string()),
            memory_mb: Self::detect_memory_mb(),
            os: Some(std::env::consts::OS.to_string()),
            arch: Some(std::env::consts::ARCH.to_string()),
        }
    }

    /// Detect the system hostname
    fn detect_hostname() -> Option<String> {
        hostname::get().ok().and_then(|h| h.into_string().ok())
    }

    /// Detect system memory in MB
    fn detect_memory_mb() -> Option<u64> {
        #[cfg(target_os = "linux")]
        {
            use std::fs;
            fs::read_to_string("/proc/meminfo").ok().and_then(|content| {
                content.lines().find(|line| line.starts_with("MemTotal:")).and_then(|line| {
                    line.split_whitespace()
                        .nth(1)
                        .and_then(|kb| kb.parse::<u64>().ok())
                        .map(|kb| kb / 1024) // Convert KB to MB
                })
            })
        }
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()
                .and_then(|output| {
                    String::from_utf8(output.stdout)
                        .ok()
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .map(|bytes| bytes / (1024 * 1024)) // Convert bytes to MB
                })
        }
        #[cfg(target_os = "windows")]
        {
            // Windows: use GlobalMemoryStatusEx via winapi or fallback
            None // TODO: Implement Windows memory detection
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            None
        }
    }
}

impl std::fmt::Display for KalamNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref hostname) = self.hostname {
            write!(f, "{}|{}|{}", self.rpc_addr, self.api_addr, hostname)
        } else {
            write!(f, "{}|{}", self.rpc_addr, self.api_addr)
        }
    }
}

impl std::error::Error for KalamNode {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kalam_node_display() {
        let node = KalamNode::new("127.0.0.1:2910", "127.0.0.1:2900");
        assert_eq!(format!("{}", node), "127.0.0.1:2910|127.0.0.1:2900");
    }

    #[test]
    fn test_kalam_node_display_with_hostname() {
        let node = KalamNode::with_metadata(
            "127.0.0.1:2910",
            "127.0.0.1:2900",
            Some("node-1".to_string()),
            Some("0.1.0".to_string()),
            Some(16384),
            Some("linux".to_string()),
            Some("x86_64".to_string()),
        );
        assert_eq!(format!("{}", node), "127.0.0.1:2910|127.0.0.1:2900|node-1");
    }

    #[test]
    fn test_kalam_node_default() {
        let node = KalamNode::default();
        assert_eq!(node.rpc_addr, "");
        assert_eq!(node.api_addr, "");
        assert!(node.hostname.is_none());
        assert!(node.version.is_none());
    }

    #[test]
    fn test_kalam_node_with_auto_metadata() {
        let node = KalamNode::with_auto_metadata("127.0.0.1:2910", "127.0.0.1:2900");
        assert_eq!(node.rpc_addr, "127.0.0.1:2910");
        assert_eq!(node.api_addr, "127.0.0.1:2900");
        // These should be auto-detected
        assert!(node.version.is_some());
        assert!(node.os.is_some());
        assert!(node.arch.is_some());
    }
}
