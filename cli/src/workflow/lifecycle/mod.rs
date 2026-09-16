//! Everyday database lifecycle: `up`, `down`, `status`, and `logs`.

mod down;
mod instance_kind;
mod instance_state;
mod instance_summary;
mod instances;
mod instances_options;
mod logs;
mod prepare;
mod server_details;
mod servers;
mod sql_diagnostics;
mod sql_log_cursor;
mod status;
mod tracked_server;
mod up;

pub use down::stop_database;
pub use instance_kind::InstanceKind;
pub use instance_state::InstanceState;
pub use instance_summary::InstanceSummary;
pub use instances::{list_instances, resolve_instance_name, show_cloud_instance_status};
pub use instances_options::InstancesOptions;
pub use logs::{print_database_logs, LogsOptions};
pub use prepare::{attach_or_start_managed_server, clear_managed_data, prepare_managed_server};
pub(crate) use servers::track_server;
pub use sql_diagnostics::{
    diagnostic_client, local_diagnostic_instance, sql_instance_logs, sql_instance_status,
};
pub use status::{show_lifecycle_status, show_local_instance_status};
pub use up::start_database;
