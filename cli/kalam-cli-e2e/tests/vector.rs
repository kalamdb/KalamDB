// Aggregator for vector and embedding tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test vector
//
// Run individual test files:
//   cargo test --test vector --features cloud-aws

#[cfg(feature = "cloud-aws")]
#[path = "storage/minio/common.rs"]
pub(crate) mod minio_common;

#[cfg(feature = "cloud-aws")]
#[path = "vector/helpers.rs"]
mod helpers;

#[cfg(feature = "cloud-aws")]
#[path = "vector/embedding_flush.rs"]
mod embedding_flush;

#[cfg(feature = "cloud-aws")]
#[path = "vector/vector_index_manifest_snapshot.rs"]
mod vector_index_manifest_snapshot;
