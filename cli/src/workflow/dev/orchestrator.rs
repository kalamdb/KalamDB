//! Main `kalam dev` session orchestration loop.

use std::time::SystemTime;

use tokio::time::{self, Duration};

use crate::{
    error::Result,
    output::{WorkflowDisplayMode, WorkflowOutput},
    terminal_ui::ProgressTaskStatus,
    workflow::{
        dev::{
            logs::ServiceLogRegistry,
            precheck::{ensure_local_dev_authentication_ready, run_dev_prechecks},
            processes::ProcessSupervisor,
            server::{build_local_server_command, wait_for_server_ready},
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
    pub display_mode: WorkflowDisplayMode,
}

pub async fn run_dev_session(ctx: &WorkflowContext, options: DevSessionOptions) -> Result<()> {
    let output = ctx.output().with_display_mode(options.display_mode);
    let mut registry = ServiceLogRegistry::new();
    registry.register("server");

    for name in ctx.config.dev.processes.keys() {
        registry.register(name);
    }

    let server_source = registry.get("server").expect("server source registered").clone();

    output.service_log(&server_source, "starting kalam dev session");
    output.progress_task(
        "project",
        ProgressTaskStatus::Succeeded,
        project_ready_message(&ctx.config.project.name, &ctx.project_root),
    );
    let precheck = run_dev_prechecks(ctx, &output, &server_source).await?;

    let mut supervisor = ProcessSupervisor::new();
    let mut local_server_spawned = false;

    if ctx.config.dev.auto_start_db {
        if precheck.local_server_reused {
            output.service_log(
                &server_source,
                format!("using existing KalamDB server at {}", precheck.environment.url),
            );
            output.progress_task(
                "server",
                ProgressTaskStatus::Succeeded,
                local_server_ready_message(&precheck.environment.url, output.workflow_log_path()),
            );
        } else {
            let command = build_local_server_command(
                &ctx.project_root,
                &ctx.config,
                &precheck.environment.url,
            )?;
            output.service_log(&server_source, "starting local KalamDB server");
            output.progress_task(
                "server",
                ProgressTaskStatus::Running,
                "Starting local KalamDB server...",
            );
            supervisor.spawn_one("server", &command, &registry, &output).await?;
            wait_for_server_ready(&precheck.environment.url, &output).await?;
            ensure_local_dev_authentication_ready(
                ctx,
                &precheck.environment,
                &output,
                &server_source,
            )
            .await?;
            output.progress_task(
                "server",
                ProgressTaskStatus::Succeeded,
                local_server_ready_message(&precheck.environment.url, output.workflow_log_path()),
            );
            local_server_spawned = true;
        }
    }

    let mut schema_mtime: Option<SystemTime> = None;

    if ctx.config.dev.apply_schema || ctx.config.dev.generate_types {
        let _ = run_initial_schema_pipeline(ctx, &output, options.force, None).await;
        if let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) {
            schema_mtime = schema_file_mtime(&path);
        }
    }

    if !ctx.config.dev.processes.is_empty() {
        supervisor.spawn_all(&ctx.config.dev.processes, &registry, &output).await?;
    }

    let watch_enabled = precheck.watch_enabled;
    if watch_enabled {
        output.service_log(&server_source, "watching schema source for changes");
    }
    if ctx.config.dev.processes.is_empty() {
        output.service_log(&server_source, "no custom dev processes configured");
    }
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
                if schema_file_changed(&path, schema_mtime) {
                    let watched_path = display_project_relative_path(&ctx.project_root, &path);
                    let message = format!("Schema changed in {watched_path}; applying...");
                    output.status(&message);
                    let _ =
                        run_initial_schema_pipeline(ctx, &output, options.force, Some(message)).await;
                    schema_mtime = schema_file_mtime(&path);
                }
            }
            _ = time::sleep(Duration::from_millis(200)) => {
                let finished = supervisor.reap_finished().await;
                for (name, code) in &finished {
                    if let Some(source) = registry.get(name) {
                        output.service_log(source, format!("process exited with code {code}"));
                    }
                }
                let managed_count = supervisor.count();
                if managed_count == 0 && (local_server_spawned || !ctx.config.dev.processes.is_empty()) {
                    output.service_log(&server_source, "all dev processes exited");
                    break;
                }
                if managed_count == 0 && !watch_enabled {
                    output.service_log(&server_source, "dev session completed");
                    break;
                }
            }
        }
    }

    supervisor.shutdown().await;
    Ok(())
}

