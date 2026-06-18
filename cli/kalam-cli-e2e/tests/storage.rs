// Aggregator for storage tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test storage
//
// Run individual test files:
//   cargo test --test storage test_hot_cold_storage
//   cargo test --test storage test_storage_lifecycle
//   cargo test --test storage test_minio_storage

mod common;

#[path = "storage/test_hot_cold_storage.rs"]
mod test_hot_cold_storage;

#[path = "storage/test_storage_lifecycle.rs"]
mod test_storage_lifecycle;

#[path = "storage/minio/mod.rs"]
mod test_minio_storage;
