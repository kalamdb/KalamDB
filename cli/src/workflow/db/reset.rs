//! Reset local dev server data for the current project.

use std::fs;

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{display_project_path, WorkflowContext},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetSummary {
    pub removed_paths: usize,
}

pub fn reset_local_dev_server_data(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
) -> Result<ResetSummary> {
    let server_dir = ctx.config.local_server_dir(&ctx.project_root);
    let schema_baseline = ctx.config.schema_baseline_path(&ctx.project_root);

    let mut removed_paths = 0usize;

    if server_dir.exists() {
        fs::remove_dir_all(&server_dir).map_err(|error| {
            CLIError::FileError(format!("failed to remove '{}': {error}", server_dir.display()))
        })?;
        removed_paths += 1;
        output.status(format!(
            "removed {}",
            display_project_path(&ctx.project_root, &server_dir)
        ));
    } else {
        output.detail(format!(
            "skipped {} (not present)",
            display_project_path(&ctx.project_root, &server_dir)
        ));
    }

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
    } else {
        output.detail(format!(
            "skipped {} (not present)",
            display_project_path(&ctx.project_root, &schema_baseline)
        ));
    }

    if removed_paths == 0 {
        output.status("no local server data to reset");
    } else {
        output.status(format!(
            "cleared local dev server data ({removed_paths} path{}); run `kalam dev` to start fresh",
            if removed_paths == 1 { "" } else { "s" }
        ));
    }

    Ok(ResetSummary { removed_paths })
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
            project_root: root.to_path_buf(),
            config: parse_minimal_project_config(),
            cli_config: crate::config::CLIConfiguration::default(),
            use_color: false,
            project_dir: None,
            env_override: None,
            namespace_override: None,
            url_override: None,
        }
    }

    #[test]
    fn reset_removes_entire_server_directory() {
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
        let summary = reset_local_dev_server_data(&ctx, &output).unwrap();

        assert_eq!(summary.removed_paths, 1);
        assert!(!server_dir.exists());
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
        let summary = reset_local_dev_server_data(&ctx, &output).unwrap();

        assert_eq!(summary.removed_paths, 1);
        assert!(!baseline.exists());
    }

    #[test]
    fn reset_removes_server_directory_and_schema_baseline() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let ctx = test_context(root);
        let server_dir = ctx.config.local_server_dir(root);
        let baseline = ctx.config.schema_baseline_path(root);
        fs::create_dir_all(server_dir.join("data")).unwrap();
        fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        fs::write(&baseline, "CREATE TABLE foo (id INT);").unwrap();

        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let summary = reset_local_dev_server_data(&ctx, &output).unwrap();

        assert_eq!(summary.removed_paths, 2);
        assert!(!server_dir.exists());
        assert!(!baseline.exists());
    }

    #[test]
    fn reset_is_no_op_when_nothing_is_present() {
        let temp = TempDir::new().unwrap();
        let ctx = test_context(temp.path());
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let summary = reset_local_dev_server_data(&ctx, &output).unwrap();

        assert_eq!(summary.removed_paths, 0);
    }
}
