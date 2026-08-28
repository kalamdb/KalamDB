//! High-level helpers for running shell scripts and child programs.

use std::{
    collections::HashMap,
    env,
    path::Path,
    process::{Command, Output, Stdio},
};

use tokio::process::Child;

use super::{
    lifecycle::configure_supervised_child,
    path::{resolve_program_on_path, shell_working_directory},
    shell::{shell_command, tokio_shell_command},
};

/// Run a shell script and capture stdout/stderr.
pub fn run_shell_script(working_dir: Option<&Path>, script: &str) -> std::io::Result<Output> {
    let mut command = shell_command(script);
    if let Some(dir) = working_dir {
        command.current_dir(shell_working_directory(dir));
    }
    command.output()
}

/// Run a shell script attached to the current terminal streams.
pub fn run_shell_script_inherited(script: &str) -> std::io::Result<std::process::ExitStatus> {
    shell_command(script)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
}

/// Run a program directly and capture stdout/stderr.
pub fn run_program(
    program: &Path,
    args: &[impl AsRef<str>],
    working_dir: Option<&Path>,
) -> std::io::Result<Output> {
    let mut command = Command::new(program);
    command.args(args.iter().map(AsRef::as_ref));
    if let Some(dir) = working_dir {
        command.current_dir(shell_working_directory(dir));
    }
    command.output()
}

/// Run a program after configuring args, env, and working directory.
pub fn run_program_configured<F>(program: &Path, configure: F) -> std::io::Result<Output>
where
    F: FnOnce(&mut Command),
{
    let mut command = Command::new(program);
    configure(&mut command);
    command.output()
}

/// Run a PATH-resolved tool such as npm or node.
///
/// On Windows, npm/pnpm/yarn shims are executed through `cmd /C` so `.cmd`
/// launchers and extensionless shims work reliably.
pub fn run_path_tool(name: &str, args: &[&str], working_dir: &Path) -> std::io::Result<Output> {
    if cfg!(windows) {
        let script = if args.is_empty() {
            name.to_string()
        } else {
            format!("{} {}", name, args.join(" "))
        };
        return run_shell_script(Some(working_dir), &script);
    }

    let program = resolve_program_on_path(name).unwrap_or_else(|| name.into());
    run_program(&program, args, Some(working_dir))
}

/// Spawn a shell script as an async child with piped stdout/stderr.
///
/// `extra_env` is applied only for keys that are not already set in this process.
pub fn spawn_shell_piped(
    script: &str,
    working_dir: Option<&Path>,
    extra_env: &HashMap<String, String>,
) -> std::io::Result<Child> {
    let mut command = tokio_shell_command(script);
    command.stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    if let Some(dir) = working_dir {
        command.current_dir(shell_working_directory(dir));
    }
    apply_unset_env(&mut command, extra_env);
    configure_supervised_child(&mut command);
    command.spawn()
}

fn apply_unset_env(command: &mut tokio::process::Command, extra_env: &HashMap<String, String>) {
    for (key, value) in extra_env {
        if env::var_os(key).is_none() {
            command.env(key, value);
        }
    }
}

/// Spawn a program directly as an async child with piped stdout/stderr.
pub fn spawn_program_piped(
    program: &Path,
    args: &[impl AsRef<str>],
    working_dir: Option<&Path>,
) -> std::io::Result<Child> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args.iter().map(AsRef::as_ref))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = working_dir {
        command.current_dir(shell_working_directory(dir));
    }
    configure_supervised_child(&mut command);
    command.spawn()
}

/// Spawn a detached program without waiting for completion.
pub fn spawn_detached(program: &str, args: &[impl AsRef<str>]) -> std::io::Result<()> {
    Command::new(program).args(args.iter().map(AsRef::as_ref)).spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;

    fn echo_env_script(var: &str) -> String {
        if cfg!(windows) {
            format!("echo %{var}%")
        } else {
            format!("printf %s \"${var}\"")
        }
    }

    #[tokio::test]
    async fn spawn_shell_piped_injects_extra_env_when_unset() {
        let key = "KALAM_TEST_DOTENV_PASSWORD";
        env::remove_var(key);
        let mut extra = HashMap::new();
        extra.insert(key.to_string(), "from-dotenv".to_string());

        let mut child = spawn_shell_piped(&echo_env_script(key), None, &extra).unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).await.unwrap();
        let status = child.wait().await.unwrap();
        assert!(status.success(), "echo env command failed: {status:?}");
        assert!(
            String::from_utf8_lossy(&buf).contains("from-dotenv"),
            "stdout: {}",
            String::from_utf8_lossy(&buf)
        );
    }

    #[tokio::test]
    async fn spawn_shell_piped_does_not_override_process_env() {
        let key = "KALAM_TEST_DOTENV_PRESERVE";
        env::set_var(key, "from-process");
        let mut extra = HashMap::new();
        extra.insert(key.to_string(), "from-dotenv".to_string());

        let mut child = spawn_shell_piped(&echo_env_script(key), None, &extra).unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).await.unwrap();
        let _ = child.wait().await;
        env::remove_var(key);

        let output = String::from_utf8_lossy(&buf);
        assert!(output.contains("from-process"), "stdout: {output}");
        assert!(!output.contains("from-dotenv"), "stdout: {output}");
    }
}
