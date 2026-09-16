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
    let spinner = output.status_spinner("Checking server state");
    let Some(record) = instance::load_instance(layout)? else {
        drop(spinner);
        output.status("database already stopped");
        output.agent_event("KALAM_DOWN", &[("state", "stopped")]);
        return Ok(());
    };

    let was_running = instance::instance_is_live(&record);
    drop(spinner);
    if was_running || record.dev_pid.is_some_and(crate::process::pid_is_running) {
        let spinner = output.status_spinner("Stopping KalamDB server");
        instance::stop_recorded_processes(&record);
        if instance::instance_is_live(&record) {
            return Err(CLIError::ConfigurationError(
                "Server is still running; could not stop the managed process".into(),
            ));
        }
        drop(spinner);
    }
    let mut stopped = record;
    instance::clear_live_pids(&mut stopped);
    save_instance(layout, &stopped)?;
    output.status(if was_running {
        "Server stopped; data preserved"
    } else {
        "Server already stopped; data preserved"
    });
    output.agent_event("KALAM_DOWN", &[("state", "stopped"), ("url", &stopped.url)]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::WorkflowLoggingPolicy,
        workflow::instance::{new_instance_record, project_layout, StartedBy, TargetKind},
    };

    #[tokio::test]
    #[ntest::timeout(1500)]
    async fn repeated_down_reports_already_stopped() {
        let temp = tempfile::tempdir().unwrap();
        let layout = project_layout(temp.path());
        layout.ensure_dirs().unwrap();
        let record = new_instance_record(
            TargetKind::ProjectLocal,
            &layout,
            2900,
            None,
            None,
            StartedBy::Up,
            true,
            None,
            None,
        );
        save_instance(&layout, &record).unwrap();
        let output =
            WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled()).with_animations(false);
        stop_database(&TargetSelector::new(temp.path()), &output).await.unwrap();
        assert!(output
            .test_buffered_terminal_lines()
            .iter()
            .any(|line| line.contains("already stopped")));
        assert!(!output
            .test_buffered_terminal_lines()
            .iter()
            .any(|line| line.contains("stopped local database")));
    }
}
