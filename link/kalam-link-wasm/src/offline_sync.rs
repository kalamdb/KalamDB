//! WASM entrypoints for the portable sync state machine.
//!
//! Export one batched call per planning operation instead of making a
//! Dart/JavaScript <-> Rust crossing for each row or queued action.

use wasm_bindgen::prelude::*;

/// Return JSON describing eligible durable outbox action IDs.
/// The browser storage adapter must claim them atomically before execution.
#[wasm_bindgen(js_name = syncPlanOutbox)]
pub fn sync_plan_outbox(request_json: &str) -> Result<String, JsValue> {
    link_common::offline_sync::plan_outbox_json(request_json)
        .map_err(|error| JsValue::from_str(&error))
}

/// Return JSON describing an allowed replay/snapshot transition.
/// No mutation occurs here; commit decision + rows in one storage transaction.
#[wasm_bindgen(js_name = syncPlanReplay)]
pub fn sync_plan_replay(request_json: &str) -> Result<String, JsValue> {
    link_common::offline_sync::plan_replay_json(request_json)
        .map_err(|error| JsValue::from_str(&error))
}
