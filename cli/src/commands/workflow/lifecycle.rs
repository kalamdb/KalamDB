use kalam_cli::{workflow::lifecycle, Result};

use super::context::{command_output, target_selector, try_workflow_context};
use crate::args::{Cli, DownArgs, LogsArgs, StatusArgs, UpArgs};

pub(super) async fn handle_up(cli: &Cli, args: &UpArgs) -> Result<()> {
    let ctx = try_workflow_context(cli, args.project_dir.as_deref(), None)?;
    let output = command_output(cli, ctx.as_ref());
    let selector = match ctx.as_ref() {
        Some(ctx) => ctx.target_selector(),
        None => target_selector(cli, args.project_dir.as_deref(), None),
    };
    lifecycle::start_database(&selector, cli.port, ctx.as_ref(), &output).await
}

pub(super) async fn handle_down(cli: &Cli, args: &DownArgs) -> Result<()> {
    let ctx = try_workflow_context(cli, args.project_dir.as_deref(), None)?;
    let output = command_output(cli, ctx.as_ref());
    let selector = match ctx.as_ref() {
        Some(ctx) => ctx.target_selector(),
        None => target_selector(cli, args.project_dir.as_deref(), None),
    };
    lifecycle::stop_database(&selector, &output).await
}

pub(super) async fn handle_status(cli: &Cli, args: &StatusArgs) -> Result<()> {
    let ctx = try_workflow_context(cli, args.project_dir.as_deref(), args.namespace.as_deref())?;
    let output = command_output(cli, ctx.as_ref());
    let selector = match ctx.as_ref() {
        Some(ctx) => ctx.target_selector(),
        None => target_selector(cli, args.project_dir.as_deref(), args.namespace.as_deref()),
    };
    lifecycle::show_lifecycle_status(&selector, ctx.as_ref(), &output).await
}

pub(super) async fn handle_logs(cli: &Cli, args: &LogsArgs) -> Result<()> {
    let selector = match try_workflow_context(cli, args.project_dir.as_deref(), None)? {
        Some(ctx) => ctx.target_selector(),
        None => target_selector(cli, args.project_dir.as_deref(), None),
    };
    lifecycle::print_database_logs(
        &selector,
        lifecycle::LogsOptions {
            follow: args.follow,
            lines:  args.lines,
        },
    )
    .await
}
