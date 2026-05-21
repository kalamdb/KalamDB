//! Scenario-based end-to-end tests for KalamDB.
//!
//! These tests validate KalamDB as a SQL-first, real-time database with:
//! - Table-per-user isolation (USER tables)
//! - Shared reference data (SHARED tables)
//! - Ephemeral streams with TTL (STREAM tables)
//! - Hot + cold tiers (RocksDB + Parquet) with flush jobs
//! - Live SQL subscriptions with initial snapshot batching
//! - RBAC, direct subject scoping, and role-matrix EXECUTE AS USER writes
//! - Parallel usage under realistic workloads

pub mod helpers;

// Scenario tests - organized by runnable category
mod lifecycle;
mod realtime;
mod scale;
