// Aggregator for user tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test users
//
// Run individual test files:
//   cargo test --test users test_admin
//   cargo test --test users test_concurrent_users

mod common;

#[path = "users/test_admin.rs"]
mod test_admin;

#[path = "users/test_concurrent_users.rs"]
mod test_concurrent_users;

#[path = "users/test_sql_file_corpus.rs"]
mod test_sql_file_corpus;
