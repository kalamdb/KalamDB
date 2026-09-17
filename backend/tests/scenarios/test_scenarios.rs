//! Test driver for scenario-based end-to-end tests.
//!
//! Run with: cargo nextest run -p kalamdb-server --features e2e-tests --test e2e

// Include all scenario categories
pub mod helpers;

mod lifecycle;
mod realtime;
mod scale;
