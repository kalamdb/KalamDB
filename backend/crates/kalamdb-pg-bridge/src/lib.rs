//! PostgreSQL extension bridge.
//!
//! This crate is the only server-side host for the extension gRPC service.
//! The native PostgreSQL wire listener does not use it. When
//! `pg_extension.enabled` is false, [`install`] returns before building the
//! service, so an idle server does not register those RPC methods.
//!
//! Removing the extension later means deleting this crate, `kalamdb-pg`, and
//! the `install` call in the server lifecycle.

mod error;
mod host;
mod scan;
mod service;

pub use host::{connect, install};
pub use service::OperationService;
