//! Library entry point for kalam-cli components.
//!
//! Exposes reusable modules (formatter, session, config, etc.) so integration
//! tests and other crates can leverage CLI formatting and behaviors without
//! going through the binary entry point.

/// CLI version constant (avoids repeated env! macro calls)
pub const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod completer;
pub mod config;
pub mod credentials;
pub mod error;
pub mod formatter;
pub mod history;
pub mod history_menu;
pub mod output;
pub mod parser;
pub mod session;
pub mod terminal_input;
pub mod terminal_ui;
pub mod update_check;
pub mod workflow;

pub use config::CLIConfiguration;
pub use credentials::FileCredentialStore;
pub use error::{CLIError, Result};
pub use formatter::OutputFormatter;
pub use session::{CLISession, OutputFormat};
