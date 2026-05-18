use kalam_cli::{FileCredentialStore, Result};

use crate::args::Cli;

pub mod credentials;
pub mod subscriptions;
pub mod watch_schema;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreSessionCommand {
    CredentialManagement,
    CredentialLogin,
    WatchSchema,
    Subscriptions,
}

pub struct CommandContext<'a> {
    pub cli: &'a Cli,
    pub credential_store: &'a mut FileCredentialStore,
}

fn pre_session_command(cli: &Cli) -> Option<PreSessionCommand> {
    if cli.list_instances || cli.show_credentials || cli.delete_credentials {
        Some(PreSessionCommand::CredentialManagement)
    } else if cli.update_credentials {
        Some(PreSessionCommand::CredentialLogin)
    } else if cli.watch_schema {
        Some(PreSessionCommand::WatchSchema)
    } else if cli.list_subscriptions || cli.subscribe.is_some() {
        Some(PreSessionCommand::Subscriptions)
    } else {
        None
    }
}

async fn run_pre_session_command(
    command: PreSessionCommand,
    context: CommandContext<'_>,
) -> Result<bool> {
    match command {
        PreSessionCommand::CredentialManagement => {
            credentials::handle_credentials(context.cli, context.credential_store)
        },
        PreSessionCommand::CredentialLogin => {
            credentials::login_and_store_credentials(context.cli, context.credential_store).await
        },
        PreSessionCommand::WatchSchema => {
            watch_schema::handle_watch_schema(context.cli, context.credential_store).await
        },
        PreSessionCommand::Subscriptions => {
            subscriptions::handle_subscriptions(context.cli, context.credential_store).await
        },
    }
}

pub async fn handle_pre_session_commands(
    cli: &Cli,
    credential_store: &mut FileCredentialStore,
) -> Result<bool> {
    let Some(command) = pre_session_command(cli) else {
        return Ok(false);
    };

    run_pre_session_command(
        command,
        CommandContext {
            cli,
            credential_store,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    fn parse_cli(args: &[&str]) -> Cli {
        let argv = std::iter::once("kalam").chain(args.iter().copied());
        Cli::try_parse_from(argv).expect("test cli args should parse")
    }

    #[test]
    fn credential_management_handler_matches_local_credential_modes() {
        for flag in [
            "--list-instances",
            "--show-credentials",
            "--delete-credentials",
        ] {
            let cli = parse_cli(&[flag]);

            assert_eq!(pre_session_command(&cli), Some(PreSessionCommand::CredentialManagement));
        }
    }

    #[test]
    fn credential_login_handler_matches_update_mode() {
        let cli = parse_cli(&[
            "--update-credentials",
            "--user",
            "root",
            "--password",
            "secret",
        ]);

        assert_eq!(pre_session_command(&cli), Some(PreSessionCommand::CredentialLogin));
    }

    #[test]
    fn watch_schema_handler_matches_watch_mode() {
        let cli = parse_cli(&["--watch-schema", "--run", "echo changed"]);

        assert_eq!(pre_session_command(&cli), Some(PreSessionCommand::WatchSchema));
    }

    #[test]
    fn subscription_handler_matches_subscription_modes() {
        for args in [
            &["--list-subscriptions"][..],
            &["--subscribe", "SELECT 1"][..],
        ] {
            let cli = parse_cli(args);

            assert_eq!(pre_session_command(&cli), Some(PreSessionCommand::Subscriptions));
        }
    }

    #[test]
    fn no_pre_session_handler_matches_regular_sql_modes() {
        for args in [
            &["--command", "SELECT 1"][..],
            &["--file", "queries.sql"][..],
            &[][..],
        ] {
            let cli = parse_cli(args);

            assert_eq!(pre_session_command(&cli), None);
        }
    }
}
