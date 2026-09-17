use kalam_cli::{
    workflow::{target::TargetSelector, WorkflowContext},
    CLIConfiguration, CLIError, Result,
};

use crate::args::Cli;

pub(super) fn start_dir(project_dir: Option<&std::path::Path>) -> std::path::PathBuf {
    project_dir
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| ".".into())
}

pub(super) fn target_selector(
    cli: &Cli,
    project_dir: Option<&std::path::Path>,
    namespace: Option<&str>,
) -> TargetSelector {
    TargetSelector {
        start_dir:   start_dir(project_dir),
        project_dir: project_dir.map(std::path::Path::to_path_buf),
        global:      cli.global,
        env:         cli.env.clone(),
        url:         cli.url.clone(),
        host:        cli.host.clone(),
        port:        cli.port,
        namespace:   namespace.map(str::to_string),
        instance:    cli.explicit_instance().map(str::to_string),
    }
}

pub(super) fn apply_cli_target(ctx: &mut WorkflowContext, cli: &Cli) {
    ctx.global = cli.global;
    ctx.env_override = cli.env.clone().or(ctx.env_override.take());
    ctx.url_override = cli.url.clone().or(ctx.url_override.take());
    ctx.host = cli.host.clone();
    ctx.port = cli.port;
    ctx.instance = cli.explicit_instance().map(str::to_string);
    ctx.animations = !cli.no_spinner && !cli.json && !cli.agent;
    ctx.json = cli.json;
    if cli.agent {
        ctx.agent = true;
        ctx.use_color = false;
        ctx.animations = false;
    }
}

pub(super) fn workflow_context(
    cli: &Cli,
    project_dir: Option<&std::path::Path>,
    namespace: Option<&str>,
) -> Result<WorkflowContext> {
    let start = start_dir(project_dir);
    let cli_config = CLIConfiguration::load(&cli.config)?;
    let mut ctx = WorkflowContext::discover(
        &start,
        project_dir,
        &cli_config,
        !cli.no_color,
        cli.env.clone(),
        namespace.map(str::to_string),
        cli.url.clone(),
    )?;
    apply_cli_target(&mut ctx, cli);
    Ok(ctx)
}

pub(super) fn try_workflow_context(
    cli: &Cli,
    project_dir: Option<&std::path::Path>,
    namespace: Option<&str>,
) -> Result<Option<WorkflowContext>> {
    match workflow_context(cli, project_dir, namespace) {
        Ok(ctx) => Ok(Some(ctx)),
        Err(CLIError::ConfigurationError(message)) if message.contains("kalam.toml is missing") => {
            Ok(None)
        },
        Err(error) => Err(error),
    }
}

pub(super) fn command_output(
    cli: &Cli,
    ctx: Option<&WorkflowContext>,
) -> kalam_cli::output::WorkflowOutput {
    if let Some(ctx) = ctx {
        ctx.output()
    } else {
        kalam_cli::workflow::standalone_output(
            !cli.no_color,
            !cli.no_spinner && !cli.json,
            cli.json,
            cli.agent,
        )
    }
}
