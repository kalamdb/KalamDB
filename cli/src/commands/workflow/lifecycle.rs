use std::path::Path;

use kalam_cli::{
    workflow::{
        lifecycle::{self, InstanceKind, InstanceSummary},
        target::TargetSelector,
        WorkflowContext,
    },
    CLIError, FileCredentialStore, Result,
};

use super::context::{command_output, target_selector, try_workflow_context};
use crate::args::{Cli, DownArgs, InstancesArgs, LogsArgs, StatusArgs, UpArgs};

fn named_target(cli: &Cli, project_dir: Option<&Path>) -> Result<Option<InstanceSummary>> {
    let Some(name) = cli.explicit_instance() else {
        return Ok(None);
    };
    if cli.global
        || cli.env.is_some()
        || cli.url.is_some()
        || cli.host.is_some()
        || project_dir.is_some()
    {
        return Err(CLIError::ConfigurationError(
            "Use --instance on its own; it cannot be combined with --global, --env, --url, \
             --host, or --project-dir"
                .into(),
        ));
    }
    lifecycle::resolve_instance_name(name, &command_output(cli, None)).map(Some)
}

fn selected_context(
    cli: &Cli,
    project_dir: Option<&Path>,
    namespace: Option<&str>,
    named: Option<&InstanceSummary>,
) -> Result<(TargetSelector, Option<WorkflowContext>)> {
    if let Some(instance) = named {
        if instance.kind == InstanceKind::Cloud {
            return Err(CLIError::ConfigurationError(format!(
                "'{}' is a cloud connection. up and down manage local servers only; use `kalam \
                 status --instance {}` to check it.",
                instance.name, instance.name,
            )));
        }
        if instance.global {
            let mut selector = target_selector(cli, None, namespace);
            selector.global = true;
            selector.instance = None;
            return Ok((selector, None));
        }
        if instance.folder.as_ref().is_none_or(|folder| !folder.is_dir()) {
            return Err(CLIError::ConfigurationError(format!(
                "The folder for instance '{}' is unavailable. Run `kalam instances` to inspect it.",
                instance.name,
            )));
        }
    }
    let folder = named.and_then(|instance| instance.folder.as_deref()).or(project_dir);
    let mut ctx = try_workflow_context(cli, folder, namespace)?;
    if named.is_some() {
        if let Some(ctx) = ctx.as_mut() {
            ctx.instance = None;
        }
    }
    let mut selector = ctx
        .as_ref()
        .map(|ctx| ctx.target_selector())
        .unwrap_or_else(|| target_selector(cli, folder, namespace));
    if named.is_some() {
        selector.instance = None;
    }
    Ok((selector, ctx))
}

pub(super) async fn handle_up(cli: &Cli, args: &UpArgs) -> Result<()> {
    let named = named_target(cli, args.project_dir.as_deref())?;
    let (selector, ctx) = selected_context(cli, args.project_dir.as_deref(), None, named.as_ref())?;
    lifecycle::start_database(&selector, cli.port, ctx.as_ref(), &command_output(cli, ctx.as_ref()))
        .await
}

pub(super) async fn handle_down(cli: &Cli, args: &DownArgs) -> Result<()> {
    let named = named_target(cli, args.project_dir.as_deref())?;
    let (selector, ctx) = selected_context(cli, args.project_dir.as_deref(), None, named.as_ref())?;
    lifecycle::stop_database(&selector, &command_output(cli, ctx.as_ref())).await
}

