//! Server-backed kalam-cli tests in one binary so `kalam-cli` is linked once.
//!
//! Run: `cargo nextest run -p kalam-cli-e2e --test e2e`

mod common;

#[path = "auth.rs"]
mod auth;
#[path = "auth_retry_test.rs"]
mod auth_retry_test;
#[path = "cli.rs"]
mod cli;
#[path = "cluster.rs"]
mod cluster;
#[path = "connection.rs"]
mod connection;
#[path = "flushing.rs"]
mod flushing;
#[path = "performance.rs"]
mod performance;
#[path = "repro_issue.rs"]
mod repro_issue;
#[path = "smoke.rs"]
mod smoke;
#[path = "storage.rs"]
mod storage;
#[path = "subscription.rs"]
mod subscription;
#[path = "tables.rs"]
mod tables;
#[path = "usecases.rs"]
mod usecases;
#[path = "users.rs"]
mod users;
#[cfg(feature = "cloud-aws")]
#[path = "vector.rs"]
mod vector;

pub(crate) use cli::test_project_workflow_dev;
pub(crate) use cluster::cluster_common;
pub(crate) use smoke::{kobj_helpers, topic_test_support};
#[cfg(feature = "cloud-aws")]
pub(crate) use vector::minio_common;
