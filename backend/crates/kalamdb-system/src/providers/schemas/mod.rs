//! System.namespaces table module (system_namespaces in RocksDB)

pub mod models;
pub mod namespaces_provider;

pub use models::Namespace;
pub use namespaces_provider::NamespacesTableProvider;
