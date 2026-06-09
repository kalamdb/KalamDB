//! System.migrations table module.

pub mod migrations_provider;
pub mod models;

pub use migrations_provider::{MigrationsStore, MigrationsTableProvider};
