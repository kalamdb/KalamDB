use kalam_cli::{
    workflow::{
        lifecycle::{list_instances, InstancesOptions},
        standalone_output,
    },
    FileCredentialStore, Result,
};

use crate::args::Cli;

pub async fn handle_credentials(
    cli: &Cli,
    credential_store: &mut FileCredentialStore,
) -> Result<bool> {
    if !cli.list_instances {
        return Ok(false);
    }
    let output =
        standalone_output(!cli.no_color, !cli.no_spinner && !cli.json, cli.json, cli.agent);
    list_instances(InstancesOptions::default(), &output, credential_store).await?;
    Ok(true)
}
