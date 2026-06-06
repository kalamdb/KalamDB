//! Schema file watch helpers and the dev schema pipeline.

use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    error::Result,
    output::WorkflowOutput,
    workflow::{
        migration::apply::apply_pending_migrations,
        project::config::KalamProjectConfig,
        schema::gen::{generate_schema_artifacts, GenerateOptions},
        WorkflowContext,
    },
};

/// Poll interval for schema file changes during `kalam dev`.
pub const SCHEMA_WATCH_INTERVAL_SECS: u64 = 2;

/// Run apply-migrations, optional type generation, and baseline update.
pub fn run_schema_pipeline(ctx: &WorkflowContext, output: &WorkflowOutput) -> Result<()> {
    let config = &ctx.config;
    let project_root = &ctx.project_root;

    if config.dev.apply_schema {
        apply_pending_migrations(project_root, config, output)?;
    }

    if config.dev.generate_types {
        generate_schema_artifacts(
            project_root,
            config,
            &GenerateOptions { languages: None },
            output,
        )?;
    }

    update_schema_baseline(project_root, config)?;
    Ok(())
}

/// Copy the active schema source into `kalam/.schema-baseline.sql`.
pub fn update_schema_baseline(project_root: &Path, config: &KalamProjectConfig) -> Result<()> {
    let Some(source) = config.schema_source_path(project_root) else {
        return Ok(());
    };
    if !source.is_file() {
        return Ok(());
    }

    let baseline = config.schema_baseline_path(project_root);
    if let Some(parent) = baseline.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&source, &baseline)?;
    Ok(())
}

/// Resolved schema source path when file-based watching is enabled.
pub fn schema_watch_path(project_root: &Path, config: &KalamProjectConfig) -> Option<PathBuf> {
    if !config.dev.watch || !config.schema.watch {
        return None;
    }
    config.schema_source_path(project_root)
}

/// Read filesystem modification time for a schema watch target.
pub fn schema_file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

/// Returns true when `path` mtime differs from `previous`.
pub fn schema_file_changed(path: &Path, previous: Option<SystemTime>) -> bool {
    let current = schema_file_mtime(path);
    current != previous
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    use crate::{
        config::WorkflowLoggingPolicy,
        workflow::project::config::{
            ConnectionEnv, DevSection, LoggingSection, MigrationsSection, ProjectSection,
            SchemaMode, SchemaSection, SchemaTarget,
        },
    };

    fn sample_config() -> KalamProjectConfig {
        KalamProjectConfig {
            project: ProjectSection {
                name: "demo".into(),
                default_env: "dev".into(),
            },
            connection: HashMap::from([(
                "dev".into(),
                ConnectionEnv {
                    url: "http://localhost:2900".into(),
                    namespace: "demo".into(),
                },
            )]),
            schema: SchemaSection {
                mode: SchemaMode::Sql,
                path: Some("schema.sql".into()),
                watch: true,
                languages: vec!["typescript".into()],
                targets: HashMap::from([(
                    "typescript".into(),
                    SchemaTarget {
                        output: "src/generated/kalam.ts".into(),
                    },
                )]),
            },
            migrations: MigrationsSection::default(),
            dev: DevSection {
                apply_schema: true,
                generate_types: false,
                watch: true,
                ..Default::default()
            },
            logging: LoggingSection::default(),
        }
    }

    #[test]
    fn update_baseline_copies_schema_source() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(root.join("schema.sql"), "CREATE TABLE t (id INT);").unwrap();
        let config = sample_config();

        update_schema_baseline(root, &config).unwrap();

        let baseline = config.schema_baseline_path(root);
        assert!(baseline.is_file());
        let contents = fs::read_to_string(baseline).unwrap();
        assert!(contents.contains("CREATE TABLE"));
    }

    #[test]
    fn schema_file_changed_detects_updates() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("schema.sql");
        fs::write(&path, "v1").unwrap();
        let first = schema_file_mtime(&path);
        assert!(!schema_file_changed(&path, first));

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&path, "v2").unwrap();
        assert!(schema_file_changed(&path, first));
    }

    #[test]
    fn run_schema_pipeline_updates_baseline() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/generated")).unwrap();
        fs::write(root.join("schema.sql"), "CREATE TABLE users (id INT);").unwrap();
        let config = sample_config();
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());
        let ctx = WorkflowContext {
            project_root: root.to_path_buf(),
            config,
            cli_config: crate::config::CLIConfiguration::default(),
            use_color: false,
            project_dir: None,
            env_override: None,
            namespace_override: None,
            url_override: None,
        };

        run_schema_pipeline(&ctx, &output).unwrap();
        assert!(ctx.config.schema_baseline_path(root).is_file());
    }
}
