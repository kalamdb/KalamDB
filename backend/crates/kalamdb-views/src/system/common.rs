//! Shared helpers for `system.*` virtual views.

use std::sync::{Arc, OnceLock};

use datafusion::arrow::datatypes::SchemaRef;
use kalamdb_commons::schemas::TableDefinition;
use kalamdb_system::SystemTablesRegistry;

use crate::view_base::{ViewTableProvider, VirtualView};

/// `i64` timestamp column. `#[table(view = true)]` maps this name to
/// `KalamDataType::Timestamp`.
pub type Timestamp = i64;

/// JSON column. `#[table(view = true)]` maps this name to `KalamDataType::Json`.
pub type Json = serde_json::Value;

/// Memoize an Arrow schema derived from a view [`TableDefinition`].
pub fn schema_from_definition(
    lock: &'static OnceLock<SchemaRef>,
    definition: impl FnOnce() -> TableDefinition,
) -> SchemaRef {
    lock.get_or_init(|| {
        definition()
            .to_arrow_schema()
            .expect("view TableDefinition must convert to Arrow schema")
    })
    .clone()
}

/// [`ViewTableProvider`] alias used by most system views.
pub type SystemViewProvider<V> = ViewTableProvider<V>;

/// Create a [`ViewTableProvider`] for a zero-argument view.
pub fn view_provider<V>(constructor: impl FnOnce() -> V) -> SystemViewProvider<V>
where
    V: VirtualView + 'static,
{
    SystemViewProvider::new(Arc::new(constructor()))
}

/// Create a [`ViewTableProvider`] for a view constructed from one parameter.
pub fn view_provider_with<V, P>(param: P, constructor: impl FnOnce(P) -> V) -> SystemViewProvider<V>
where
    V: VirtualView + 'static,
{
    SystemViewProvider::new(Arc::new(constructor(param)))
}

/// Create a [`ViewTableProvider`] for a registry-backed system view.
pub fn registry_view_provider<V>(
    system_registry: Arc<SystemTablesRegistry>,
    constructor: impl FnOnce(Arc<SystemTablesRegistry>) -> V,
) -> SystemViewProvider<V>
where
    V: VirtualView + 'static,
{
    SystemViewProvider::new(Arc::new(constructor(system_registry)))
}
