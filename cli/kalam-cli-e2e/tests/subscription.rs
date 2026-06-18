// Aggregator for subscription tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test subscription
//
// Run individual test files:
//   cargo test --test subscription test_subscribe
//   cargo test --test subscription test_subscription_e2e
//   cargo test --test subscription test_subscription_manual
//   cargo test --test subscription test_link_subscription_initial_data
//   cargo test --test subscription subscription_options
//   cargo test --test subscription live_connection

mod common;

#[path = "subscription/test_subscribe.rs"]
mod test_subscribe;

#[path = "subscription/test_subscription_e2e.rs"]
mod test_subscription_e2e;

#[path = "subscription/test_subscription_manual.rs"]
mod test_subscription_manual;

#[path = "subscription/test_link_subscription_initial_data.rs"]
mod test_link_subscription_initial_data;

#[path = "subscription/subscription_options_tests.rs"]
mod subscription_options_tests;

#[path = "subscription/live_connection_tests.rs"]
mod live_connection_tests;

#[path = "subscription/slow_subscriber.rs"]
mod slow_subscriber;
