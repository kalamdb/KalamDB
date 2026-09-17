//! Test driver for cluster integration tests.
//!
//! Run with: cargo nextest run -p kalamdb-server --features e2e-tests --test e2e

// Include cluster test modules
mod test_cluster_basic_operations;
mod test_cluster_node_recovery_and_sync;
