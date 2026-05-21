//! Scale and resilience scenarios.
//!
//! These scenarios emphasize wide-row ingestion, burst traffic, performance
//! baselines, and longer-running soak-style workloads.

pub(super) mod helpers {
    pub use crate::helpers::*;
}

#[path = "../scenario_04_iot_telemetry.rs"]
mod scenario_04_iot_telemetry;
#[path = "../scenario_08_burst.rs"]
mod scenario_08_burst;
#[path = "../scenario_12_performance.rs"]
mod scenario_12_performance;
#[path = "../scenario_13_soak_test.rs"]
mod scenario_13_soak_test;
