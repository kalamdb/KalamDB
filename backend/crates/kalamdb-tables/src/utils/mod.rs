//! Table utilities: MVCC providers, Parquet reads, and primary-key helpers.
//!
//! Shared planning helpers live in `kalamdb-datafusion-sources`.

pub mod base;
pub mod core;
pub mod datafusion_dml;
pub mod dml_provider;
pub mod parquet;
pub mod pk;
pub mod row_utils;
pub mod unified_dml;
pub mod vector_staging;
pub mod version_resolution;

#[cfg(test)]
pub mod test_backend;

pub(crate) use crate::shared_tables::SharedTableProvider;
pub use crate::stream_tables::StreamTableEngine;
pub use base::TableProviderCore;
pub use core::TableServices;
pub use dml_provider::TableRowDml;
pub use unified_dml::{extract_user_pk_value, validate_primary_key};
