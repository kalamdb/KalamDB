//! Shared workflow module surface for project lifecycle commands.

pub mod db;
pub mod deploy;
pub mod dev;
pub mod migration;
pub mod project;
pub mod schema;

use std::path::PathBuf;

use crate::{
    config::{CLIConfiguration, WorkflowLoggingPolicy},
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::{
        db::migrate::apply_migrations_for_db_command,
        migration::{
            create::{create_migration, CreateMigrationOptions},
            status::migration_status,
        },
        project::{
            config::KalamProjectConfig,
            init::{run_init, InitOptions},
            link::{link_environment, LinkOptions},
            resolve::{resolve_environment, EnvironmentOverrides, ResolvedEnvironment},
            status::show_status,
        },
        schema::{
            gen::{generate_schema_artifacts, validate_language_filter, GenerateOptions},
            load::pull_remote_schema,
        },
    },
};

/// Shared workflow context for command handlers.
#[derive(Debug, Clone)]
pub struct WorkflowContext {
    pub project_root: PathBuf,
    pub config: KalamProjectConfig,
    pub cli_config: CLIConfiguration,
    pub use_color: bool,
    pub project_dir: Option<PathBuf>,
    pub env_override: Option<String>,
    pub namespace_override: Option<String>,
    pub url_override: Option<String>,
}

impl WorkflowContext {
    pub fn discover(
        start: &std::path::Path,
        project_dir: Option<&std::path::Path>,
        cli_config: &CLIConfiguration,
        use_color: bool,
        env_override: Option<String>,
        namespace_override: Option<String>,
        url_override: Option<String>,
    ) -> Result<Self> {
        let (project_root, config) = KalamProjectConfig::discover(start, project_dir)?;
        Ok(Self {
            project_root,
            config,
            cli_config: cli_config.clone(),
            use_color,
            project_dir: project_dir.map(PathBuf::from),
            env_override,
            namespace_override,
            url_override,
        })
    }

    pub fn output(&self) -> WorkflowOutput {
        let logging = WorkflowLoggingPolicy::merge_global(
            &self.project_root,
            &self.config.logging,
            self.cli_config.workflow_logging.as_ref(),
        );
        WorkflowOutput::new(self.use_color, logging)
    }

    pub fn resolved_environment(&self) -> Result<ResolvedEnvironment> {
        resolve_environment(
            &self.config,
            &EnvironmentOverrides {
                env: self.env_override.as_deref(),
                url: self.url_override.as_deref(),
                namespace: self.namespace_override.as_deref(),
            },
        )
    }
}

pub fn init_project(options: InitOptions, use_color: bool) -> Result<()> {
    let output = WorkflowOutput::new(use_color, WorkflowLoggingPolicy::disabled());
    run_init(options, &output)
}

pub fn generate_schema(ctx: &WorkflowContext, languages: Option<Vec<String>>) -> Result<()> {
    let output = ctx.output();
    if let Some(requested) = languages.as_ref() {
        validate_language_filter(requested, &ctx.config.schema.languages)?;
    }
    generate_schema_artifacts(
        &ctx.project_root,
        &ctx.config,
        &GenerateOptions { languages },
        &output,
    )
}

pub fn pull_schema(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    let env = ctx.resolved_environment()?;
    let snapshot = pull_remote_schema(&ctx.project_root, &ctx.config, &env.url, &env.namespace)?;

    if let Some(path) = ctx.config.schema_source_path(&ctx.project_root) {
        let sql = render_snapshot_as_sql(&snapshot);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, sql)?;
        output.status(format!("pulled schema into {}", path.display()));
    }

    Ok(())
}

fn render_snapshot_as_sql(snapshot: &crate::workflow::schema::SchemaSnapshot) -> String {
    let mut out = String::from("-- Pulled from remote schema\n");
    for table in snapshot.tables.values() {
        out.push_str(&format!("\nCREATE TABLE {} (\n", table.name));
        for (idx, column) in table.columns.iter().enumerate() {
            let comma = if idx + 1 < table.columns.len() {
                ","
            } else {
                ""
            };
            let nullability = if column.nullable { "" } else { " NOT NULL" };
            let pk = if column.primary_key {
                " PRIMARY KEY"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {} {}{}{}{}\n",
                column.name, column.sql_type, nullability, pk, comma
            ));
        }
        out.push_str(");\n");
    }
    out
}

pub fn create_project_migration(ctx: &WorkflowContext, name: String) -> Result<()> {
    let output = ctx.output();
    create_migration(&ctx.project_root, &ctx.config, &CreateMigrationOptions { name }, &output)
}

pub fn show_migration_status(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    migration_status(&ctx.project_root, &ctx.config, &output)
}

pub fn migrate_database(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    apply_migrations_for_db_command(&ctx.project_root, &ctx.config, &output)
}

pub async fn run_dev(ctx: &WorkflowContext, force: bool) -> Result<()> {
    dev::run_dev_session(ctx, dev::DevSessionOptions { force }).await
}

pub fn link_project(ctx: &WorkflowContext, options: LinkOptions) -> Result<()> {
    let output = ctx.output();
    link_environment(ctx, &options, &output)
}

pub fn project_status(ctx: &WorkflowContext) -> Result<()> {
    let output = ctx.output();
    show_status(ctx, &output)
}

pub async fn deploy_project(ctx: &WorkflowContext, env: Option<String>) -> Result<()> {
    deploy::run_deploy(ctx, &deploy::DeployOptions { env }).await
}

pub fn not_implemented(command: &str) -> Result<()> {
    Err(CLIError::ConfigurationError(format!(
        "{command} is not implemented yet; see `kalam {command} --help`"
    )))
}
