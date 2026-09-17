//! Server-backed e2e tests in one binary so `kalamdb-server` is linked once.
//!
//! Run: `cargo nextest run -p kalamdb-server --features e2e-tests --test e2e`

#[path = "common/testserver/mod.rs"]
mod test_support;

#[path = "testserver/test_testserver.rs"]
mod test_testserver;

#[path = "scenarios/test_scenarios.rs"]
mod test_scenarios;

#[path = "cluster/test_cluster.rs"]
mod test_cluster;

#[path = "endurance_test.rs"]
mod endurance_test;

#[path = "pgwire_catalog/test_client_catalog.rs"]
mod pgwire_catalog;
