//! Isolated `kalam dev --exec` test environments.

use std::{collections::HashMap, process::ExitStatus};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        db::seed::{maybe_seed_once, SeedMode},
        instance::{self, StartedBy},
        migration::apply::apply_migrations_for_db_command,
        target::TargetSelector,
        WorkflowContext,
    },
};

pub async fn run_isolated_exec(
    ctx: &WorkflowContext,
    command: &str,
    output: &WorkflowOutput,
) -> Result<ExitStatus> {
    let temp = tempfile::TempDir::new().map_err(|error| {
        CLIError::FileError(format!("failed to create isolated database directory: {error}"))
    })?;
    let selector = TargetSelector::new(temp.path());
    let target = crate::workflow::target::resolve_target(&selector)?;
    let prepared = crate::workflow::lifecycle::attach_or_start_managed_server(
        &target,
        None,
        StartedBy::Exec,
        false,
        Some(ctx.config.resolved_server_version()),
        output,
    )
    .await?;

    let mut exec_ctx = ctx.clone();
    exec_ctx.url_override = Some(prepared.record.url.clone());

    let cleanup = || {
        if let Ok(Some(record)) = instance::load_instance(&prepared.layout) {
            instance::stop_recorded_processes(&record);
        }
    };

    let result = async {
        let environment = exec_ctx.resolved_environment()?;
        apply_migrations_for_db_command(&exec_ctx, output).await?;
        maybe_seed_once(&exec_ctx, &environment, Some(&prepared.layout), SeedMode::Force, output)
            .await?;

        let mut env = HashMap::<String, String>::new();
        env.insert("KALAM_URL".to_string(), prepared.record.url.clone());
        env.insert("KALAM_NAMESPACE".to_string(), environment.namespace.as_str().to_string());
        env.insert("KALAM_ENV".to_string(), "exec".to_string());
        env.insert("KALAM_USER".to_string(), "root".to_string());
        env.insert(
            "KALAM_PASSWORD".to_string(),
            crate::workflow::dev::server::DEFAULT_LOCAL_DEV_ROOT_PASSWORD.to_string(),
        );

        output.status(format!("running `{command}` against {}", prepared.record.url));
        let mut shell = crate::process::shell_command(command);
        for (key, value) in &env {
            shell.env(key, value);
        }
        shell
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .map_err(|error| CLIError::FileError(format!("failed to run `{command}`: {error}")))
    }
    .await;

    cleanup();
    result
}
