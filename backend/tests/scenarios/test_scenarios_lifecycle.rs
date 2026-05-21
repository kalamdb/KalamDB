//! Test driver for lifecycle and platform scenarios.
//!
//! Run with: cargo test --test test_scenarios_lifecycle

#[path = "../common/testserver/mod.rs"]
mod test_support;

#[path = "helpers.rs"]
pub mod helpers;

mod lifecycle;
