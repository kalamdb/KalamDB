//! TypeScript SDK scaffolding for `kalam init` and related workflow commands.
//!
//! Dart and Rust SDK helpers will live in sibling modules (`dart/`, `rust/`) as they are added.

pub mod guidance;
pub mod init;
pub mod package_manager;
pub mod templates;

pub use init::{
    apply_scaffold, install_dependencies, is_enabled, resolve_package_manager, resolve_template,
    SCHEMA_TARGET_OUTPUT, SKIP_PACKAGE_INSTALL_ENV,
};
pub use package_manager::{
    default_package_manager, detect_installed_package_managers, detect_invoking_package_manager,
    detect_package_manager_from_project, execute_package_install, resolve_package_manager_for_init,
    PackageManager, PackageManagerInitOptions,
};
pub use templates::{
    resolve as resolve_typescript_template, DEFAULT_TEMPLATE as DEFAULT_TYPESCRIPT_TEMPLATE,
};
