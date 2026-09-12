//! Server-backed kalam-client tests in one binary so `kalamdb-server` is linked once.
//!
//! Run: `cargo nextest run -p kalam-client-e2e --test e2e`

mod common;

#[path = "connection_lifecycle.rs"]
mod connection_lifecycle;
#[path = "consumer_api.rs"]
mod consumer_api;
#[path = "healthcheck_api.rs"]
mod healthcheck_api;
#[path = "integration_tests.rs"]
mod integration_tests;
#[path = "live_rows.rs"]
mod live_rows;
#[path = "proxied.rs"]
mod proxied;
#[path = "query_api.rs"]
mod query_api;
#[path = "test_consumer.rs"]
mod test_consumer;
#[path = "test_files.rs"]
mod test_files;
#[path = "test_shared_connection.rs"]
mod test_shared_connection;
#[path = "test_subscription_cleanup.rs"]
mod test_subscription_cleanup;
#[path = "test_user_table_subscriptions.rs"]
mod test_user_table_subscriptions;
#[path = "test_websocket_integration.rs"]
mod test_websocket_integration;
