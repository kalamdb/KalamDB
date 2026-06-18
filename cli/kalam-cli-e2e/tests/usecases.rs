// Aggregator for use case tests to ensure Cargo picks them up
//
// Run these tests with:
//   cargo test --test usecases
//
// Run individual test files:
//   cargo test --test usecases test_batch_streaming
//   cargo test --test usecases test_chat_simulation
//   cargo test --test usecases test_datatypes_json
//   cargo test --test usecases test_reactive_transaction_workflow
//   cargo test --test usecases test_update_all_types

mod common;

#[path = "usecases/test_batch_streaming.rs"]
mod test_batch_streaming;

#[path = "usecases/test_chat_simulation.rs"]
mod test_chat_simulation;

#[path = "usecases/test_datatypes_json.rs"]
mod test_datatypes_json;

#[path = "usecases/test_update_all_types.rs"]
mod test_update_all_types;

#[path = "usecases/test_update_null_values.rs"]
mod test_update_null_values;

#[path = "usecases/test_reactive_transaction_workflow.rs"]
mod test_reactive_transaction_workflow;
