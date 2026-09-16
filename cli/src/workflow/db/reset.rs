//! Reset a managed local database from committed migrations and fixtures.

use std::fs;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        db::seed::{maybe_seed_once, SeedMode},
        display_project_path,
        instance::{self, save_instance, StartedBy},
        lifecycle::clear_managed_data,
        migration::apply::apply_migrations_for_db_command,
        WorkflowContext,
    },
};

#[derive(Debug, Clone, Copy, Default)]
pub struct DbResetOptions {
    pub assume_yes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetSummary {
    pub removed_paths: usize,
}

pub fn reset_local_dev_server_data(
    ctx: &WorkflowContext,
    layout: &crate::workflow::instance::ManagedLayout,
    output: &WorkflowOutput,
) -> Result<ResetSummary> {
    let mut removed_paths = 0usize;
    if layout.data_dir.exists() || layout.root.exists() {
        removed_paths += clear_managed_data(&layout, output)?;
    } else {
        output.detail(format!(
            "skipped {} (not present)",
            display_project_path(&ctx.project_root, &layout.data_dir)
        ));
    }

    let schema_baseline = ctx.config.schema_baseline_path(&ctx.project_root);
    if schema_baseline.exists() {
        fs::remove_file(&schema_baseline).map_err(|error| {
            CLIError::FileError(format!(
                "failed to remove '{}': {error}",
                schema_baseline.display()
            ))
        })?;
        removed_paths += 1;
        output.status(format!(
            "removed {}",
            display_project_path(&ctx.project_root, &schema_baseline)
        ));
    }

    if let Ok(Some(mut record)) = instance::load_instance(&layout) {
        record.seed_sql_hash = None;
        instance::clear_live_pids(&mut record);
        save_instance(&layout, &record)?;
    }

    if removed_paths == 0 {
        output.status("no local server data to reset");
    } else {
        output.status("cleared local database data; configuration preserved");
    }

    Ok(ResetSummary { removed_paths })
}

pub async fn reset_managed_database(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    _options: DbResetOptions,
) -> Result<()> {
    let target = ctx.resolved_target()?;
    if !target.is_managed_local() {
        return Err(CLIError::ConfigurationError(
            "`kalam db reset` rebuilds the local database from migrations and fixtures; remote \
             namespace reset is not part of this command"
                .into(),
        ));
    }
    let layout = target.layout.clone().ok_or_else(|| {
        CLIError::ConfigurationError("local reset requires a managed database layout".into())
    })?;
    if let Ok(Some(record)) = instance::load_instance(&layout) {
        instance::stop_recorded_processes(&record);
        let mut stopped = record;
        instance::clear_live_pids(&mut stopped);
        stopped.seed_sql_hash = None;
        save_instance(&layout, &stopped)?;
    }

    reset_local_dev_server_data(ctx, &layout, output)?;
    crate::workflow::lifecycle::attach_or_start_managed_server(
        &target,
        None,
        StartedBy::Up,
        true,
        Some(ctx.config.resolved_server_version()),
        output,
    )
    .await?;
    apply_migrations_for_db_command(ctx, output).await?;
    let environment = ctx.resolved_environment()?;
    maybe_seed_once(ctx, &environment, Some(&layout), SeedMode::Force, output).await?;
    output.status("local database reset from migrations and development fixtures");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::{
        config::WorkflowLoggingPolicy, workflow::test_support::parse_minimal_project_config,
    };

    fn test_context(root: &std::path::Path) -> WorkflowContext {
        WorkflowContext {
            project_root:       root.to_path_buf(),
            config:             parse_minimal_project_config(),
            cli_config:         crate::config::CLIConfiguration::default(),
            use_color:          false,
            animations:         true,
            agent:              false,
            json:               false,
            project_dir:        None,
            env_override:       None,
            namespace_override: None,
            url_override:       None,
            global:             false,
            host:               None,
            port:               None,
            instance:           None,
        }
    }

    #[test]
    fn reset_removes_data_and_keeps_server_config() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let server_dir = root.join("kalam/server");
        let data = server_dir.join("data/rocksdb");
        let logs = server_dir.join("logs");
        let config_path = server_dir.join("server.toml");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("CURRENT"), "ok").unwrap();
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("server.log"), "log").unwrap();
        fs::write(&config_path, "[auth]\nroot_password = \"kalamdb123\"\n").unwrap();

        let ctx = test_context(root);
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let layout = crate::workflow::instance::project_layout(root);
        let summary = reset_local_dev_server_data(&ctx, &layout, &output).unwrap();

        assert!(summary.removed_paths >= 1);
        assert!(config_path.is_file());
        assert!(!data.join("CURRENT").exists());
        assert!(server_dir.exists());
    }

    #[test]
    fn reset_removes_schema_baseline_file() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let ctx = test_context(root);
        let baseline = ctx.config.schema_baseline_path(root);
        fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        fs::write(&baseline, "CREATE TABLE foo (id INT);").unwrap();

        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let layout = crate::workflow::instance::project_layout(root);
        let summary = reset_local_dev_server_data(&ctx, &layout, &output).unwrap();

        assert_eq!(summary.removed_paths, 1);
        assert!(!baseline.exists());
    }

    #[test]
    fn reset_is_no_op_when_nothing_is_present() {
        let temp = TempDir::new().unwrap();
        let ctx = test_context(temp.path());
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let layout = crate::workflow::instance::project_layout(temp.path());
        let summary = reset_local_dev_server_data(&ctx, &layout, &output).unwrap();

        assert_eq!(summary.removed_paths, 0);
    }
}
