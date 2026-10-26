//! Linked together so CI does not link the full engine once per source file.

#[path = "support/mod.rs"]
mod support;

#[path = "shared_live_and_restart.rs"]
mod shared_live_and_restart;
#[path = "shared_shard_ownership.rs"]
mod shared_shard_ownership;
#[path = "test_flush_operations.rs"]
mod test_flush_operations;
#[path = "test_manifest_cache.rs"]
mod test_manifest_cache;
#[path = "test_manifest_operations.rs"]
mod test_manifest_operations;
