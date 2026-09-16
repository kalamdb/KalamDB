use kalam_cli::{output::WorkflowDisplayMode, workflow::dev, CLIError, Result};

use super::context::workflow_context;
use crate::args::{Cli, DevArgs, DevCommand};

pub(super) async fn handle_dev(cli: &Cli, args: &DevArgs) -> Result<()> {
    let ctx = workflow_context(cli, args.project_dir.as_deref(), args.namespace.as_deref())?;
    if let Some(command) = args.exec.as_deref() {
        if args.command.is_some() {
            return Err(CLIError::ConfigurationError(
                "`kalam dev --exec` cannot be combined with start/stop/status/logs".into(),
            ));
        }
        let code = dev::run_dev_exec(&ctx, command).await?;
        std::process::exit(code);
    }
    let display_mode = if ctx.agent {
        WorkflowDisplayMode::Agent
    } else if cli.verbose {
        WorkflowDisplayMode::Verbose
    } else {
        WorkflowDisplayMode::Normal
    };
    match &args.command {
        None => dev::run_dev(&ctx, args.force, display_mode).await,
        Some(DevCommand::Start) => dev::start_dev(&ctx, args.force).await,
        Some(DevCommand::Status) => dev::show_session_status(&ctx).await,
        Some(DevCommand::Logs(logs)) => {
            dev::print_session_logs(&ctx, logs.follow, logs.lines).await
        },
        Some(DevCommand::Stop) => dev::stop_dev(&ctx).await,
    }
}
