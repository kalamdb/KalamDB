// Aggregator for vector and embedding tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test vector
//
// Run individual test files:
//   cargo test --test vector test_minio_embedding_flush_multiple_common_dimensions
//   cargo test --test vector test_minio_vector_index_manifest_snapshot_exists

mod common;

#[path = "storage/minio/common.rs"]
mod minio_common;

#[path = "vector/helpers.rs"]
mod helpers;

#[path = "vector/embedding_flush.rs"]
mod embedding_flush;

#[path = "vector/vector_index_manifest_snapshot.rs"]
mod vector_index_manifest_snapshot;
