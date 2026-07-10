//! Shared backend connection session lifecycle for KalamDB transports.

pub mod manager;
pub mod session;

pub use session::LiveSessionTransaction;
