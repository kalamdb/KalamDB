//! Dart/Flutter SDK scaffolding for `kalam init` and related workflow commands.

pub mod init;
pub mod templates;

pub use init::{
    is_enabled, maybe_bootstrap_flutter_project, resolve_starter, DEFAULT_DEV_COMMAND,
    SCHEMA_TARGET_OUTPUT,
};
pub use templates::{resolve as resolve_dart_template, DEFAULT_TEMPLATE as DEFAULT_DART_TEMPLATE};
