// Aggregator for table tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test tables
//
// Run individual test files:
//   cargo test --test tables test_user_tables
//   cargo test --test tables test_shared_tables

mod common;

#[path = "tables/test_user_tables.rs"]
mod test_user_tables;

#[path = "tables/test_shared_tables.rs"]
mod test_shared_tables;
