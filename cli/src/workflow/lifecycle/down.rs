//! `kalam down` — stop the managed local database and keep its data.

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        instance::{self, save_instance},
        target::{resolve_lifecycle_target, TargetSelector},
    },
};

pub async fn stop_database(selector: &TargetSelector, output: &WorkflowOutput) -> Result<()> {
    let target = resolve_lifecycle_target(selector)?;
    if !target.is_managed_local() {
        return Err(CLIError::ConfigurationError(
            "`kalam down` stops a managed local database; remote environments are not suspended \
             by this command"
                .into(),
        ));
    }
    let Some(layout) = target.layout.as_ref() else {
        output.status("no managed database in this directory");
        return Ok(());
    };
    let Some(record) = instance::load_instance(layout)? else {
        output.status("database already stopped");
        output.agent_event("KALAM_DOWN", &[("state", "stopped")]);
        return Ok(());
    };

    if record.dev_pid.is_some() || record.pid.is_some() {
        instance::stop_recorded_processes(&record);
    }
    let mut stopped = record;
    instance::clear_live_pids(&mut stopped);
    save_instance(layout, &stopped)?;
    output.status("stopped local database; data preserved");
    output.agent_event("KALAM_DOWN", &[("state", "stopped"), ("url", &stopped.url)]);
    Ok(())
}
