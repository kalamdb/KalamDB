use kalam_cli::Result;

use crate::args::{Cli, CliCommand};

mod context;
mod db;
mod deploy;
mod dev;
mod functions;
mod init;
mod lifecycle;
mod link;
mod schema;

pub async fn handle_workflow_command(cli: &Cli) -> Result<bool> {
    let Some(subcommand) = &cli.subcommand else {
        return Ok(false);
    };

    match subcommand {
        CliCommand::Init(args) => {
            init::handle_init(cli, args).await?;
            Ok(true)
        },
        CliCommand::Link(args) => {
            link::handle_link(cli, args)?;
            Ok(true)
        },
        CliCommand::Schema(args) => {
            schema::handle_schema(cli, args)?;
            Ok(true)
        },
        CliCommand::Db(args) => {
            db::handle_db(cli, args).await?;
            Ok(true)
        },
        CliCommand::Dev(args) => {
            dev::handle_dev(cli, args).await?;
            Ok(true)
        },
        CliCommand::Instances(args) => {
            lifecycle::handle_instances(cli, args).await?;
            Ok(true)
        },
        CliCommand::Up(args) => {
            lifecycle::handle_up(cli, args).await?;
            Ok(true)
        },
        CliCommand::Down(args) => {
            lifecycle::handle_down(cli, args).await?;
            Ok(true)
        },
        CliCommand::Status(args) => {
            lifecycle::handle_status(cli, args).await?;
            Ok(true)
        },
        CliCommand::Logs(args) => {
            lifecycle::handle_logs(cli, args).await?;
            Ok(true)
        },
        CliCommand::Deploy(args) => {
            deploy::handle_deploy(cli, args).await?;
            Ok(true)
        },
        CliCommand::Functions(args) => {
            functions::handle_functions(cli, args).await?;
            Ok(true)
        },
        _ => Ok(false),
    }
}