pub(super) async fn handle_status(cli: &Cli, args: &StatusArgs) -> Result<()> {
    let named = named_target(cli, args.project_dir.as_deref())?;
    if let Some(instance) = named.as_ref().filter(|instance| instance.kind == InstanceKind::Cloud) {
        let client = lifecycle::diagnostic_client(instance)?.ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "No matching authentication for '{}'. Run `kalam login --instance {}`.",
                instance.name, instance.name
            ))
        })?;
        return lifecycle::sql_instance_status(instance, &client, &command_output(cli, None)).await;
    }
    let (selector, ctx) = selected_context(
        cli,
        args.project_dir.as_deref(),
        args.namespace.as_deref(),
        named.as_ref(),
    )?;
    let output = command_output(cli, ctx.as_ref());
    if let Some(instance) = lifecycle::local_diagnostic_instance(&selector, &output)? {
        if let Some(client) = lifecycle::diagnostic_client(&instance)? {
            return lifecycle::sql_instance_status(&instance, &client, &output).await;
        }
    }
    if named.is_some() {
        lifecycle::show_local_instance_status(&selector, ctx.as_ref(), &output).await
    } else {
        lifecycle::show_lifecycle_status(&selector, ctx.as_ref(), &output).await
    }
}

pub(super) async fn handle_logs(cli: &Cli, args: &LogsArgs) -> Result<()> {
    let named = named_target(cli, args.project_dir.as_deref())?;
    let options = lifecycle::LogsOptions {
        follow: args.follow,
        lines:  args.lines,
    };
    if let Some(instance) = named.as_ref().filter(|row| row.kind == InstanceKind::Cloud) {
        if args.local_file {
            return Err(CLIError::ConfigurationError(
                "--local-file is available only for local instances".into(),
            ));
        }
        let client = lifecycle::diagnostic_client(instance)?.ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "No matching authentication for '{}'. Run `kalam login --instance {}`.",
                instance.name, instance.name
            ))
        })?;
        return lifecycle::sql_instance_logs(
            instance,
            &client,
            options,
            &command_output(cli, None),
        )
        .await;
    }
    let (selector, ctx) = selected_context(cli, args.project_dir.as_deref(), None, named.as_ref())?;
    let output = command_output(cli, ctx.as_ref());
    if !args.local_file {
        if let Some(instance) = lifecycle::local_diagnostic_instance(&selector, &output)? {
            if let Some(client) = lifecycle::diagnostic_client(&instance)? {
                return lifecycle::sql_instance_logs(&instance, &client, options, &output).await;
            }
        }
    }
    lifecycle::print_database_logs(&selector, options, &output).await
}

pub(super) async fn handle_instances(cli: &Cli, args: &InstancesArgs) -> Result<()> {
    lifecycle::list_instances(
        lifecycle::InstancesOptions {
            local: args.local,
            cloud: args.cloud,
            check: args.check,
        },
        &command_output(cli, None),
        &FileCredentialStore::new()?,
    )
    .await
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use kalam_cli::workflow::lifecycle::InstanceState;

    use super::*;

    #[test]
    fn named_local_target_uses_registered_folder_without_requiring_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let cli = Cli::try_parse_from(["kalam", "status", "--instance", "analytics"]).unwrap();
        let instance = InstanceSummary {
            name:     "analytics".into(),
            kind:     InstanceKind::Local,
            state:    InstanceState::Stopped,
            url:      Some("http://127.0.0.1:2900".into()),
            folder:   Some(temp.path().into()),
            global:   false,
            user:     None,
            auth:     None,
            aliases:  vec![],
            identity: "test".into(),
        };
        let (selector, ctx) = selected_context(&cli, None, None, Some(&instance)).unwrap();
        assert_eq!(selector.start_dir, temp.path());
        assert_eq!(selector.project_dir.as_deref(), Some(temp.path()));
        assert!(selector.instance.is_none());
        assert!(ctx.is_none());
    }

    #[test]
    fn cloud_target_cannot_fall_back_to_current_folder_for_local_operations() {
        let cli = Cli::try_parse_from(["kalam", "down", "--instance", "production"]).unwrap();
        let instance = InstanceSummary {
            name:     "production".into(),
            kind:     InstanceKind::Cloud,
            state:    InstanceState::NotChecked,
            url:      Some("https://example.com".into()),
            folder:   None,
            global:   false,
            user:     None,
            auth:     Some("Signed in".into()),
            aliases:  vec![],
            identity: "production".into(),
        };
        assert!(selected_context(&cli, None, None, Some(&instance)).is_err());
    }
}
