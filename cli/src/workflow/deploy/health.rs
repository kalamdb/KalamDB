//! Post-deploy health verification.

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
};

pub async fn check_deploy_health(url: &str, output: &WorkflowOutput) -> Result<()> {
    let base = url.trim_end_matches('/');
    let health_url = format!("{base}/ui");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| CLIError::ConfigurationError(format!("failed to build HTTP client: {e}")))?;

    output.detail(format!("health check GET {health_url}"));

    let response =
        client.get(&health_url).send().await.map_err(|e| {
            CLIError::ConfigurationError(format!("health check request failed: {e}"))
        })?;

    let status = response.status();
    if status.is_success() || status.is_redirection() {
        output.status(format!("health check passed ({status})"));
        return Ok(());
    }

    Err(CLIError::ConfigurationError(format!(
        "health check failed with status {status}"
    )))
}
