//! Platform shell helpers for running script strings.

use std::process::Command;

use tokio::process::Command as TokioCommand;

/// Shell used to run script strings (`cmd /C` on Windows, `sh -c` elsewhere).
pub fn shell_command_program() -> &'static str {
    if cfg!(windows) {
        "cmd"
    } else {
        "sh"
    }
}

/// Configure `command` to run `script` through the platform shell.
pub fn configure_shell_command(command: &mut Command, script: &str) {
    if cfg!(windows) {
        command.arg("/C").arg(script);
    } else {
        command.arg("-c").arg(script);
    }
}

/// Configure `command` to run `script` through the platform shell.
pub fn configure_tokio_shell_command(command: &mut TokioCommand, script: &str) {
    if cfg!(windows) {
        command.arg("/C").arg(script);
    } else {
        command.arg("-c").arg(script);
    }
}

/// Build a new process command that runs `script` through the platform shell.
pub fn shell_command(script: &str) -> Command {
    let mut command = Command::new(shell_command_program());
    configure_shell_command(&mut command, script);
    command
}

/// Build a new async process command that runs `script` through the platform shell.
pub fn tokio_shell_command(script: &str) -> TokioCommand {
    let mut command = TokioCommand::new(shell_command_program());
    configure_tokio_shell_command(&mut command, script);
    command
}

/// Quote `value` for use inside a platform shell command string.
pub fn quote_for_shell(value: &str) -> String {
    if cfg!(windows) {
        windows_cmd_quote(value)
    } else {
        unix_shell_quote(value)
    }
}

fn windows_cmd_quote(value: &str) -> String {
    if value.chars().all(|c| c.is_ascii_alphanumeric() || r"\/._-:".contains(c)) {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

fn unix_shell_quote(value: &str) -> String {
    if value.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}
