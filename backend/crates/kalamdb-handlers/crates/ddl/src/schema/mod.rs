//! Schema DDL handlers.
//!
//! The module used to live at `namespace/`. Stored catalog rows and Raft
//! `CreateNamespace` commands stay on the folded name.

pub mod alter;
pub mod create;
pub mod drop;
pub mod show;
pub mod use_namespace;

pub use alter::AlterSchemaHandler;
pub use create::CreateSchemaHandler;
pub use drop::DropSchemaHandler;
pub use show::ShowSchemasHandler;
pub use use_namespace::UseSchemaHandler;