async fn run_initial_schema_pipeline(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    force: bool,
    running_message: Option<String>,
) -> SchemaPipelineState {
    let running_message = running_message.unwrap_or_else(|| "Applying schema...".to_string());
    output.progress_task("schema", ProgressTaskStatus::Running, &running_message);
    match run_schema_pipeline(ctx, output, force).await {
        Ok(()) => {
            if output.display_mode == WorkflowDisplayMode::Progress {
                output.progress_task("schema", ProgressTaskStatus::Succeeded, "Schema applied");
            } else {
                output.status("schema pipeline completed");
            }
            SchemaPipelineState::Synced
        },
        Err(error) => {
            if force {
                output.warn("retrying schema pipeline (--force)");
                return match run_schema_pipeline(ctx, output, true).await {
                    Ok(()) => {
                        if output.display_mode == WorkflowDisplayMode::Progress {
                            output.progress_task(
                                "schema",
                                ProgressTaskStatus::Succeeded,
                                "Schema recovered",
                            );
                        } else {
                            output.status("schema pipeline recovered");
                        }
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
    if output.display_mode == WorkflowDisplayMode::Progress {
        output.progress_task(
            "schema",
            ProgressTaskStatus::Failed,
            format!("Schema failed: {error}"),
        );
        return;
    }
    output.error(format!("schema pipeline failed: {error}"));
    output.warn(
        "schema pipeline paused; managed processes continue (retry with `kalam dev --force`)",
    );
}

fn project_ready_message(project_name: &str, project_root: &std::path::Path) -> String {
    format!("Project {project_name} at {}", project_root.display())
}

fn local_server_ready_message(server_url: &str, log_path: Option<&std::path::Path>) -> String {
    match log_path {
        Some(path) => format!("Local server ready at {server_url} (full log: {})", path.display()),
        None => format!("Local server ready at {server_url} (full log disabled)"),
    }
}

fn display_project_relative_path(project_root: &std::path::Path, path: &std::path::Path) -> String {
    path.strip_prefix(project_root).unwrap_or(path).display().to_string()
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
                    kalam_dir: "kalam".into(),
                },
                connection: HashMap::from([(
                    "dev".into(),
                    ConnectionEnv {
                        url: "http://localhost:2900".into(),
                        namespace: "demo".into(),
                    },
                )]),
                schema: SchemaSection {
                    mode: SchemaMode::Remote,
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
        let runtime = tokio::runtime::Runtime::new().unwrap();

        let paused = runtime.block_on(run_initial_schema_pipeline(&ctx, &output, false, None));
        assert_eq!(paused, SchemaPipelineState::Paused);

        // Remove failing migration and retry with force.
        std::fs::remove_file(root.join("kalam/migrations/20250101120000_fail.sql")).unwrap();
        let recovered = runtime.block_on(run_initial_schema_pipeline(&ctx, &output, true, None));
        assert_eq!(recovered, SchemaPipelineState::Synced);
    }

    #[test]
    fn project_ready_message_includes_name_and_directory() {
        let message = project_ready_message("demo", std::path::Path::new("/tmp/demo"));
        assert_eq!(message, "Project demo at /tmp/demo");
    }

    #[test]
    fn local_server_ready_message_includes_log_path() {
        let message = local_server_ready_message(
            "http://localhost:2900",
            Some(std::path::Path::new("/tmp/demo/.kalam/logs/kalam.log")),
        );
        assert_eq!(
            message,
            "Local server ready at http://localhost:2900 (full log: /tmp/demo/.kalam/logs/kalam.log)"
        );
    }

    #[test]
    fn display_project_relative_path_prefers_project_relative_output() {
        let root = std::path::Path::new("/tmp/demo");
        let path = root.join("schema.sql");

        assert_eq!(display_project_relative_path(root, &path), "schema.sql");
    }
}
