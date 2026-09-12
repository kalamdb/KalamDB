//! CREATE / ALTER / DROP TYPE handlers.

mod alter;
mod create;
mod drop;

pub use alter::AlterTypeHandler;
pub(crate) use create::require_procedure_types;
pub use create::{ensure_implicit_row_type, CreateTypeHandler};
pub use drop::DropTypeHandler;
