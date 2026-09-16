use kalam_cli::{workflow::schema, Result};

use super::context::workflow_context;
use crate::args::{Cli, SchemaArgs, SchemaCommand};

pub(super) fn handle_schema(cli: &Cli, args: &SchemaArgs) -> Result<()> {
    match &args.command {
        SchemaCommand::Gen(gen_args) => {
            let ctx = workflow_context(cli, args.project_dir.as_deref(), None)?;
            schema::generate_schema(&ctx, gen_args.languages.clone())
        },
    }
}
