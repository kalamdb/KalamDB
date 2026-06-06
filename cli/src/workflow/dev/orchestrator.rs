//! Main `kalam dev` session orchestration loop.

use std::time::SystemTime;

use tokio::time::{self, Duration};

use crate::{
    error::Result,
    output::WorkflowOutput,
    workflow::{
        dev::{
            logs::ServiceLogRegistry,
            processes::ProcessSupervisor,
            server::{build_local_server_command, server_already_ready, wait_for_server_ready},
            watch::{
                run_schema_pipeline, schema_file_changed, schema_file_mtime, schema_watch_path,
                SCHEMA_WATCH_INTERVAL_SECS,
            },
        },
        WorkflowContext,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaPipelineState {
    Idle,
    Synced,
    Paused,
}

pub struct DevSessionOptions {
    pub force: bool,
}

pub async fn run_dev_session(ctx: &WorkflowContext, options: DevSessionOptions) -> Result<()> {
    let output = ctx.output();
    let mut registry = ServiceLogRegistry::new();
    registry.register("server");

    for name in ctx.config.dev.processes.keys() {
        registry.register(name);
    }

    let server_source = registry.get("server").expect("server source registered").clone();

    output.service_log(&server_source, "starting kalam dev session");

    let mut supervisor = ProcessSupervisor::new();

    if ctx.config.dev.auto_start_db {
        let env = ctx.resolved_environment()?;
        if server_already_ready(&env.url).await {
            output.service_log(
                &server_source,
                format!("using existing KalamDB server at {}", env.url),
            );
        } else {
            let command = build_local_server_command(&ctx.project_root, &ctx.config)?;
            output.service_log(&server_source, "starting local KalamDB server");
            supervisor.spawn_one("server", &command, &registry, &output).await?;
            wait_for_server_ready(&env.url, &output).await?;
        }
    }

    let mut schema_state = SchemaPipelineState::Idle;
    let mut schema_mtime: Option<SystemTime> = None;

    if ctx.config.dev.apply_schema || ctx.config.dev.generate_types {
        schema_state = run_initial_schema_pipeline(ctx, &output, options.force);
        if let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) {
            schema_mtime = schema_file_mtime(&path);
        }
    }

    if !ctx.config.dev.processes.is_empty() {
        supervisor.spawn_all(&ctx.config.dev.processes, &registry, &output).await?;
    }

    let watch_enabled = schema_watch_path(&ctx.project_root, &ctx.config).is_some();
    let mut watch_interval = time::interval(Duration::from_secs(SCHEMA_WATCH_INTERVAL_SECS));
    watch_interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            result = &mut ctrl_c => {
                result.map_err(|e| crate::error::CLIError::ConfigurationError(format!("ctrl-c handler failed: {e}")))?;
                output.service_log(&server_source, "shutting down (Ctrl+C)");
                break;
            }
            _ = watch_interval.tick(), if watch_enabled => {
                let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) else {
                    continue;
                };
                if schema_state == SchemaPipelineState::Paused {
                    continue;
                }
                if schema_file_changed(&path, schema_mtime) {
                    output.service_log(&server_source, "schema change detected, running pipeline");
                    schema_state = run_initial_schema_pipeline(ctx, &output, false);
                    schema_mtime = schema_file_mtime(&path);
                }
            }
            _ = time::sleep(Duration::from_millis(200)) => {
                supervisor.reap_finished().await;
                let managed_count = supervisor.count();
                if managed_count == 0 && (!ctx.config.dev.processes.is_empty() || ctx.config.dev.auto_start_db) {
                    output.service_log(&server_source, "all dev processes exited");
                    break;
                }
            }
        }
    }

    supervisor.shutdown().await;
    Ok(())
}

fn run_initial_schema_pipeline(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    force: bool,
) -> SchemaPipelineState {
    match run_schema_pipeline(ctx, output) {
        Ok(()) => {
            output.status("schema pipeline completed");
            SchemaPipelineState::Synced
        },
        Err(error) => {
            if force {
                output.warn("retrying schema pipeline (--force)");
                return match run_schema_pipeline(ctx, output) {
                    Ok(()) => {
                        output.status("schema pipeline recovered");
                        SchemaPipelineState::Synced
                    },
                    Err(retry_error) => {
                        emit_schema_failure(output, &retry_error);
                        SchemaPipelineState::Paused
                    },
                };
            }
            emit_schema_failure(output, &error);
            SchemaPipelineState::Paused
        },
    }
}

fn emit_schema_failure(output: &WorkflowOutput, error: &crate::error::CLIError) {
    output.error(format!("schema pipeline failed: {error}"));
    output.warn(
        "schema pipeline paused; managed processes continue (retry with `kalam dev --force`)",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::{
        config::{CLIConfiguration, WorkflowLoggingPolicy},
        workflow::project::config::{
            ConnectionEnv, DevSection, KalamProjectConfig, LoggingSection, MigrationsSection,
            ProjectSection, SchemaMode, SchemaSection, SchemaTarget,
        },
    };

    fn failing_ctx(root: &std::path::Path) -> WorkflowContext {
        let migrations_dir = root.join("kalam/migrations");
        std::fs::create_dir_all(&migrations_dir).unwrap();
        std::fs::write(
            migrations_dir.join("20250101120000_fail.sql"),
            "-- UP\nSELECT __KALAM_TEST_SCHEMA_FAIL__;\n\n-- DOWN\n",
        )
        .unwrap();

        WorkflowContext {
            project_root: root.to_path_buf(),
            config: KalamProjectConfig {
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
                    watch: false,
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
                    watch: false,
                    ..Default::default()
                },
                logging: LoggingSection::default(),
            },
            cli_config: CLIConfiguration::default(),
            use_color: false,
            project_dir: None,
            env_override: None,
            namespace_override: None,
            url_override: None,
        }
    }

    #[test]
    fn force_retries_paused_schema_pipeline_after_fix() {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        std::fs::write(root.join("schema.sql"), "CREATE TABLE t (id INT);").unwrap();
        let ctx = failing_ctx(root);
        let output = WorkflowOutput::new(false, WorkflowLoggingPolicy::disabled());

        let paused = run_initial_schema_pipeline(&ctx, &output, false);
        assert_eq!(paused, SchemaPipelineState::Paused);

        // Remove failing migration and retry with force.
        std::fs::remove_file(root.join("kalam/migrations/20250101120000_fail.sql")).unwrap();
        let recovered = run_initial_schema_pipeline(&ctx, &output, true);
        assert_eq!(recovered, SchemaPipelineState::Synced);
    }
}
