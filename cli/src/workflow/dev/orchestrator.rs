//! Main `kalam dev` session orchestration loop.

use std::{fs, time::SystemTime};

use tokio::time::{self, Duration};

use crate::{
    error::{CLIError, Result},
    output::{WorkflowDisplayMode, WorkflowOutput},
    terminal_ui::ProgressTaskStatus,
    workflow::{
        dev::{
            draft_prompt::{
                draft_migration_exists, draft_migration_path, prompt_for_draft_application,
                DraftPromptDecision,
            },
            logs::ServiceLogRegistry,
            precheck::{ensure_local_dev_authentication_ready, run_dev_prechecks},
            processes::ProcessSupervisor,
            server::{prepare_local_server_launch, wait_for_server_ready},
            watch::{
                run_schema_pipeline, schema_file_changed, schema_file_mtime, schema_watch_path,
                update_schema_baseline, wait_for_stable_schema_file, SCHEMA_WATCH_INTERVAL_SECS,
            },
        },
        display_project_path,
        migration::{
            apply::{apply_pending_migrations, ApplyMigrationOptions},
            create::update_draft_migration,
        },
        project::{
            config::SchemaMode,
            guidance::dev_local_kalamdb_server_start_failed,
        },
        schema::{generate_schema_artifacts, GenerateOptions},
        sql::{build_workflow_client, drop_namespace_if_exists},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DevLoopAction {
    Continue,
    Stop,
}

#[derive(Debug)]
struct DevSchemaLoop {
    force: bool,
    schema_mtime: Option<SystemTime>,
}

impl DevSchemaLoop {
    fn new(force: bool) -> Self {
        Self {
            force,
            schema_mtime: None,
        }
    }

    async fn bootstrap(
        &mut self,
        ctx: &WorkflowContext,
        output: &WorkflowOutput,
    ) -> Result<DevLoopAction> {
        if !ctx.config.dev.apply_schema && !ctx.config.dev.generate_types {
            return Ok(DevLoopAction::Continue);
        }

        let _ = run_initial_schema_pipeline(ctx, output, self.force, None).await;
        self.refresh_schema_mtime(ctx);
        self.prompt_pending_draft(ctx, output).await
    }

    async fn handle_watch_tick(
        &mut self,
        ctx: &WorkflowContext,
        output: &WorkflowOutput,
    ) -> Result<DevLoopAction> {
        let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) else {
            return Ok(DevLoopAction::Continue);
        };
        if !schema_file_changed(&path, self.schema_mtime) {
            return Ok(DevLoopAction::Continue);
        }

        let stable_mtime = wait_for_stable_schema_file(&path).await;
        if stable_mtime == self.schema_mtime {
            return Ok(DevLoopAction::Continue);
        }

        let watched_path = display_project_path(&ctx.project_root, &path);
        let message = format!("Schema changed in {watched_path}; applying...");
        output.status(&message);
        let _ = run_initial_schema_pipeline(ctx, output, self.force, Some(message)).await;
        self.schema_mtime = schema_file_mtime(&path).or(stable_mtime);
        self.prompt_pending_draft(ctx, output).await
    }

    async fn prompt_pending_draft(
        &mut self,
        ctx: &WorkflowContext,
        output: &WorkflowOutput,
    ) -> Result<DevLoopAction> {
        if self.force {
            return Ok(DevLoopAction::Continue);
        }

        loop {
            let draft_path = draft_migration_path(ctx);
            if !draft_migration_exists(&draft_path) {
                return Ok(DevLoopAction::Continue);
            }

            match prompt_for_draft_application(output, &ctx.project_root, &draft_path)? {
                DraftPromptDecision::Apply => self.apply_confirmed_draft(ctx, output).await?,
                DraftPromptDecision::Reset => self.reset_and_apply_schema(ctx, output).await?,
                DraftPromptDecision::Cancel => {
                    output.status("schema draft apply cancelled; stopping kalam dev");
                    return Ok(DevLoopAction::Stop);
                },
            }
        }
    }

    async fn apply_confirmed_draft(
        &mut self,
        ctx: &WorkflowContext,
        output: &WorkflowOutput,
    ) -> Result<()> {
        let schema_path = schema_watch_path(&ctx.project_root, &ctx.config);
        let before_mtime = schema_path.as_ref().and_then(|p| schema_file_mtime(p));
        let before_schema = schema_path.as_ref().and_then(|p| fs::read_to_string(p).ok());
        let pipeline_state = run_confirmed_schema_draft_pipeline(ctx, output).await;
        self.schema_mtime = schema_path.as_ref().and_then(|p| schema_file_mtime(p));

        let after_schema = schema_path.as_ref().and_then(|p| fs::read_to_string(p).ok());
        let schema_changed_during_apply = match (before_schema.as_ref(), after_schema.as_ref()) {
            (Some(before), Some(after)) => before != after,
            _ => before_mtime != self.schema_mtime,
        };

        if pipeline_state == SchemaPipelineState::Synced && schema_changed_during_apply {
            if let Some(schema) = before_schema {
                restore_schema_baseline(ctx, &schema)?;
            }
            output.status("schema.sql changed while migrations were applying; refreshing draft");
            let _ = run_initial_schema_pipeline(
                ctx,
                output,
                self.force,
                Some("Refreshing schema draft after concurrent edit...".to_string()),
            )
            .await;
            self.refresh_schema_mtime(ctx);
        }

        output.restore_dev_session_view()?;
        Ok(())
    }

    fn refresh_schema_mtime(&mut self, ctx: &WorkflowContext) {
        if let Some(path) = schema_watch_path(&ctx.project_root, &ctx.config) {
            self.schema_mtime = schema_file_mtime(&path);
        }
    }

    async fn reset_and_apply_schema(
        &mut self,
        ctx: &WorkflowContext,
        output: &WorkflowOutput,
    ) -> Result<()> {
        output.status("resetting schema state and applying schema.sql from scratch");
        reset_remote_namespace(ctx, output).await?;
        reset_local_schema_state(ctx, output)?;

        let _ = run_initial_schema_pipeline(
            ctx,
            output,
            self.force,
            Some("Rebuilding schema draft from schema.sql...".to_string()),
        )
        .await;
        self.apply_confirmed_draft(ctx, output).await?;
        self.refresh_schema_mtime(ctx);
        Ok(())
    }
}

pub async fn run_dev_session(ctx: &WorkflowContext, options: DevSessionOptions) -> Result<()> {
    let output = ctx.output().with_display_mode(options.display_mode);
    let mut supervisor = ProcessSupervisor::new();
    let mut local_server_managed = false;

    let result =
        run_dev_session_inner(ctx, options, &output, &mut supervisor, &mut local_server_managed).await;

    if local_server_managed {
        if let Some(pid) = supervisor.managed_pid("server") {
            output.status(format!("stopping local KalamDB server (pid {pid})"));
        } else {
            output.status("stopping local KalamDB server");
        }
        supervisor.shutdown_process("server").await;
    }
    supervisor.shutdown().await;
    result
}

async fn run_dev_session_inner(
    ctx: &WorkflowContext,
    options: DevSessionOptions,
    output: &WorkflowOutput,
    supervisor: &mut ProcessSupervisor,
    local_server_managed: &mut bool,
) -> Result<()> {
    let mut registry = ServiceLogRegistry::new();
    registry.register("server");

    for name in ctx.config.dev.processes.keys() {
        registry.register(name);
    }

    let server_source = registry.get("server").expect("server source registered").clone();

    output.status("starting kalam dev session");
    output.progress_task(
        "project",
        ProgressTaskStatus::Succeeded,
        project_ready_message(&ctx.config.project.name, &ctx.project_root),
    );
    let precheck = run_dev_prechecks(ctx, output, &server_source).await?;

    if ctx.config.dev.auto_start_db {
        if precheck.local_server_reused {
            output.status(format!("using existing KalamDB server at {}", precheck.environment.url));
            output.progress_task(
                "server",
                ProgressTaskStatus::Succeeded,
                local_server_ready_message(&precheck.environment.url, output.workflow_log_path()),
            );
        } else {
            let launch = prepare_local_server_launch(
                &ctx.project_root,
                &ctx.config,
                &precheck.environment.url,
            )?;
            output.status("starting local KalamDB server");
            output.progress_task(
                "server",
                ProgressTaskStatus::Running,
                "Starting local KalamDB server...",
            );
            let config_arg = launch.config_path.display().to_string();
            if let Err(error) = supervisor
                .spawn_program_one(
                    "server",
                    &launch.program,
                    &[config_arg],
                    &launch.working_dir,
                    &registry,
                    output,
                )
                .await
            {
                let message = dev_local_kalamdb_server_start_failed(
                    &launch.program,
                    &error.to_string(),
                );
                output.status(&message);
                return Err(CLIError::ConfigurationError(message));
            }
            *local_server_managed = true;
            wait_for_server_ready(
                &precheck.environment.url,
                &launch.program,
                output,
                supervisor,
            )
                .await
                .map_err(|error| {
                    output.status(error.to_string());
                    error
                })?;
            ensure_local_dev_authentication_ready(
                ctx,
                &precheck.environment,
                output,
                &server_source,
            )
            .await?;
            output.progress_task(
                "server",
                ProgressTaskStatus::Succeeded,
                local_server_ready_message(&precheck.environment.url, output.workflow_log_path()),
            );
        }
    }

    let mut schema_loop = DevSchemaLoop::new(options.force);
    if schema_loop.bootstrap(ctx, output).await? == DevLoopAction::Stop {
        return Ok(());
    }

    if !ctx.config.dev.processes.is_empty() {
        supervisor.spawn_all(&ctx.config.dev.processes, &registry, output).await?;
    }

    let watch_enabled = precheck.watch_enabled;
    if watch_enabled {
        output.status("watching schema source for changes");
    }
    if ctx.config.dev.processes.is_empty() {
        output.status("no custom dev processes configured");
    }
    let mut watch_interval = time::interval(Duration::from_secs(SCHEMA_WATCH_INTERVAL_SECS));
    watch_interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        tokio::select! {
            result = &mut ctrl_c => {
                result.map_err(|e| crate::error::CLIError::ConfigurationError(
                    format!("ctrl-c handler failed: {e}")
                ))?;
                output.status("shutting down (Ctrl+C)");
                break;
            }
            _ = watch_interval.tick(), if watch_enabled => {
                if schema_loop.handle_watch_tick(ctx, output).await? == DevLoopAction::Stop {
                    output.status("shutting down (schema prompt cancelled)");
                    break;
                }
            }
            _ = time::sleep(Duration::from_millis(200)) => {
                let finished = supervisor.reap_finished().await;
                for (name, code) in &finished {
                    output.status(format!("process {name} exited with code {code}"));
                }
                let managed_count = supervisor.count();
                if managed_count == 0
                    && (*local_server_managed || !ctx.config.dev.processes.is_empty())
                {
                    output.status("all dev processes exited");
                    break;
                }
                if managed_count == 0 && !watch_enabled {
                    output.status("dev session completed");
                    break;
                }
            }
        }
    }

    Ok(())
}

