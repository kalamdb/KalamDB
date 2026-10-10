use kalam_cli::{workflow::deploy, Result};

use super::context::{ensure_project_command_matches_instance, workflow_context};
use crate::args::{Cli, DeployArgs};

pub(super) async fn handle_deploy(cli: &Cli, args: &DeployArgs) -> Result<()> {
    let ctx = workflow_context(cli, args.project_dir.as_deref(), None)?;
    ensure_project_command_matches_instance(cli, &ctx)?;
    deploy::run_deploy(
        &ctx,
        &deploy::DeployOptions {
            env:     cli.env.clone(),
            dry_run: args.dry_run,
        },
    )
    .await
}
