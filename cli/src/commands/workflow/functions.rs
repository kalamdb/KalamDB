use kalam_cli::{workflow::functions, Result};

use super::context::workflow_context;
use crate::args::{Cli, FunctionsArgs, FunctionsCommand};

pub(super) async fn handle_functions(cli: &Cli, args: &FunctionsArgs) -> Result<()> {
    let ctx = workflow_context(cli, args.project_dir.as_deref(), None)?;
    match &args.command {
        FunctionsCommand::Build => functions::build_functions(&ctx).await,
        FunctionsCommand::Status => functions::show_function_status(&ctx).await,
        FunctionsCommand::Revisions => functions::show_function_revisions(&ctx).await,
        FunctionsCommand::Rollback(rollback) => {
            functions::rollback_function(&ctx, &rollback.revision).await
        },
        FunctionsCommand::Logs(logs) => {
            functions::show_function_logs(&ctx, logs.procedure.as_deref()).await
        },
        FunctionsCommand::Runtime => functions::show_function_runtime(&ctx).await,
        FunctionsCommand::Override(override_args) => {
            functions::override_function(&ctx, &override_args.procedure)
        },
    }
}
