//! CREATE / ALTER / DROP TYPE handlers.

mod alter;
mod create;
mod drop;
mod replicate;

pub use alter::AlterTypeHandler;
pub use create::{ensure_implicit_row_type, CreateTypeHandler};
pub(crate) use create::{require_procedure_types, resolve_named_type_id};
pub use drop::DropTypeHandler;
use kalamdb_system::CatalogType;
pub(crate) use replicate::{replicate_dropped_type, replicate_named_type, replicate_stored_type};

pub(crate) fn type_alias(catalog_type: &CatalogType) -> String {
    format!("{}.{}", catalog_type.namespace_id, catalog_type.name)
}
