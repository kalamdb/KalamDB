//! Realtime workflow scenarios.
//!
//! These are app-style scenarios centered on live workflows, offline sync,
//! filtered subscriptions, and collaborative user flows.

pub(super) mod helpers {
    pub use crate::helpers::*;
}

#[path = "../scenario_01_chat_app.rs"]
mod scenario_01_chat_app;
#[path = "../scenario_02_offline_sync.rs"]
mod scenario_02_offline_sync;
#[path = "../scenario_03_shopping_cart.rs"]
mod scenario_03_shopping_cart;
#[path = "../scenario_07_collaborative.rs"]
mod scenario_07_collaborative;