// ── Schema pipeline ───────────────────────────────────────────────────────────

async fn run_initial_schema_pipeline(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
    force: bool,
    running_message: Option<String>,
) -> SchemaPipelineState {
    let running_message = running_message.unwrap_or_else(|| "Applying schema...".to_string());
    // Clear stale details (e.g. a lingering "Apply? [y/N]:" from a previous
    // prompt cycle) before the new pipeline run adds its own lines.
    output.progress_task("schema", ProgressTaskStatus::Running, &running_message);
    output.clear_progress_details("schema");

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

async fn run_confirmed_schema_draft_pipeline(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
) -> SchemaPipelineState {
    output.progress_task(
        "schema",
        ProgressTaskStatus::Running,
        "Applying confirmed schema draft...",
    );
    output.clear_progress_details("schema");
    match apply_confirmed_schema_draft(ctx, output).await {
        Ok(()) => {
            if output.display_mode == WorkflowDisplayMode::Progress {
                output.progress_task("schema", ProgressTaskStatus::Succeeded, "Schema applied");
            } else {
                output.status("schema pipeline completed");
            }
            SchemaPipelineState::Synced
        },
        Err(error) => {
            emit_schema_failure(output, &error);
            SchemaPipelineState::Paused
        },
    }
}

async fn apply_confirmed_schema_draft(
    ctx: &WorkflowContext,
    output: &WorkflowOutput,
) -> Result<()> {
    let config = &ctx.config;
    let project_root = &ctx.project_root;

    if config.dev.apply_schema {
        if matches!(config.schema.mode, SchemaMode::Sql) {
            let _ = update_draft_migration(project_root, config, output)?;
        }
        apply_pending_migrations(ctx, output, &ApplyMigrationOptions::dev_confirmed_draft())
            .await?;
        update_schema_baseline(project_root, config, output)?;
    }

    if config.dev.generate_types {
        generate_schema_artifacts(ctx, &GenerateOptions { languages: None }, output)?;
    }

    if !config.dev.apply_schema {
        update_schema_baseline(project_root, config, output)?;
    }

    Ok(())
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn restore_schema_baseline(ctx: &WorkflowContext, schema: &str) -> Result<()> {
    let baseline = ctx.config.schema_baseline_path(&ctx.project_root);
    if let Some(parent) = baseline.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&baseline, schema).map_err(|e| {
        crate::error::CLIError::FileError(format!(
            "failed to restore schema baseline '{}': {e}",
            baseline.display()
        ))
    })
}

async fn reset_remote_namespace(ctx: &WorkflowContext, output: &WorkflowOutput) -> Result<()> {
    let environment = ctx.resolved_environment()?;
    let client = build_workflow_client(ctx, &environment)?;
    output.status(format!("dropping namespace {} before reset", environment.namespace));
    drop_namespace_if_exists(&client, &environment.namespace).await?;
    output.status(format!("reset namespace {}", environment.namespace));
    Ok(())
}

fn reset_local_schema_state(ctx: &WorkflowContext, output: &WorkflowOutput) -> Result<()> {
    let migrations_dir = ctx.config.migrations_dir(&ctx.project_root);
    if migrations_dir.is_dir() {
        for entry in fs::read_dir(&migrations_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "sql") {
                fs::remove_file(&path).map_err(|error| {
                    crate::error::CLIError::FileError(format!(
                        "failed to remove migration '{}': {error}",
                        display_project_path(&ctx.project_root, &path)
                    ))
                })?;
                output.status(format!(
                    "removed migration {}",
                    display_project_path(&ctx.project_root, &path)
                ));
            }
        }
    }

    let baseline = ctx.config.schema_baseline_path(&ctx.project_root);
    if baseline.is_file() {
        fs::remove_file(&baseline).map_err(|error| {
            crate::error::CLIError::FileError(format!(
                "failed to remove schema baseline '{}': {error}",
                display_project_path(&ctx.project_root, &baseline)
            ))
        })?;
        output.status(format!(
            "removed schema baseline {}",
            display_project_path(&ctx.project_root, &baseline)
        ));
    }

    Ok(())
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
        Some(path) => {
            format!("Local server ready at {server_url} (full log: {})", path.display())
        },
        None => format!("Local server ready at {server_url} (full log disabled)"),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
    fn display_project_path_prefers_project_relative_output() {
        let root = std::path::Path::new("/tmp/demo");
        let path = root.join("schema.sql");
        assert_eq!(display_project_path(root, &path), "schema.sql");
    }
}
