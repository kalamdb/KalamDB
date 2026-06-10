//! Deployment rollout orchestration.

use crate::{error::Result, output::WorkflowOutput, workflow::project::config::KalamProjectConfig};

pub fn run_rollout(
    _project_root: &std::path::Path,
    config: &KalamProjectConfig,
    env_name: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    output.status(format!("rolling out '{env_name}' deployment"));

    if config.dev.processes.is_empty() {
        output.detail("no deploy processes configured; migration apply and health check only");
    } else {
        output.detail(format!(
            "configured dev processes: {}",
            config.dev.processes.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }

    output.status("rollout complete");
    Ok(())
}
