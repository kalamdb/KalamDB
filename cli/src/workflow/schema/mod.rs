pub mod dart;
pub mod diff;
pub mod gen;
pub mod load;
pub mod model;
pub mod naming;
pub mod output;
pub mod procedure_bindings;
pub mod procedures;
pub mod rust;
pub mod types;
pub mod typescript;

pub use diff::diff_project_schema_files;
pub use gen::{generate_schema_artifacts, GenerateOptions};
pub use load::{compile_project_contract, pull_remote_schema};
pub use model::{LanguageTarget, SchemaSnapshot, TableKind};

#[cfg(test)]
mod generate_tests;
