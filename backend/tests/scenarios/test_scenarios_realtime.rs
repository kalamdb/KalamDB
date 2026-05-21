//! Test driver for realtime workflow scenarios.
//!
//! Run with: cargo test --test test_scenarios_realtime

#[path = "../common/testserver/mod.rs"]
mod test_support;

#[path = "helpers.rs"]
pub mod helpers;

mod realtime;
