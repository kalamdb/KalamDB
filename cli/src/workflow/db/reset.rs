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
    let targets = [
        server_dir.join("data"),
        server_dir.join("logs"),
    ];

    let mut removed_paths = 0usize;
    for path in targets {
        if !path.exists() {
            output.detail(format!(
                "skipped {} (not present)",
                display_project_path(&ctx.project_root, &path)
            ));
            continue;
        }

        fs::remove_dir_all(&path).map_err(|error| {
            CLIError::FileError(format!(
                "failed to remove '{}': {error}",
                path.display()
            ))
        })?;
        removed_paths += 1;
        output.status(format!(
            "removed {}",
            display_project_path(&ctx.project_root, &path)
        ));
    }

    if removed_paths == 0 {
        output.status("no local server data to reset");
    } else {
        output.status(format!(
            "cleared local dev server data ({removed_paths} director{}); run `kalam dev` to start fresh",
            if removed_paths == 1 { "y" } else { "ies" }
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
        config::WorkflowLoggingPolicy,
        workflow::test_support::parse_minimal_project_config,
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
    fn reset_removes_data_and_logs_directories() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let data = root.join("kalam/server/data/rocksdb");
        let logs = root.join("kalam/server/logs");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("CURRENT"), "ok").unwrap();
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("server.log"), "log").unwrap();

        let ctx = test_context(root);
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let summary = reset_local_dev_server_data(&ctx, &output).unwrap();

        assert_eq!(summary.removed_paths, 2);
        assert!(!data.exists());
        assert!(!logs.exists());
    }

    #[test]
    fn reset_is_no_op_when_directories_are_missing() {
        let temp = TempDir::new().unwrap();
        let ctx = test_context(temp.path());
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let summary = reset_local_dev_server_data(&ctx, &output).unwrap();

        assert_eq!(summary.removed_paths, 0);
    }

    #[test]
    fn reset_preserves_server_config() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let config_path = root.join("kalam/server/server.toml");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, "[auth]\nroot_password = \"kalamdb123\"\n").unwrap();
        fs::create_dir_all(root.join("kalam/server/data")).unwrap();

        let ctx = test_context(root);
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        reset_local_dev_server_data(&ctx, &output).unwrap();

        assert!(config_path.is_file());
    }
}
