//! Lifecycle and platform scenarios.
//!
//! These scenarios focus on RBAC, jobs, schema evolution, namespace isolation,
//! storage routing, and advanced document workflows.

pub(super) mod helpers {
    pub use crate::helpers::*;
}

#[path = "../scenario_05_dashboards.rs"]
mod scenario_05_dashboards;
#[path = "../scenario_06_jobs.rs"]
mod scenario_06_jobs;
#[path = "../scenario_09_ddl_while_active.rs"]
mod scenario_09_ddl_while_active;
#[path = "../scenario_10_multi_tenant.rs"]
mod scenario_10_multi_tenant;
#[path = "../scenario_11_multi_storage.rs"]
mod scenario_11_multi_storage;
#[path = "../scenario_14_vector_rag.rs"]
mod scenario_14_vector_rag;
#[path = "../scenario_15_v07_storage_indexes.rs"]
mod scenario_15_v07_storage_indexes;
