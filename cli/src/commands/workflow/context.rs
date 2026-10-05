use kalam_cli::{
    config::WorkflowLoggingPolicy,
    output::WorkflowOutput,
    workflow::{lifecycle::find_instance, target::TargetSelector, WorkflowContext},
    CLIConfiguration, CLIError, FileCredentialStore, Result,
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

/// Database, deploy, link, and function commands follow the project. A saved
/// server selected with `--instance` must be the same endpoint, otherwise the
/// command would change the wrong database.
pub(super) fn ensure_project_command_matches_instance(
    cli: &Cli,
    ctx: &WorkflowContext,
) -> Result<()> {
    let Some((name, instance_url, project_url)) = instance_project_mismatch(cli, ctx)? else {
        return Ok(());
    };
    Err(CLIError::ConfigurationError(format!(
        "`--instance {name}` is {instance_url}, but this command is aimed at {project_url}. \
         Database, deploy, link, and function commands follow the project. Use `kalam --instance \
         {name} -c` for SQL on {name}, or run the command from that server's project."
    )))
}

pub(super) fn note_dev_keeps_project_database(cli: &Cli, ctx: &WorkflowContext) -> Result<()> {
    let Some((name, instance_url, project_url)) = instance_project_mismatch(cli, ctx)? else {
        return Ok(());
    };
    eprintln!(
        "`--instance {name}` is {instance_url}. `kalam dev` still uses this project's database at \
         {project_url}."
    );
    Ok(())
}

fn instance_project_mismatch(
    cli: &Cli,
    ctx: &WorkflowContext,
) -> Result<Option<(String, String, String)>> {
    let Some(name) = cli.explicit_instance() else {
        return Ok(None);
    };
    let store = FileCredentialStore::new().map_err(|error| {
        CLIError::ConfigurationError(format!("failed to read saved servers: {error}"))
    })?;
    let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
    let summary = find_instance(name, &output, &store)?;
    let Some(instance_url) = summary.url.filter(|url| !url.trim().is_empty()) else {
        return Ok(None);
    };
    let project_url = ctx.resolved_environment()?.url;
    if crate::connect::endpoints_match_servers(&instance_url, &project_url) {
        return Ok(None);
    }
    Ok(Some((name.to_string(), instance_url, project_url)))
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
