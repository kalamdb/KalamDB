//! Child process supervision for `kalam dev`.

use std::{collections::HashMap, path::Path};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Child,
    task::JoinHandle,
};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    process::{
        kill_supervised_child, spawn_program_piped, spawn_shell_piped, SupervisedKillScope,
    },
    workflow::{
        dev::logs::{ServiceLogRegistry, ServiceLogSource},
        project::guidance::{dev_empty_process_command, dev_process_spawn_failed},
    },
};

pub struct ManagedProcess {
    pub pid: u32,
    pub kill_scope: SupervisedKillScope,
    pub child: Child,
    pub stdout_task: JoinHandle<()>,
    pub stderr_task: JoinHandle<()>,
}

pub struct ProcessSupervisor {
    processes: HashMap<String, ManagedProcess>,
}

impl ProcessSupervisor {
    pub fn new() -> Self {
        Self {
            processes: HashMap::new(),
        }
    }

    pub async fn spawn_all(
        &mut self,
        commands: &HashMap<String, String>,
        registry: &ServiceLogRegistry,
        output: &WorkflowOutput,
    ) -> Result<()> {
        for (name, command) in commands {
            if command.trim().is_empty() {
                return Err(CLIError::ConfigurationError(dev_empty_process_command(name)));
            }
            self.spawn_one(name, command, registry, output).await?;
        }
        Ok(())
    }

    pub async fn spawn_one(
        &mut self,
        name: &str,
        command: &str,
        registry: &ServiceLogRegistry,
        output: &WorkflowOutput,
    ) -> Result<()> {
        self.attach_managed_process(
            name,
            spawn_shell_piped(command),
            SupervisedKillScope::Tree,
            crate::process::shell_command_program(),
            command,
            registry,
            output,
        )
        .await
    }

    pub async fn spawn_program_one(
        &mut self,
        name: &str,
        program: &Path,
        args: &[impl AsRef<str>],
        working_dir: &Path,
        registry: &ServiceLogRegistry,
        output: &WorkflowOutput,
    ) -> Result<()> {
        let command_display = format!(
            "{} {}",
            program.display(),
            args.iter()
                .map(|arg| arg.as_ref())
                .collect::<Vec<_>>()
                .join(" ")
        );
        self.attach_managed_process(
            name,
            spawn_program_piped(program, args, Some(working_dir)),
            SupervisedKillScope::Process,
            &program.display().to_string(),
            &command_display,
            registry,
            output,
        )
        .await
    }

    pub fn manages_process(&self, name: &str) -> bool {
        self.processes.contains_key(name)
    }

    pub fn managed_pid(&self, name: &str) -> Option<u32> {
        self.processes.get(name).map(|managed| managed.pid)
    }

    pub fn count(&self) -> usize {
        self.processes.len()
    }

    pub async fn reap_finished(&mut self) -> Vec<(String, i32)> {
        let mut finished = Vec::new();
        for (name, managed) in self.processes.iter_mut() {
            if let Ok(Some(status)) = managed.child.try_wait() {
                let code = status.code().unwrap_or(-1);
                finished.push((name.clone(), code));
            }
        }

        for name in finished.iter().map(|(name, _)| name) {
            if let Some(managed) = self.processes.remove(name) {
                let _ = managed.stdout_task.await;
                let _ = managed.stderr_task.await;
            }
        }

        finished
    }

    pub async fn shutdown_process(&mut self, name: &str) {
        let Some(mut managed) = self.processes.remove(name) else {
            return;
        };
        kill_supervised_child(&mut managed.child, managed.kill_scope).await;
        let _ = managed.stdout_task.await;
        let _ = managed.stderr_task.await;
    }

    pub async fn shutdown(&mut self) {
        let names: Vec<String> = self.processes.keys().cloned().collect();
        for name in names {
            self.shutdown_process(&name).await;
        }
    }

    async fn attach_managed_process(
        &mut self,
        name: &str,
        child: std::io::Result<Child>,
        kill_scope: SupervisedKillScope,
        spawn_label: &str,
        command_display: &str,
        registry: &ServiceLogRegistry,
        output: &WorkflowOutput,
    ) -> Result<()> {
        if self.processes.contains_key(name) {
            return Err(CLIError::ConfigurationError(format!(
                "duplicate dev process name '{name}'"
            )));
        }

        let source = registry
            .get(name)
            .ok_or_else(|| {
                CLIError::ConfigurationError(format!("log source not registered for '{name}'"))
            })?
            .clone();

        let mut child = child.map_err(|error| {
            CLIError::ConfigurationError(dev_process_spawn_failed(
                name,
                spawn_label,
                command_display,
                &error.to_string(),
            ))
        })?;

        let pid = child.id().ok_or_else(|| {
            CLIError::ConfigurationError(format!(
                "failed to capture pid for dev process '{name}'"
            ))
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_task = spawn_log_reader(stdout, source.clone(), output.clone());
        let stderr_task = spawn_log_reader(stderr, source, output.clone());

        self.processes.insert(
            name.to_string(),
            ManagedProcess {
                pid,
                kill_scope,
                child,
                stdout_task,
                stderr_task,
            },
        );

        Ok(())
    }
}

impl Default for ProcessSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

fn spawn_log_reader(
    stream: Option<impl tokio::io::AsyncRead + Unpin + Send + 'static>,
    source: ServiceLogSource,
    output: WorkflowOutput,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(stream) = stream else {
            return;
        };
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            output.process_log(&source, &line);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_pid_is_none_for_unknown_process() {
        let supervisor = ProcessSupervisor::new();
        assert!(supervisor.managed_pid("server").is_none());
    }
}
