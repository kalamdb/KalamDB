//! Child process supervision for `kalam dev`.

use std::collections::HashMap;

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    task::JoinHandle,
};

use crate::{
    error::{CLIError, Result},
    output::WorkflowOutput,
    workflow::dev::logs::{ServiceLogRegistry, ServiceLogSource},
};

pub struct ManagedProcess {
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
                return Err(CLIError::ConfigurationError(format!(
                    "dev.processes.{name} command must not be empty"
                )));
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

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                CLIError::ConfigurationError(format!("failed to start dev process '{name}': {e}"))
            })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_task = spawn_log_reader(stdout, source.clone(), output.clone());
        let stderr_task = spawn_log_reader(stderr, source, output.clone());

        self.processes.insert(
            name.to_string(),
            ManagedProcess {
                child,
                stdout_task,
                stderr_task,
            },
        );

        Ok(())
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

    pub async fn shutdown(&mut self) {
        for (_, mut managed) in self.processes.drain() {
            let _ = managed.child.start_kill();
            let _ = managed.child.wait().await;
            let _ = managed.stdout_task.await;
            let _ = managed.stderr_task.await;
        }
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
            output.service_log(&source, &line);
        }
    })
}
