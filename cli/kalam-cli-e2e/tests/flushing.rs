// Aggregator for flushing tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test flushing
//
// Run individual test files:
//   cargo test --test flushing test_flush

mod common;

#[path = "flushing/test_flush.rs"]
mod test_flush;
