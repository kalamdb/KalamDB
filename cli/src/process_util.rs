//! Cross-platform process and shell command helpers.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

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

/// Build `cd <dir> && <program> <args...>` for the platform shell.
pub fn build_cd_and_run_command(
    working_dir: &Path,
    program: &Path,
    args: &[impl AsRef<str>],
) -> String {
    let work = quote_for_shell(&working_dir.display().to_string());
    let bin = quote_for_shell(&program.display().to_string());
    let trailing = args
        .iter()
        .map(|arg| quote_for_shell(arg.as_ref()))
        .collect::<Vec<_>>()
        .join(" ");
    let invocation = if trailing.is_empty() {
        bin
    } else {
        format!("{bin} {trailing}")
    };

    if cfg!(windows) {
        format!("cd /d {work} && {invocation}")
    } else {
        format!("cd {work} && exec {invocation}")
    }
}

/// Resolve an executable name on `PATH`, including Windows `PATHEXT` suffixes.
pub fn resolve_program_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for dir in env::split_paths(&path_var) {
        for candidate in program_candidate_names(name) {
            let path = dir.join(&candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn program_candidate_names(name: &str) -> Vec<String> {
    let mut names = vec![name.to_string()];
    if cfg!(windows)
        && !name.ends_with(".exe")
        && !name.ends_with(".cmd")
        && !name.ends_with(".bat")
    {
        names.push(format!("{name}.exe"));
        names.push(format!("{name}.cmd"));
        names.push(format!("{name}.bat"));
    }
    names
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_cd_and_run_command_quotes_unix_paths_with_spaces() {
        if cfg!(windows) {
            return;
        }
        let command = build_cd_and_run_command(
            Path::new("/tmp/my project"),
            Path::new("/opt/kalamdb-server"),
            &["/tmp/my project/server.toml"],
        );
        assert_eq!(
            command,
            "cd '/tmp/my project' && exec /opt/kalamdb-server '/tmp/my project/server.toml'"
        );
    }

    #[test]
    fn build_cd_and_run_command_uses_windows_cd_flag() {
        if !cfg!(windows) {
            return;
        }
        let command = build_cd_and_run_command(
            Path::new(r"C:\projects\demo"),
            Path::new(r"C:\Users\Jamal\.kalam\bin\kalamdb-server.exe"),
            &[r"C:\projects\demo\kalam\server\server.toml"],
        );
        assert!(command.starts_with("cd /d "));
        assert!(command.contains("kalamdb-server.exe"));
        assert!(command.contains("server.toml"));
    }

    #[test]
    fn resolve_program_on_path_finds_cmd_shims_on_windows() {
        if !cfg!(windows) {
            return;
        }
        let path = resolve_program_on_path("cmd");
        assert!(path.is_some(), "cmd should resolve on Windows");
    }
}
