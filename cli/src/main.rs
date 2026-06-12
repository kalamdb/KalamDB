//! Kalam CLI - Terminal client for KalamDB
//!
//! **Implements T083**: CLI entry point with argument parsing using clap 4.4
//!
//! # Usage
//!
//! ```bash
//! # Interactive mode
//! kalam-cli -u http://localhost:3000 --token <JWT>
//!
//! # Execute SQL file
//! kalam-cli -u http://localhost:3000 --file queries.sql
//!
//! # JSON output
//! kalam-cli -u http://localhost:3000 --json -c "SELECT * FROM users"
//! ```

use std::io::IsTerminal;

use clap::Parser;
use kalam_cli::{terminal_ui, CLIConfiguration, CLIError, FileCredentialStore, Result};

mod args;
mod commands;
mod connect;
mod terminal_input;

use args::{Cli, CliCommand};
use commands::{handle_early_commands, handle_pre_session_commands, workflow, PreSessionResult};
use connect::create_session;
use terminal_input::prompt_password;

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        print_cli_error(&e);
        std::process::exit(1);
    }
}

fn print_cli_error(err: &CLIError) {
    let message = err.to_string();
    if message.contains('\n') {
        eprintln!("Error:\n{message}");
    } else {
        eprintln!("Error: {message}");
    }
}

async fn run() -> Result<()> {
    // Parse command-line arguments
    let mut cli = Cli::parse();

    // If the password is explicitly set to an empty string, only prompt in interactive mode.
    // In non-interactive modes (--command/--file), an empty password may be valid (e.g. default
    // root).
    let password_prompt_mode =
        matches!(
            cli.subcommand,
            Some(CliCommand::Login(_))
                | Some(CliCommand::Token(_))
                | Some(CliCommand::Invite(_))
                | Some(CliCommand::Whoami)
        ) || (cli.subcommand.is_none() && cli.command.is_none() && cli.file.is_none());
    if cli.password.as_deref() == Some("") && password_prompt_mode && std::io::stdin().is_terminal()
    {
        let password = prompt_password(&terminal_ui::prompt_label("Password:", !cli.no_color))
            .map_err(|e| CLIError::FileError(format!("Failed to read password: {}", e)))?;
        cli.password = Some(password);
    }

    // Initialize logging (basic)
    if cli.verbose {
        eprintln!("Verbose mode enabled");
    }

    // Commands such as version/update/doctor should not fail because local
    // credentials are missing or malformed.
    if handle_early_commands(&cli).await? {
        return Ok(());
    }

    // Load credential store
    let mut credential_store = FileCredentialStore::new()?;

    // Handle modes that do not use the regular command/file/interactive session path.
    match handle_pre_session_commands(&cli, &mut credential_store).await? {
        PreSessionResult::NotHandled => {},
        PreSessionResult::Exit => return Ok(()),
        PreSessionResult::ContinueToSession(login_continuation) => {
            cli.subcommand = None;
            cli.url = Some(login_continuation.server_url);
            cli.token = login_continuation.access_token;
            cli.user = None;
            cli.password = None;
        },
    }

    if workflow::handle_workflow_command(&cli).await? {
        return Ok(());
    }

    if cli.subcommand.is_some() {
        return Err(CLIError::ConfigurationError(
            "Unhandled command. Run `kalam --help` for available commands.".into(),
        ));
    }

    // Load configuration
    let config = CLIConfiguration::load(&cli.config)?;
    let config_path = kalam_cli::config::expand_config_path(&cli.config);

    let mut session = create_session(&cli, &mut credential_store, &config, config_path).await?;

    // Execute based on mode
    let command_text = cli.command_text();
    match (cli.file, command_text, cli.consume) {
        // Consume mode takes precedence
        (_, _, true) => {
            let topic = cli.topic.ok_or_else(|| {
                CLIError::ConfigurationError("--topic is required for consume mode".into())
            })?;
            session.print_execution_target_banner();
            session
                .cmd_consume(
                    &topic,
                    cli.group.as_deref(),
                    cli.from.as_deref(),
                    cli.consume_limit,
                    cli.consume_timeout,
                )
                .await?;
        },

        // Execute SQL file
        (Some(file), None, false) => {
            session.print_execution_target_banner();
            session.execute_file(&file).await?;
        },

        // Execute single command
        (None, Some(command), false) => {
            session.print_execution_target_banner();
            session.execute_input(&command).await?;
        },

        // Interactive mode
        (None, None, false) => {
            session.run_interactive().await?;
        },

        // Invalid combination
        (Some(_), Some(_), false) => {
            return Err(CLIError::ConfigurationError(
                "Cannot specify both --file and --command".into(),
            ));
        },
    }

    Ok(())
}
