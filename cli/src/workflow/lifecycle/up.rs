//! `kalam up` — start the managed local database in the background.

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        db::seed::{maybe_seed_once, SeedMode},
        instance::StartedBy,
        target::{resolve_lifecycle_target, TargetSelector},
        WorkflowContext,
    },
};

pub async fn start_database(
    selector: &TargetSelector,
    explicit_port: Option<u16>,
    ctx: Option<&WorkflowContext>,
    output: &WorkflowOutput,
) -> Result<()> {
    let target = resolve_lifecycle_target(selector)?;
    if !target.is_managed_local() {
        return Err(CLIError::ConfigurationError(
            "`kalam up` starts a local database; use a local directory or `--global`, not a \
             remote `--env`"
                .into(),
        ));
    }

    let version = ctx.map(|item| item.config.resolved_server_version());
    let prepared = super::attach_or_start_managed_server(
        &target,
        explicit_port,
        StartedBy::Up,
        true,
        version,
        output,
    )
    .await?;

    if let Some(ctx) = ctx {
        let mut environment = target.to_environment();
        environment.url = prepared.record.url.clone();
        maybe_seed_once(ctx, &environment, Some(&prepared.layout), SeedMode::Once, output).await?;
    }

    output.agent_event(
        "KALAM_UP",
        &[
            ("environment", target.display_kind()),
            ("url", &prepared.record.url),
            ("namespace", target.namespace.as_str()),
        ],
    );
    if !output.is_agent() {
        output.status(format!(
            "Environment   {}\nDatabase      {}\nNamespace     {}",
            target.display_kind(),
            prepared.record.url,
            target.namespace.as_str()
        ));
        output.detail("View logs: kalam logs --follow");
    }
    Ok(())
}
