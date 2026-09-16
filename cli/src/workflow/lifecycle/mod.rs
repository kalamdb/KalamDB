//! Everyday database lifecycle: `up`, `down`, `status`, and `logs`.

mod down;
mod logs;
mod prepare;
mod status;
mod up;

pub use down::stop_database;
pub use logs::{print_database_logs, LogsOptions};
pub use prepare::{attach_or_start_managed_server, clear_managed_data, prepare_managed_server};
pub use status::show_lifecycle_status;
pub use up::start_database;
