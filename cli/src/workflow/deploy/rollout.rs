//! Deployment rollout orchestration.

use crate::{error::Result, output::WorkflowOutput, workflow::project::config::KalamProjectConfig};

pub fn run_rollout(
    _project_root: &std::path::Path,
    config: &KalamProjectConfig,
    env_name: &str,
    output: &WorkflowOutput,
) -> Result<()> {
    output.detail(format!(
        "procedure activation for '{env_name}' does not roll back schema changes"
    ));
    if !config.dev.processes.is_empty() {
        output.detail(format!(
            "application processes are not started by deploy: {}",
            config.dev.processes.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(())
}
