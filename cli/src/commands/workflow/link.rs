use kalam_cli::{
    workflow::project::link::{link_project, LinkOptions},
    Result,
};

use super::context::workflow_context;
use crate::args::{Cli, LinkArgs};

pub(super) fn handle_link(cli: &Cli, args: &LinkArgs) -> Result<()> {
    let ctx = workflow_context(cli, args.project_dir.as_deref(), args.namespace.as_deref())?;
    link_project(
        &ctx,
        LinkOptions {
            env:       cli.env.clone(),
            url:       args.url.clone().or_else(|| cli.url.clone()),
            namespace: args.namespace.clone(),
        },
    )
}
