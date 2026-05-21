//! Test driver for scenario-based end-to-end tests.
//!
//! Run with: cargo test --test test_scenarios

// Include the common test support
#[path = "../common/testserver/mod.rs"]
mod test_support;

// Include all scenario categories
pub mod helpers;

mod lifecycle;
mod realtime;
mod scale;
