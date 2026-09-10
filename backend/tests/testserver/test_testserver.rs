//! Test driver for testserver integration tests.
//!
//! Run with: cargo nextest run -p kalamdb-server --features e2e-tests --test e2e

pub(super) use crate::test_support;

// Include test modules organized by category
mod cluster;
mod files;
mod flush;
mod manifest;
mod observability;
mod security;
mod sql;
mod storage;
mod stress;
mod subscription;
mod system;
mod tables;

// Include standalone smoke test
mod test_http_test_server_smoke;
