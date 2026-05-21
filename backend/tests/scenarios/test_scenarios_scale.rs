//! Test driver for scale and resilience scenarios.
//!
//! Run with: cargo test --test test_scenarios_scale

#[path = "../common/testserver/mod.rs"]
mod test_support;

#[path = "helpers.rs"]
pub mod helpers;

mod scale;
