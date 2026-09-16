pub(crate) mod display;
pub mod draft_prompt;
pub mod exec;
pub mod logs;
pub mod orchestrator;
pub mod precheck;
pub mod processes;
pub mod server;
pub mod session;
pub mod watch;

pub use logs::{ServiceColor, ServiceLogRegistry, ServiceLogSource};
pub use orchestrator::{run_dev_session, DevSessionOptions, SchemaPipelineState};
pub use watch::{
    functions_watch_stamp, run_schema_pipeline, schema_file_changed, schema_file_mtime,
    schema_watch_path, update_schema_baseline, SCHEMA_WATCH_INTERVAL_SECS,
};

use crate::{error::Result, output::WorkflowDisplayMode, workflow::WorkflowContext};

pub async fn start_dev(ctx: &WorkflowContext, force: bool) -> Result<()> {
    session::start_background_session(ctx, force).await
}

pub async fn show_session_status(ctx: &WorkflowContext) -> Result<()> {
    session::show_background_status(ctx).await
}

pub async fn print_session_logs(ctx: &WorkflowContext, follow: bool, lines: usize) -> Result<()> {
    session::print_background_logs(ctx, follow, lines).await
}

pub async fn stop_dev(ctx: &WorkflowContext) -> Result<()> {
    session::stop_background_session(ctx).await
}

pub async fn run_dev_exec(ctx: &WorkflowContext, command: &str) -> Result<i32> {
    let output = ctx.output();
    let status = exec::run_isolated_exec(ctx, command, &output).await?;
    Ok(status.code().unwrap_or(1))
}

pub async fn run_dev(
    ctx: &WorkflowContext,
    force: bool,
    display_mode: WorkflowDisplayMode,
) -> Result<()> {
    let display_mode = if ctx.agent {
        WorkflowDisplayMode::Agent
    } else {
        display_mode
    };
    run_dev_session(
        ctx,
        DevSessionOptions {
            force,
            display_mode,
            agent: ctx.agent,
        },
    )
    .await
}
