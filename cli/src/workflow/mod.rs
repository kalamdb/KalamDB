//! Shared workflow module surface, grouped by command category.
//!
//! | Folder | Commands |
//! |---|---|
//! | [`project::init`] | `kalam init` |
//! | [`project::link`] | `kalam link` |
//! | [`lifecycle`] | `kalam up` / `down` / `status` / `logs` |
//! | [`dev`] | `kalam dev` |
//! | [`db`] | `kalam db` (migrate, reset, seed, migration) |
//! | [`schema`] | `kalam schema` |
//! | [`deploy`] | `kalam deploy` |
//! | [`functions`] | `kalam functions` |

pub(crate) mod agent;
pub(crate) mod auth;
pub mod context;
pub mod db;
pub mod deploy;
pub mod dev;
pub mod functions;
pub(crate) mod instance;
pub(crate) mod io;
pub mod lifecycle;
pub mod project;
pub(crate) mod prompts;
pub mod schema;
pub(crate) mod sql;
pub mod target;

#[cfg(test)]
pub(crate) mod test_support;

pub use context::{standalone_output, WorkflowContext};
pub use db::{migration, DbResetOptions};
pub(crate) use io::display_project_path;
