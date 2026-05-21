//! Test driver for cluster integration tests.
//!
//! Run with: cargo test --test test_cluster

// Include the common test support
#[path = "../common/testserver/mod.rs"]
mod test_support;

// Include cluster test modules
mod test_cluster_basic_operations;
mod test_cluster_node_recovery_and_sync;
